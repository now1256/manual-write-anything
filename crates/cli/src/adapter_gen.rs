//! Writing a product's adapter from its code (D-049): people should not have to know the
//! adapter format. The writer reads the file layout and the files that hold the menu, API
//! calls, permissions and login, writes `adapter.toml`, and the product map built from it is
//! the check; one repair round with what went wrong.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

const FORMAT: &str = include_str!("../../../docs/adapter.md");

const SYSTEM: &str = "You write the adapter file (TOML) that lets a code reader understand one web \
product: where its menu is defined, how its pages call the API, how its frontend API routes \
forward to the backend, where its permissions are named and checked, and how users log in. You \
get the adapter format with an example, the file layout of the frontend (Next.js app router) and \
the backend (FastAPI), and the contents of the files most likely to matter. Use only names that \
appear in the given code (file paths relative to each code root, exported constant names, \
function names, cookie and field names); never invent. Leave out optional parts the code does \
not show. nav_sections list every exported menu array in nav_file. Return the TOML text, the \
product's name as its screens show it (product_name), and in notes, in Korean, what you could \
not find.";

/// What the reader is shown: the layout and the files that usually decide the adapter.
struct Gathered {
    front_tree: String,
    back_tree: String,
    files: Vec<(String, String)>,
    /// Menu files and the menu arrays each exports (found in the code, not by the model).
    navs: Vec<(String, Vec<String>)>,
}

/// Names of arrays a file exports (`export const MAIN_NAV: NavItem[] = [`).
fn exported_arrays(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim_start().strip_prefix("export const ")?;
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let after = rest[name.len()..].split_once('=')?.1.trim_start();
            (!name.is_empty() && after.starts_with('[')).then_some(name)
        })
        .collect()
}

/// Files a file imports through the alias (`@/src/lib/x` → `<root>/src/lib/x.ts`).
fn imported(root: &Path, text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|l| {
            let spec = l.trim().strip_prefix("import ")?.rsplit_once(" from ")?.1;
            let spec = spec.trim().trim_end_matches(';').trim_matches(['"', '\'']);
            let rel = spec.strip_prefix("@/")?;
            ["", ".ts", ".tsx", ".js", "/index.ts"]
                .iter()
                .map(|ext| root.join(format!("{rel}{ext}")))
                .find(|p| p.is_file())
        })
        .collect()
}

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".next",
    ".git",
    "dist",
    "build",
    "out",
    "coverage",
    "__pycache__",
    ".venv",
    "venv",
    "public",
    "migrations",
    "tests",
    "test",
];

