use std::path::{Component, Path, PathBuf};

/// Recursively lists files under `dir` (skipping dependency and build folders), sorted.
pub fn walk_files(dir: &Path, keep: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name();
            if p.is_dir() {
                if !matches!(
                    name.to_str(),
                    Some("node_modules" | ".next" | ".git" | "__pycache__")
                ) {
                    stack.push(p);
                }
            } else if keep(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Lexical normalization (`a/./b/../c` → `a/c`) without touching the filesystem.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `path` relative to `root` with `/` separators, for stable output.
pub fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
