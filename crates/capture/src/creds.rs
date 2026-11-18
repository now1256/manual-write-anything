//! Per-user credential file (like `~/.aws/credentials`): JSON, mode 0600, outside any workspace.
//! The OS keyring is not used because cron-driven runs have no desktop session (D-019).

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredState {
    Ok,
    /// The server rejected it. No automatic login until a person sets it again.
    Broken,
    /// First login: the password must be changed in the browser first.
    NeedsPasswordChange,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Credential {
    pub password: String,
    pub state: CredState,
    pub note: Option<String>,
    pub updated_at: u64,
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("password", &"<redacted>")
            .field("state", &self.state)
            .field("note", &self.note)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CredError {
    #[error("자격 파일 위치를 정할 수 없습니다 (HOME 없음)")]
    NoHome,
    #[error("자격 파일 {0}의 권한이 너무 넓습니다 ({1:o}). `chmod 600 {0}` 후 다시 실행하세요")]
    TooOpen(PathBuf, u32),
    #[error("자격 파일 {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("자격 파일 {0} 형식 오류: {1}")]
    Format(PathBuf, serde_json::Error),
}

pub struct CredStore {
    path: PathBuf,
    entries: BTreeMap<String, Credential>,
}

/// Credentials are keyed by account and server, not by workspace name.
pub fn key(user_id: &str, base_url: &str) -> String {
    format!("{user_id}@{}", base_url.trim_end_matches('/'))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl CredStore {
    /// `$MWA_CREDENTIALS_FILE`, else `$XDG_CONFIG_HOME/mwa/credentials.json`, else `~/.config/mwa/credentials.json`.
    pub fn default_path() -> Result<PathBuf, CredError> {
        if let Some(p) = std::env::var_os("MWA_CREDENTIALS_FILE") {
            return Ok(PathBuf::from(p));
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .ok_or(CredError::NoHome)?;
        Ok(base.join("mwa/credentials.json"))
    }

    pub fn open(path: &Path) -> Result<Self, CredError> {
        let entries = match std::fs::metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(CredError::Io(path.to_path_buf(), e)),
            Ok(meta) => {
                let mode = meta.permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    return Err(CredError::TooOpen(path.to_path_buf(), mode));
                }
                let text = std::fs::read_to_string(path)
                    .map_err(|e| CredError::Io(path.to_path_buf(), e))?;
                serde_json::from_str(&text).map_err(|e| CredError::Format(path.to_path_buf(), e))?
            }
        };
        Ok(Self {
            path: path.to_path_buf(),
            entries,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, key: &str) -> Option<&Credential> {
        self.entries.get(key)
    }

    pub fn set_password(&mut self, key: &str, password: String) {
        self.entries.insert(
            key.to_string(),
            Credential {
                password,
                state: CredState::Ok,
                note: None,
                updated_at: now(),
            },
        );
    }

    pub fn mark(&mut self, key: &str, state: CredState, note: Option<String>) {
        if let Some(c) = self.entries.get_mut(key) {
            c.state = state;
            c.note = note;
            c.updated_at = now();
        }
    }

    pub fn remove(&mut self, key: &str) -> bool {
        self.entries.remove(key).is_some()
    }

    /// Atomic write: temp file created with 0600, then renamed over the old file.
    pub fn save(&self) -> Result<(), CredError> {
        let io = |e| CredError::Io(self.path.clone(), e);
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&self.entries).expect("credentials serialize");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(io)?;
        f.write_all(text.as_bytes()).map_err(io)?;
        f.sync_all().map_err(io)?;
        std::fs::rename(&tmp, &self.path).map_err(io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_private_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mwa/credentials.json");
        let mut s = CredStore::open(&path).unwrap();
        let k = key("u1", "http://h:3001/");
        assert_eq!(k, "u1@http://h:3001");
        s.set_password(&k, "secret".into());
        s.save().unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        let mut s = CredStore::open(&path).unwrap();
        assert_eq!(s.get(&k).unwrap().password, "secret");
        s.mark(&k, CredState::Broken, Some("HTTP 401".into()));
        s.save().unwrap();
        let s = CredStore::open(&path).unwrap();
        assert_eq!(s.get(&k).unwrap().state, CredState::Broken);
    }

    #[test]
    fn refuses_world_readable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            CredStore::open(&path),
            Err(CredError::TooOpen(_, 0o644))
        ));
    }

    #[test]
    fn debug_never_shows_password() {
        let c = Credential {
            password: "hunter2".into(),
            state: CredState::Ok,
            note: None,
            updated_at: 0,
        };
        assert!(!format!("{c:?}").contains("hunter2"));
    }
}