/// Paths under `root` with one of `exts`, shallow first, at most `limit`.
fn files(root: &Path, exts: &[&str], limit: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if p.is_dir() {
                if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                    queue.push_back(p);
                }
            } else if p
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| exts.contains(&x))
            {
                out.push(p);
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

fn head(p: &Path, lines: usize) -> String {
    std::fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .take(lines)
        .collect::<Vec<_>>()
        .join("\n")
}

fn gather(front: &Path, back: &Path) -> Gathered {
    let fe = files(front, &["ts", "tsx", "js", "jsx", "json"], 20_000);
    let be = files(back, &["py"], 10_000);
    let tree = |root: &Path, all: &[PathBuf], max: usize| {
        let mut t: Vec<String> = all.iter().map(|p| rel(root, p)).collect();
        if t.len() > max {
            t.truncate(max);
            t.push("… (더 있음)".into());
        }
        t.join("\n")
    };
    let mut picked: Vec<(String, String)> = Vec::new();
    let pick = |root: &Path, p: &Path, lines: usize, picked: &mut Vec<(String, String)>| {
        let name = rel(root, p);
        if !picked.iter().any(|(n, _)| *n == name) {
            picked.push((name, head(p, lines)));
        }
    };
    let text = |p: &Path| std::fs::read_to_string(p).unwrap_or_default();
    let named = |p: &Path, words: &[&str]| {
        let s = p.to_string_lossy().to_lowercase();
        words.iter().any(|w| s.contains(w))
    };

    for f in ["tsconfig.json", "package.json"] {
        let p = front.join(f);
        if p.is_file() {
            pick(front, &p, 60, &mut picked);
        }
    }
    // The menu: files named like a nav/menu/sidebar that export arrays with hrefs.
    let mut menus: Vec<&PathBuf> = fe
        .iter()
        .filter(|p| named(p, &["nav", "menu", "sidebar"]))
        .filter(|p| text(p).contains("href"))
        .collect();
    menus.sort_by_key(|p| std::cmp::Reverse(text(p).matches("href").count()));
    let mut navs = Vec::new();
    for p in menus.into_iter().take(2) {
        // Whole: every menu array matters.
        pick(front, p, 1500, &mut picked);
        navs.push((rel(front, p), exported_arrays(&text(p))));
    }
    // How pages call the API: helpers that wrap fetch.
    for p in fe
        .iter()
        .filter(|p| !p.to_string_lossy().contains("/app/"))
        .filter(|p| {
            let t = text(p);
            t.contains("fetch(") && t.contains("export") && t.contains("/api")
        })
        .take(3)
    {
        pick(front, p, 80, &mut picked);
    }
    // API routes and the helpers they forward with.
    let routes: Vec<&PathBuf> = fe
        .iter()
        .filter(|p| p.ends_with("route.ts") || p.ends_with("route.js"))
        .collect();
    for p in routes.iter().take(3) {
        pick(front, p, 50, &mut picked);
        // The helpers a route forwards with, where they are defined.
        for q in imported(front, &text(p)).into_iter().take(3) {
            pick(front, &q, 120, &mut picked);
        }
    }
    for p in fe
        .iter()
        .filter(|p| named(p, &["proxy", "upstream", "backend", "forward"]))
        .take(2)
    {
        pick(front, p, 80, &mut picked);
    }
    // Login.
    for p in routes
        .iter()
        .filter(|p| named(p, &["login", "auth", "session"]))
        .take(3)
    {
        pick(front, p, 60, &mut picked);
    }
    // Permissions on the backend: where they are named, and how a route checks one.
    let mut perms: Vec<&PathBuf> = be
        .iter()
        .filter(|p| named(p, &["permission", "perm", "rbac", "acl", "license"]))
        .collect();
    perms.sort_by_key(|p| std::cmp::Reverse(text(p).len().min(20_000)));
    for p in perms.into_iter().take(2) {
        pick(back, p, 120, &mut picked);
    }
    if let Some(p) = be
        .iter()
        .find(|p| text(p).contains("Depends(") && text(p).to_lowercase().contains("permission"))
    {
        pick(back, p, 60, &mut picked);
    }
    Gathered {
        front_tree: tree(front, &fe, 500),
        back_tree: tree(back, &be, 300),
        files: picked,
        navs,
    }
}

#[derive(Deserialize)]
struct Written {
    toml: String,
    #[serde(default)]
    product_name: String,
    #[serde(default)]
    notes: String,
}

async fn write(d: &mwa_core::Dispatcher, prompt: &str) -> Result<Written> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "toml": {"type": "string"},
            "product_name": {"type": "string"},
            "notes": {"type": "string"}
        },
        "required": ["toml", "product_name", "notes"]
    });
    let out = d
        .call(
            mwa_protocol::Role::Writer,
            SYSTEM,
            prompt,
            &[],
            Some(&schema),
        )
        .await
        .map_err(|e| anyhow::anyhow!("어댑터 쓰기 실패: {e}"))?;
    serde_json::from_value(out.response.json.unwrap_or_default())
        .context("어댑터 쓰기: 응답 형식이 맞지 않습니다")
}

/// Which source is the Next.js frontend and which the FastAPI backend.
fn pick_sources(cfg: &mwa_protocol::Config) -> Result<(String, PathBuf, String, PathBuf)> {
    let kinds: Vec<(String, PathBuf, &'static str)> = cfg
        .sources
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                s.path.clone(),
                crate::setup::code_kind(&s.path).unwrap_or(""),
            )
        })
        .collect();
    let find = |want: &str| kinds.iter().find(|(_, _, k)| k.contains(want)).cloned();
    let (Some(f), Some(b)) = (find("Next.js"), find("FastAPI")) else {
        let seen: Vec<String> = kinds
            .iter()
            .map(|(n, _, k)| format!("{n}: {}", if k.is_empty() { "알 수 없음" } else { k }))
            .collect();
        anyhow::bail!(
            "Next.js 프론트와 FastAPI 백엔드 코드를 찾지 못했습니다 ({}). 코드 소스가 각 앱 폴더(package.json, FastAPI 코드가 있는 곳)를 가리키는지 확인하세요",
            seen.join(", ")
        );
    };
    Ok((f.0, f.1, b.0, b.1))
}

