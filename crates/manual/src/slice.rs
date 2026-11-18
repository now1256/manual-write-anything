//! The code a feature consists of, with line numbers, for fact extraction and quote checks.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mwa_productmap::graph::{Coverage, Project};
use tree_sitter::{Node, Parser};

use crate::ManualError;
use crate::model::Repo;

#[derive(Debug, Clone)]
pub struct SliceFile {
    pub repo: Repo,
    /// Relative to the repository root.
    pub path: String,
    pub lines: Vec<String>,
    /// 1-based inclusive line ranges included in the slice.
    pub ranges: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, Default)]
pub struct Slice {
    pub files: Vec<SliceFile>,
}

/// Frontend directories with shared UI or utilities: not part of any feature's own behaviour.
pub const FRONTEND_EXCLUDE: &[&str] = &[
    "src/app/components/",
    "src/lib/utils",
    "src/lib/pagination",
    "src/lib/cn.ts",
    "src/lib/date.ts",
    "src/lib/hooks/use-table-query",
    "src/lib/hooks/use-clamp-page",
];

impl Slice {
    pub fn file(&self, repo: Repo, path: &str) -> Option<&SliceFile> {
        self.files.iter().find(|f| f.repo == repo && f.path == path)
    }

    /// Text sent to the model: per file a header and numbered lines of the included ranges.
    /// Files are packed into chunks of at most `max_chars` (a large file gets its own chunk).
    pub fn chunks(&self, max_chars: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        for f in &self.files {
            let mut text = format!("=== {}:{}\n", repo_name(f.repo), f.path);
            for (a, b) in &f.ranges {
                for n in *a..=*b {
                    text.push_str(&format!("{n:>5}| {}\n", f.lines[n - 1]));
                }
                text.push_str("  ...\n");
            }
            if !cur.is_empty() && cur.len() + text.len() > max_chars {
                out.push(std::mem::take(&mut cur));
            }
            cur.push_str(&text);
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out
    }

    pub fn total_lines(&self) -> usize {
        self.files
            .iter()
            .flat_map(|f| &f.ranges)
            .map(|(a, b)| b - a + 1)
            .sum()
    }

    /// True if `quote` (whitespace-insensitive) appears within lines `start..=end` (±2 lines).
    pub fn quote_holds(
        &self,
        repo: Repo,
        path: &str,
        start: usize,
        end: usize,
        quote: &str,
    ) -> bool {
        let Some(f) = self.file(repo, path) else {
            return false;
        };
        let want = squash(quote);
        if want.is_empty() || start == 0 || end < start {
            return false;
        }
        let lo = start.saturating_sub(2).max(1);
        let hi = (end + 2).min(f.lines.len());
        if lo > hi {
            return false;
        }
        squash(&f.lines[lo - 1..hi].join("\n")).contains(&want)
    }
}

pub fn repo_name(r: Repo) -> &'static str {
    match r {
        Repo::Frontend => "frontend",
        Repo::Backend => "backend",
    }
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn read_lines(path: &Path) -> Result<Vec<String>, ManualError> {
    let text = std::fs::read_to_string(path).map_err(|e| ManualError::Io(path.to_path_buf(), e))?;
    Ok(text.lines().map(String::from).collect())
}

/// 1-based line of a byte offset.
fn line_of(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].matches('\n').count() + 1
}

/// Frontend part: everything under the screen's own folder, plus the reached declarations
/// of other project files outside shared UI and utilities.
pub fn frontend(
    project: &mut Project,
    pages: &[PathBuf],
    stop_dirs: &[&str],
) -> Result<Vec<SliceFile>, ManualError> {
    let root = project.root.clone();
    let mut merged: BTreeMap<PathBuf, Vec<(usize, usize)>> = BTreeMap::new();
    for page in pages {
        let page_dir = page.parent().unwrap_or(&root).to_path_buf();
        for (file, cov) in project.reach(page, stop_dirs)? {
            let rel = mwa_productmap::fsutil::rel(&root, &file);
            if FRONTEND_EXCLUDE.iter().any(|p| rel.starts_with(p)) {
                continue;
            }
            let text =
                std::fs::read_to_string(&file).map_err(|e| ManualError::Io(file.clone(), e))?;
            let total = text.lines().count().max(1);
            let ranges: Vec<(usize, usize)> = match &cov {
                _ if file.starts_with(&page_dir) => vec![(1, total)],
                Coverage::Whole => vec![(1, total)],
                Coverage::Ranges(rs) => rs
                    .iter()
                    .map(|r| {
                        (
                            line_of(&text, r.start),
                            line_of(&text, r.end.saturating_sub(1)),
                        )
                    })
                    .collect(),
            };
            merged.entry(file).or_default().extend(ranges);
        }
    }
    merged
        .into_iter()
        .map(|(file, ranges)| {
            Ok(SliceFile {
                repo: Repo::Frontend,
                path: mwa_productmap::fsutil::rel(&root, &file),
                lines: read_lines(&file)?,
                ranges: normalize_ranges(ranges),
            })
        })
        .collect()
}

