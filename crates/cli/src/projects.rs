//! Known projects (M-App T1): a small registry in the user's data folder so the app can list,
//! create and switch project folders. A project is any folder with a `mwa.toml`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mwa_host::{Host, ProjectEntry, Projects};
use serde::{Deserialize, Serialize};

use crate::{CONFIG_FILE, STATE_DIR, TEMPLATE};

#[derive(Default, Serialize, Deserialize)]
struct RegistryFile {
    projects: Vec<Stored>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Stored {
    name: String,
    path: PathBuf,
    #[serde(default)]
    last_opened: Option<u64>,
}

/// `$MWA_HOME`, else the platform's per-user data folder plus `mwa`.
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("MWA_HOME") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        return home.join("Library/Application Support/mwa");
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("mwa")
}

pub struct Registry {
    file: PathBuf,
    /// Where new projects go when no folder is given.
    root: PathBuf,
    lock: Mutex<()>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `~/x` → `$HOME/x`; relative paths are not accepted (the server has no meaningful cwd).
fn expand(path: &str) -> Result<PathBuf, String> {
    let p = match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(rest))
            .ok_or("HOME을 알 수 없습니다")?,
        None => PathBuf::from(path),
    };
    if p.is_absolute() {
        Ok(p)
    } else {
        Err(format!("절대 경로로 적어 주세요: {path}"))
    }
}

/// Folder name for a project name: path separators and spaces become `-`.
fn folder_name(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_whitespace() => '-',
            c => c,
        })
        .collect();
    s.trim_matches(['-', '.']).to_string()
}

/// Display name for a folder opened without a name: the product name, else the folder name.
fn default_name(path: &Path) -> String {
    std::fs::read_to_string(path.join(CONFIG_FILE))
        .ok()
        .and_then(|t| mwa_protocol::Config::parse(&t).ok())
        .and_then(|(c, _)| c.product.and_then(|p| p.name))
        .unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        })
}

impl Registry {
    pub fn new(dir: &Path) -> Self {
        Self {
            file: dir.join("projects.json"),
            root: dir.join("projects"),
            lock: Mutex::new(()),
        }
    }

    fn load(&self) -> RegistryFile {
        std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save(&self, f: &RegistryFile) -> Result<(), String> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(f).expect("registry serializes");
        std::fs::write(&self.file, text).map_err(|e| format!("{}: {e}", self.file.display()))
    }

    /// Adds `path` (or updates it) as the most recently opened project.
    pub fn touch(&self, path: &Path, name: Option<&str>) -> Result<ProjectEntry, String> {
        let _guard = self.lock.lock().expect("registry lock");
        let mut f = self.load();
        let at = now();
        let entry = match f.projects.iter_mut().find(|p| p.path == path) {
            Some(p) => {
                p.last_opened = Some(at);
                if let Some(n) = name {
                    p.name = n.to_string();
                }
                p.clone()
            }
            None => {
                let s = Stored {
                    name: name.map(String::from).unwrap_or_else(|| default_name(path)),
                    path: path.to_path_buf(),
                    last_opened: Some(at),
                };
                f.projects.push(s.clone());
                s
            }
        };
        self.save(&f)?;
        Ok(to_entry(&entry))
    }
}

fn to_entry(s: &Stored) -> ProjectEntry {
    ProjectEntry {
        name: s.name.clone(),
        path: s.path.display().to_string(),
        ready: s.path.join(CONFIG_FILE).is_file(),
        last_opened: s.last_opened,
    }
}

impl Projects for Registry {
    fn list(&self) -> Vec<ProjectEntry> {
        let mut items: Vec<ProjectEntry> = self.load().projects.iter().map(to_entry).collect();
        items.sort_by_key(|a| std::cmp::Reverse(a.last_opened));
        items
    }

    fn create(&self, name: &str, path: Option<&str>) -> Result<ProjectEntry, String> {
        let folder = folder_name(name);
        if folder.is_empty() || name.chars().count() > 60 {
            return Err("프로젝트 이름은 1~60자로 적어 주세요".into());
        }
        let dir = match path {
            Some(p) => expand(p)?,
            None => self.root.join(&folder),
        };
        if dir.join(CONFIG_FILE).exists() {
            return Err(format!(
                "{}에 이미 프로젝트가 있습니다. \"기존 폴더 열기\"를 쓰세요",
                dir.display()
            ));
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::write(dir.join(CONFIG_FILE), TEMPLATE)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        let gitignore = dir.join(".gitignore");
        if !gitignore.exists() {
            let _ = std::fs::write(&gitignore, format!("{STATE_DIR}/\n"));
        }
        self.touch(&dir, Some(name.trim()))
    }

    fn open(&self, path: &str) -> Result<Arc<dyn Host>, String> {
        let dir = expand(path)?;
        if !dir.join(CONFIG_FILE).is_file() {
            return Err(format!(
                "{}에 {CONFIG_FILE}이 없습니다. 새 프로젝트로 만드세요",
                dir.display()
            ));
        }
        let host = crate::tui_host::CliHost::open(&dir).map_err(|e| format!("{e:#}"))?;
        self.touch(&dir, None)?;
        Ok(Arc::new(host))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_list_open_and_reject_duplicates() {
        let home = tempfile::tempdir().unwrap();
        let reg = Registry::new(home.path());
        let a = reg.create("Acme 콘솔", None).unwrap();
        assert!(a.ready && a.path.ends_with("projects/Acme-콘솔"), "{a:?}");
        assert!(
            reg.create("Acme 콘솔", None)
                .unwrap_err()
                .contains("이미 프로젝트")
        );
        assert!(reg.create("  ", None).is_err());
        assert!(
            reg.create("x", Some("relative/dir"))
                .unwrap_err()
                .contains("절대 경로")
        );

        let other = tempfile::tempdir().unwrap();
        let b = reg
            .create("두 번째", Some(other.path().to_str().unwrap()))
            .unwrap();
        assert_eq!(reg.list().len(), 2);

        assert!(reg.open(home.path().to_str().unwrap()).is_err());
        reg.open(&a.path).unwrap();
        let list = reg.list();
        assert_eq!(list[0].path, a.path, "most recently opened first");
        assert_eq!(list[1].path, b.path);
    }

    #[test]
    fn folder_names_are_safe() {
        assert_eq!(folder_name(" a/b c "), "a-b-c");
        assert_eq!(folder_name(".."), "");
    }
}