/// Writes `adapter.toml` from the code, points `[product]` at it and builds the product map.
pub async fn make(ws: &Path, progress: impl Fn(String) + Send + Sync) -> Result<Vec<String>> {
    let cfg = crate::load_config(ws)?;
    let (front_name, front, back_name, back) = pick_sources(&cfg)?;
    progress(format!(
        "코드 구조 읽는 중 (프론트 {front_name}, 백엔드 {back_name})"
    ));
    let ctx = gather(&front, &back);
    let base = format!(
        "## 어댑터 형식\n{FORMAT}\n\n## 메뉴 파일과 내보내는 메뉴 배열 (코드에서 찾음)\n{}\n\n## 프론트 파일 목록 ({front_name})\n{}\n\n## 백엔드 파일 목록 ({back_name})\n{}\n\n## 중요한 파일\n{}",
        ctx.navs
            .iter()
            .map(|(f, a)| format!("- {f}: {}", a.join(", ")))
            .collect::<Vec<_>>()
            .join("\n"),
        ctx.front_tree,
        ctx.back_tree,
        ctx.files
            .iter()
            .map(|(n, t)| format!("### {n}\n```\n{t}\n```"))
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    let d = crate::open_dispatcher(ws, cfg.clone())?;
    let path = ws.join("adapter.toml");
    // Keep a hand-made adapter: the new one is written next to it first.
    let previous = std::fs::read_to_string(&path).ok();

    let mut prompt = base.clone();
    let mut last_err = String::new();
    for round in 1..=2 {
        progress(if round == 1 {
            "어댑터 쓰는 중".into()
        } else {
            format!("어댑터 고치는 중: {last_err}")
        });
        let w = write(&d, &prompt).await?;
        let missing = match mwa_productmap::adapters::from_str(&w.toml) {
            Ok(a) => ctx
                .navs
                .iter()
                .find(|(f, _)| *f == a.nav_file)
                .map(|(_, all)| {
                    all.iter()
                        .filter(|n| !a.nav_sections.iter().any(|s| s.0 == n.as_str()))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        if let Err(e) = mwa_productmap::adapters::from_str(&w.toml) {
            last_err = format!("형식 오류: {e}");
        } else if round == 1 && !missing.is_empty() {
            last_err = format!(
                "nav_sections에 메뉴 파일의 메뉴 배열이 빠졌습니다: {} (각각의 licenseMenu·묶음 이름도 확인)",
                missing.join(", ")
            );
        } else {
            std::fs::write(&path, &w.toml)?;
            set_product(ws, &w.product_name, &front_name, &back_name)?;
            let cfg = crate::load_config(ws)?;
            match crate::build_map(ws, &cfg, None, true).await {
                Ok((map, _, _)) if !map.features.is_empty() => {
                    let screens: usize = map.features.iter().map(|f| f.screens.len()).sum();
                    let apis: usize = map
                        .features
                        .iter()
                        .flat_map(|f| f.screens.iter())
                        .map(|s| s.apis.len())
                        .sum();
                    let mut lines = vec![format!(
                        "제품을 파악했습니다: 기능(메뉴) {}개, 화면 {screens}개, API 호출 {apis}개, 권한 {}개 · 사용량 정가 환산 ${:.3}",
                        map.features.len(),
                        map.permissions.len(),
                        d.spent_usd()
                    )];
                    if !w.notes.trim().is_empty() {
                        lines.push(format!("참고: {}", w.notes.trim()));
                    }
                    lines.push(format!("어댑터: {}", path.display()));
                    return Ok(lines);
                }
                Ok(_) => {
                    last_err = "메뉴를 하나도 찾지 못했습니다 (nav_file, nav_sections 확인)".into()
                }
                Err(e) => last_err = format!("제품 지도 오류: {e:#}"),
            }
        }
        prompt = format!(
            "{base}\n\n## 지난번에 쓴 어댑터\n```toml\n{}\n```\n\n## 문제\n{last_err}\n고친 어댑터 전체를 다시 쓰세요.",
            w.toml
        );
    }
    if let Some(old) = previous {
        std::fs::write(&path, old)?;
    }
    anyhow::bail!(
        "어댑터를 만들지 못했습니다: {last_err}. 설정의 \"제품\"에서 어댑터 파일을 직접 고를 수 있습니다"
    )
}

/// `[product]` pointing at the project's `adapter.toml`, keeping a name already set.
fn set_product(ws: &Path, name: &str, front: &str, back: &str) -> Result<()> {
    let mut s = crate::settings::read(ws).map_err(|e| anyhow::anyhow!(e))?;
    let keep = s
        .product
        .as_ref()
        .map(|p| (p.name.clone(), p.openapi.clone()));
    let (old_name, openapi) = keep.unwrap_or_default();
    s.product = Some(crate::settings::ProductView {
        name: if old_name.trim().is_empty() {
            name.trim().to_string()
        } else {
            old_name
        },
        adapter: "adapter.toml".into(),
        frontend: front.into(),
        backend: back.into(),
        openapi,
    });
    crate::settings::save(ws, &s).map_err(|e| anyhow::anyhow!(e))
}