pub fn normalize_ranges(mut rs: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    rs.sort();
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (a, b) in rs {
        match out.last_mut() {
            Some(last) if a <= last.1 + 1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

struct PyFile {
    rel: String,
    text: String,
    tree: tree_sitter::Tree,
}

/// Backend part for `operations` (`METHOD /raw/path` from OpenAPI): the route handler,
/// the service methods it calls (`Service.method(...)`) and the classes used in its signature.
pub fn backend(
    root: &Path,
    code_dir: &str,
    operations: &[String],
) -> Result<Vec<SliceFile>, ManualError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|e| ManualError::Invalid(e.to_string()))?;
    let files = mwa_productmap::fsutil::walk_files(&root.join(code_dir), &|p| {
        p.extension().is_some_and(|e| e == "py")
    });
    let mut py = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let Some(tree) = parser.parse(&text, None) else {
            continue;
        };
        py.push(PyFile {
            rel: mwa_productmap::fsutil::rel(root, &f),
            text,
            tree,
        });
    }

    let mut picked: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    let mut add = |rel: &str, text: &str, n: Node| {
        picked
            .entry(rel.to_string())
            .or_default()
            .push((line_of(text, n.start_byte()), line_of(text, n.end_byte())));
    };

    for op in operations {
        let Some((method, path)) = op.split_once(' ') else {
            continue;
        };
        let method = method.to_lowercase();
        for f in &py {
            for def in nodes(f.tree.root_node())
                .into_iter()
                .filter(|n| n.kind() == "decorated_definition")
            {
                let decorators: Vec<&str> = children(def)
                    .into_iter()
                    .filter(|c| c.kind() == "decorator")
                    .map(|c| &f.text[c.byte_range()])
                    .collect();
                let hit = decorators.iter().any(|d| {
                    let marker = format!(".{method}(");
                    d.find(&marker).is_some_and(|i| {
                        let rest = &d[i + marker.len()..];
                        first_string(rest).is_some_and(|p| !p.is_empty() && path.ends_with(&p))
                    })
                });
                if !hit {
                    continue;
                }
                add(&f.rel, &f.text, def);
                let body = &f.text[def.byte_range()];
                // Service calls `Name.method(` and type names used in the handler.
                for (class, member) in dotted_calls(body) {
                    for g in &py {
                        if let Some(m) = find_method(g, &class, &member) {
                            add(&g.rel, &g.text, m);
                        }
                    }
                }
                for ty in capitalized_words(body) {
                    for g in &py {
                        if let Some(c) = find_class(g, &ty) {
                            add(&g.rel, &g.text, c);
                        }
                    }
                }
            }
        }
    }

    picked
        .into_iter()
        .map(|(rel, ranges)| {
            let f = py
                .iter()
                .find(|f| f.rel == rel)
                .expect("picked from parsed files");
            Ok(SliceFile {
                repo: Repo::Backend,
                path: rel,
                lines: f.text.lines().map(String::from).collect(),
                ranges: normalize_ranges(ranges),
            })
        })
        .collect()
}

fn children(n: Node) -> Vec<Node> {
    let mut c = n.walk();
    n.children(&mut c).collect()
}

fn nodes(root: Node) -> Vec<Node> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        out.push(n);
        let mut c = n.walk();
        stack.extend(n.children(&mut c));
    }
    out
}

fn first_string(s: &str) -> Option<String> {
    let start = s.find(['"', '\''])?;
    let q = s[start..].chars().next()?;
    let rest = &s[start + 1..];
    let end = rest.find(q)?;
    Some(rest[..end].to_string())
}

