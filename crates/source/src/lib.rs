//! Source connection (FS). Every input becomes a git checkout pinned to a commit SHA.
//! M0 implements local paths only; remote mirrors and uploads arrive with M7.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub name: String,
    /// Repository top level (may differ from the configured path if it pointed inside a repo).
    pub root: PathBuf,
    pub sha: String,
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    pub remote: Option<String>,
    /// Number of modified or untracked entries (`git status --porcelain`).
    pub dirty_entries: usize,
}

impl Snapshot {
    /// A checkout with uncommitted changes can be previewed but never approved.
    pub fn is_preview(&self) -> bool {
        self.dirty_entries > 0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("경로가 없습니다: {0}")]
    PathMissing(PathBuf),
    #[error("git 저장소가 아닙니다: {0}")]
    NotARepo(PathBuf),
    #[error("git을 실행할 수 없습니다: {0}")]
    GitUnavailable(String),
    #[error("git {args} 실패: {stderr}")]
    Git { args: String, stderr: String },
}

fn git(dir: &Path, args: &[&str]) -> Result<(bool, String, String), SourceError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| SourceError::GitUnavailable(e.to_string()))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    ))
}

fn git_ok(dir: &Path, args: &[&str]) -> Result<String, SourceError> {
    match git(dir, args)? {
        (true, out, _) => Ok(out),
        (false, _, stderr) => Err(SourceError::Git {
            args: args.join(" "),
            stderr,
        }),
    }
}

pub fn snapshot_local(name: &str, path: &Path) -> Result<Snapshot, SourceError> {
    if !path.is_dir() {
        return Err(SourceError::PathMissing(path.to_path_buf()));
    }
    let root = match git(path, &["rev-parse", "--show-toplevel"])? {
        (true, out, _) => PathBuf::from(out),
        (false, _, _) => return Err(SourceError::NotARepo(path.to_path_buf())),
    };
    let sha = git_ok(&root, &["rev-parse", "HEAD"])?;
    let branch = match git(&root, &["symbolic-ref", "--short", "-q", "HEAD"])? {
        (true, out, _) if !out.is_empty() => Some(out),
        _ => None,
    };
    let remote = match git(&root, &["remote", "get-url", "origin"])? {
        (true, out, _) if !out.is_empty() => Some(out),
        _ => None,
    };
    // -z keeps paths with non-ASCII names (e.g. Korean) unquoted and one entry per NUL.
    let status = git_ok(&root, &["status", "--porcelain", "-z"])?;
    let dirty_entries = count_status_entries(&status);
    Ok(Snapshot {
        name: name.to_string(),
        root,
        sha,
        branch,
        remote,
        dirty_entries,
    })
}

/// Counts `git status --porcelain -z` entries. Renames and copies carry an extra
/// NUL-separated original path that must not be counted twice.
fn count_status_entries(z: &str) -> usize {
    let mut parts = z.split('\0').filter(|e| !e.is_empty());
    let mut n = 0;
    while let Some(entry) = parts.next() {
        n += 1;
        let xy = entry.get(..2).unwrap_or("");
        if xy.contains('R') || xy.contains('C') {
            parts.next();
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@example.invalid", "-c", "user.name=t"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q", "-b", "feat/화면-test"]);
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-q", "-m", "init"]);
        dir
    }

    #[test]
    fn clean_repo_snapshot() {
        let dir = repo();
        let s = snapshot_local("backend", dir.path()).unwrap();
        assert_eq!(s.sha, run(dir.path(), &["rev-parse", "HEAD"]));
        assert_eq!(s.branch.as_deref(), Some("feat/화면-test"));
        assert_eq!(s.remote, None);
        assert!(!s.is_preview());
    }

    #[test]
    fn dirty_entries_counted_including_korean_names() {
        let dir = repo();
        std::fs::write(dir.path().join("a.txt"), "changed").unwrap();
        std::fs::write(dir.path().join("새파일.md"), "x").unwrap();
        let s = snapshot_local("backend", dir.path()).unwrap();
        assert_eq!(s.dirty_entries, 2);
        assert!(s.is_preview());
    }

    #[test]
    fn rename_counts_once() {
        let dir = repo();
        run(dir.path(), &["mv", "a.txt", "b.txt"]);
        assert_eq!(snapshot_local("x", dir.path()).unwrap().dirty_entries, 1);
    }

    #[test]
    fn subdirectory_resolves_to_root_and_detached_head_has_no_branch() {
        let dir = repo();
        std::fs::create_dir(dir.path().join("lib")).unwrap();
        let sha = run(dir.path(), &["rev-parse", "HEAD"]);
        run(dir.path(), &["checkout", "-q", "--detach", &sha]);
        let s = snapshot_local("x", &dir.path().join("lib")).unwrap();
        assert_eq!(
            s.root.canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
        assert_eq!(s.branch, None);
    }

    #[test]
    fn non_repo_and_missing_path_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            snapshot_local("x", dir.path()),
            Err(SourceError::NotARepo(_))
        ));
        assert!(matches!(
            snapshot_local("x", &dir.path().join("nope")),
            Err(SourceError::PathMissing(_))
        ));
    }
}