/// `Upper.lower(` pairs, e.g. `AuthService.create_user(`.
fn dotted_calls(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_uppercase()
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
        {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'.' {
                let m_start = i + 1;
                let mut j = m_start;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'(' && j > m_start {
                    out.push((body[start..i].to_string(), body[m_start..j].to_string()));
                }
            }
        } else {
            i += 1;
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Capitalized identifiers ending in Request/Response/Model (pydantic models in signatures).
fn capitalized_words(body: &str) -> Vec<String> {
    let mut out: Vec<String> = body
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| w.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
        .filter(|w| w.ends_with("Request") || w.ends_with("Response") || w.ends_with("Model"))
        .map(String::from)
        .collect();
    out.sort();
    out.dedup();
    out
}

fn find_class<'a>(f: &'a PyFile, name: &str) -> Option<Node<'a>> {
    nodes(f.tree.root_node()).into_iter().find(|n| {
        n.kind() == "class_definition"
            && n.child_by_field_name("name")
                .is_some_and(|x| &f.text[x.byte_range()] == name)
    })
}

fn find_method<'a>(f: &'a PyFile, class: &str, method: &str) -> Option<Node<'a>> {
    let class_node = find_class(f, class)?;
    let body = class_node.child_by_field_name("body")?;
    nodes(body)
        .into_iter()
        .find(|n| {
            n.kind() == "function_definition"
                && n.child_by_field_name("name")
                    .is_some_and(|x| &f.text[x.byte_range()] == method)
        })
        .map(|n| match n.parent() {
            Some(p) if p.kind() == "decorated_definition" => p,
            _ => n,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_follows_handler_service_and_models() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(
            lib.join("api.py"),
            r#"from lib.svc import AuthService

@router.post("/users")
async def create_user(body: UserCreateRequest, request: Request):
    """create"""
    return await AuthService.create_user(body, request)

@router.get("/other")
async def other():
    return 1
"#,
        )
        .unwrap();
        std::fs::write(
            lib.join("svc.py"),
            r#"class AuthService:
    @staticmethod
    async def create_user(body, request):
        if exists:
            raise Conflict("이미 사용 중인 아이디입니다")
        return {}

    async def unrelated(self):
        return 2
"#,
        )
        .unwrap();
        std::fs::write(
            lib.join("schema.py"),
            "class UserCreateRequest(BaseModel):\n    user_id: str = Field(min_length=4)\n",
        )
        .unwrap();
        let files = backend(dir.path(), "lib", &["POST /users".to_string()]).unwrap();
        let by: BTreeMap<&str, &SliceFile> = files.iter().map(|f| (f.path.as_str(), f)).collect();
        assert_eq!(by["lib/api.py"].ranges, vec![(3, 6)]);
        assert_eq!(
            by["lib/svc.py"].ranges,
            vec![(2, 6)],
            "decorated method included, unrelated excluded"
        );
        assert_eq!(by["lib/schema.py"].ranges, vec![(1, 2)]);
    }

    #[test]
    fn chunks_and_quotes() {
        let s = Slice {
            files: vec![SliceFile {
                repo: Repo::Frontend,
                path: "a.tsx".into(),
                lines: vec![
                    "const a = 1;".into(),
                    "<Button>사용자 등록</Button>".into(),
                    "x".into(),
                ],
                ranges: vec![(1, 2)],
            }],
        };
        let c = s.chunks(10_000);
        assert_eq!(c.len(), 1);
        assert!(
            c[0].contains("=== frontend:a.tsx")
                && c[0].contains("    2| <Button>사용자 등록</Button>")
        );
        assert!(!c[0].contains("    3| x"));
        assert!(s.quote_holds(
            Repo::Frontend,
            "a.tsx",
            2,
            2,
            "<Button>사용자  등록</Button>"
        ));
        assert!(!s.quote_holds(Repo::Frontend, "a.tsx", 2, 2, "사용자 삭제"));
        assert!(!s.quote_holds(Repo::Backend, "a.tsx", 2, 2, "사용자"));
        assert_eq!(
            normalize_ranges(vec![(5, 6), (1, 3), (4, 4), (9, 9)]),
            vec![(1, 6), (9, 9)]
        );
    }
}
