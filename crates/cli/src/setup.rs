//! First-run help (M-App): per-field checks for the settings page and the "시작하기" checklist.
//! Checks only read (files, `git ls-remote`, HTTP GET); nothing is written anywhere.

use std::path::{Path, PathBuf};
use std::time::Duration;

use mwa_host::{Check, SetupStep};
use mwa_protocol::Config;

fn ok(detail: impl Into<String>) -> Check {
    Check {
        ok: true,
        detail: detail.into(),
    }
}

fn bad(detail: impl Into<String>) -> Check {
    Check {
        ok: false,
        detail: detail.into(),
    }
}

fn abs(ws: &Path, p: &str) -> PathBuf {
    let p = match p.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(rest))
            .unwrap_or_else(|| PathBuf::from(p)),
        None => PathBuf::from(p),
    };
    if p.is_relative() { ws.join(p) } else { p }
}

/// What kind of code a folder holds, from its top-level manifest files.
fn code_kind(dir: &Path) -> Option<&'static str> {
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap_or_default();
    let pkg = read("package.json");
    if pkg.contains("\"next\"") {
        return Some("Next.js 프론트엔드");
    }
    if !pkg.is_empty() {
        return Some("JavaScript 프로젝트");
    }
    let py = read("pyproject.toml") + &read("requirements.txt");
    if py.to_lowercase().contains("fastapi") {
        return Some("FastAPI 백엔드");
    }
    // No manifest naming FastAPI: look for the import in the top two levels (bounded).
    let mut py_files = 0;
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.filter_map(Result::ok) {
            let p = e.path();
            if p.is_dir() && depth < 1 && !e.file_name().to_string_lossy().starts_with('.') {
                stack.push((p, depth + 1));
            } else if p.extension().is_some_and(|x| x == "py") && py_files < 400 {
                py_files += 1;
                let mut head = vec![0u8; 4096];
                let n = std::fs::File::open(&p)
                    .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
                    .unwrap_or(0);
                let text = String::from_utf8_lossy(&head[..n]);
                if text.contains("from fastapi") || text.contains("import fastapi") {
                    return Some("FastAPI 백엔드");
                }
            }
        }
    }
    if !py.is_empty() || py_files > 0 {
        return Some("Python 프로젝트");
    }
    None
}

fn check_folder(ws: &Path, value: &str) -> Check {
    if value.trim().is_empty() {
        return bad("폴더를 고르세요");
    }
    let dir = abs(ws, value.trim());
    if !dir.is_dir() {
        return bad(format!("폴더가 없습니다: {}", dir.display()));
    }
    let kind = code_kind(&dir)
        .map(|k| format!("{k} · "))
        .unwrap_or_default();
    match mwa_source::snapshot_local("check", &dir) {
        Ok(s) => {
            let branch = s.branch.as_deref().unwrap_or("(분리된 HEAD)");
            let remote = s
                .remote
                .as_deref()
                .map(|r| format!(" · 원격 {r}"))
                .unwrap_or_default();
            let dirty = if s.dirty_entries > 0 {
                format!(
                    " · 커밋 안 한 변경 {}개(미리보기로만 사용)",
                    s.dirty_entries
                )
            } else {
                String::new()
            };
            ok(format!(
                "{kind}Git {branch} · {}{remote}{dirty}",
                &s.sha[..s.sha.len().min(7)]
            ))
        }
        Err(e) => bad(format!(
            "{kind}git 저장소가 아닙니다. 버전을 고정하려면 git 저장소여야 합니다 ({e})"
        )),
    }
}

fn check_git(url: &str, branch: &str) -> Check {
    if url.trim().is_empty() {
        return bad("git 주소를 적으세요");
    }
    let refname = if branch.trim().is_empty() {
        "HEAD"
    } else {
        branch.trim()
    };
    let out = std::process::Command::new("git")
        .args(["ls-remote", url.trim(), refname])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o ConnectTimeout=10",
        )
        .output();
    match out {
        Err(e) => bad(format!("git을 실행할 수 없습니다: {e}")),
        Ok(o) if !o.status.success() => {
            let err = String::from_utf8_lossy(&o.stderr);
            let last = err
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            bad(format!(
                "접속하지 못했습니다: {last} (SSH 키나 git 자격 증명을 확인하세요)"
            ))
        }
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            match text
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().next())
            {
                Some(sha) => ok(format!(
                    "접속됨 · {refname} {} · 저장 후 체크리스트의 \"받기\"로 클론합니다",
                    &sha[..sha.len().min(7)]
                )),
                None => bad(format!("브랜치 '{refname}'이 없습니다")),
            }
        }
    }
}

fn check_adapter(ws: &Path, value: &str) -> Check {
    let path = abs(ws, value.trim());
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return bad(format!("파일이 없습니다: {}", path.display())),
    };
    let map = match mwa_productmap::adapters::from_str(&text) {
        Ok(a) => a,
        Err(e) => return bad(format!("어댑터 형식 오류: {e}")),
    };
    match mwa_capture::spec::from_str(&text) {
        Ok(_) => ok(format!("어댑터 '{}' · 로그인·촬영 설정 포함", map.name)),
        Err(e) => bad(format!(
            "어댑터 '{}'에 [auth]/[ui]가 없거나 틀렸습니다: {e}",
            map.name
        )),
    }
}

async fn http_get(url: &str) -> Result<(u16, String), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|e| e.to_string())?;
    let r = client.get(url).send().await.map_err(|e| {
        if e.is_timeout() {
            "6초 안에 응답이 없습니다".to_string()
        } else {
            format!("접속하지 못했습니다: {e}")
        }
    })?;
    let status = r.status().as_u16();
    let body = r.text().await.unwrap_or_default();
    Ok((status, body))
}

async fn check_openapi(ws: &Path, value: &str) -> Check {
    let v = value.trim();
    if v.is_empty() {
        return ok("비워 두면 백엔드 오퍼레이션 연결을 건너뜁니다");
    }
    let text = if v.starts_with("http://") || v.starts_with("https://") {
        match http_get(v).await {
            Ok((200, body)) => body,
            Ok((status, _)) => return bad(format!("HTTP {status}")),
            Err(e) => return bad(e),
        }
    } else {
        match std::fs::read_to_string(abs(ws, v)) {
            Ok(t) => t,
            Err(e) => return bad(format!("파일을 읽을 수 없습니다: {e}")),
        }
    };
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(doc) => match doc["paths"].as_object() {
            Some(p) => ok(format!("OpenAPI · 경로 {}개", p.len())),
            None => bad("JSON이지만 OpenAPI(paths)가 아닙니다"),
        },
        Err(_) => bad("JSON이 아닙니다"),
    }
}

async fn check_site(value: &str) -> Check {
    let v = value.trim();
    if !(v.starts_with("http://") || v.starts_with("https://")) {
        return bad("http:// 또는 https://로 시작해야 합니다");
    }
    match http_get(v).await {
        Ok((status, _)) => ok(format!("접속됨 · HTTP {status}")),
        Err(e) => bad(e),
    }
}

fn check_file(ws: &Path, value: &str) -> Check {
    if value.trim().is_empty() {
        return bad("파일을 고르세요");
    }
    let path = abs(ws, value.trim());
    if !path.is_file() {
        return bad(format!("파일이 없습니다: {}", path.display()));
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
    {
        let info = std::process::Command::new("pdfinfo").arg(&path).output();
        return match info {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                let pages = text
                    .lines()
                    .find_map(|l| l.strip_prefix("Pages:"))
                    .map(|p| p.trim().to_string())
                    .unwrap_or_else(|| "?".into());
                ok(format!(
                    "PDF {pages}쪽 · 본문과 앞쪽 페이지 이미지로 양식을 배웁니다"
                ))
            }
            _ => bad(
                "PDF를 읽을 수 없습니다 (poppler 설치 필요: apt install poppler-utils / brew install poppler)",
            ),
        };
    }
    match std::fs::read_to_string(&path) {
        Ok(t) => ok(format!("{}줄", t.lines().count())),
        Err(_) => bad("텍스트(Markdown) 파일이 아닙니다"),
    }
}

fn check_command(value: &str) -> Check {
    let v = value.trim();
    let Some(path) = mwa_backends::find_command(v) else {
        return bad(format!(
            "'{v}'을 찾을 수 없습니다. 설치하거나 전체 경로를 적으세요"
        ));
    };
    let version = std::process::Command::new(&path)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    ok(format!("{} {version}", path.display()).trim().to_string())
}

pub async fn check(ws: &Path, kind: &str, value: &str, extra: &str) -> Check {
    match kind {
        // A saved source by name: what code it holds ("frontend"/"backend" guess in detail).
        "source" => {
            let Ok((mut cfg, _)) = std::fs::read_to_string(ws.join(crate::CONFIG_FILE))
                .map_err(|e| e.to_string())
                .and_then(|t| Config::parse(&t).map_err(|e| e.to_string()))
            else {
                return bad("설정을 읽을 수 없습니다");
            };
            cfg.resolve_paths(ws);
            match cfg.sources.iter().find(|s| s.name == value) {
                None => bad("저장된 소스가 아닙니다"),
                Some(s) => match code_kind(&s.path) {
                    Some(k) => ok(k),
                    None if s.path.is_dir() => ok("알 수 없는 종류"),
                    None => bad("아직 받지 않았습니다"),
                },
            }
        }
        "folder" => check_folder(ws, value),
        "git" => {
            let (url, branch) = (value.to_string(), extra.to_string());
            tokio::task::spawn_blocking(move || check_git(&url, &branch))
                .await
                .unwrap_or_else(|e| bad(e.to_string()))
        }
        "adapter" => check_adapter(ws, value),
        "openapi" => check_openapi(ws, value).await,
        "site" => check_site(value).await,
        "file" => check_file(ws, value),
        "command" => check_command(value),
        other => bad(format!("알 수 없는 확인 종류: {other}")),
    }
}

/// The "시작하기" checklist, in order. `action` is `settings` or `command:<name>`.
pub fn steps(ws: &Path, cfg: &Config) -> Vec<SetupStep> {
    let step = |key: &str, label: &str, done: bool, detail: String, action: &str| SetupStep {
        key: key.into(),
        label: label.into(),
        done,
        detail,
        action: action.into(),
    };
    let mut out = Vec::new();

    let missing: Vec<&str> = cfg
        .sources
        .iter()
        .filter(|s| !s.path.is_dir())
        .map(|s| s.name.as_str())
        .collect();
    let git_missing = cfg
        .sources
        .iter()
        .any(|s| s.git.is_some() && !s.path.is_dir());
    out.push(if cfg.sources.is_empty() {
        step(
            "code",
            "코드 연결",
            false,
            "프론트·백엔드 코드를 폴더나 Git으로 연결합니다".into(),
            "settings",
        )
    } else if git_missing {
        step(
            "code",
            "코드 연결",
            false,
            format!("Git 소스를 아직 받지 않았습니다: {}", missing.join(", ")),
            "command:sync",
        )
    } else if !missing.is_empty() {
        step(
            "code",
            "코드 연결",
            false,
            format!("폴더가 없습니다: {}", missing.join(", ")),
            "settings",
        )
    } else {
        step(
            "code",
            "코드 연결",
            true,
            format!("소스 {}개", cfg.sources.len()),
            "settings",
        )
    });

    out.push(match &cfg.product {
        None => step(
            "product",
            "제품",
            false,
            "제품 이름, 어댑터, 프론트·백엔드를 고릅니다".into(),
            "settings",
        ),
        Some(p) if !abs(ws, &p.adapter).is_file() => step(
            "product",
            "제품",
            false,
            format!("어댑터 파일이 없습니다: {}", p.adapter),
            "settings",
        ),
        Some(p) => step(
            "product",
            "제품",
            true,
            p.name.clone().unwrap_or_else(|| "이름 없음".into()),
            "settings",
        ),
    });

    let account = cfg.capture.as_ref().and_then(|c| {
        let p = cfg.profiles.get(&c.profile)?;
        Some((c.profile.clone(), p.user_id.clone(), p.base_url.clone()))
    });
    out.push(match account {
        None => step(
            "account",
            "사이트와 계정",
            false,
            "촬영할 사이트 주소와 계정을 정합니다".into(),
            "settings",
        ),
        Some((name, user, url)) => {
            use mwa_capture::creds::{CredState, CredStore, key};
            let state = CredStore::default_path()
                .and_then(|p| CredStore::open(&p))
                .ok()
                .and_then(|s| s.get(&key(&user, &url)).map(|c| c.state));
            match state {
                Some(CredState::Ok) => step(
                    "account",
                    "사이트와 계정",
                    true,
                    format!("{name}: {user} @ {url}"),
                    "settings",
                ),
                Some(_) => step(
                    "account",
                    "사이트와 계정",
                    false,
                    format!("{name}: 자동 로그인이 멈춰 있습니다. 비밀번호를 다시 저장하세요"),
                    "settings",
                ),
                None => step(
                    "account",
                    "사이트와 계정",
                    false,
                    format!("{name}: 비밀번호를 저장하세요"),
                    "settings",
                ),
            }
        }
    });

    let learned = ws.join(mwa_manual::pipeline::TEMPLATE_FILE).is_file();
    let sample = cfg.manual.template.as_deref().map(|t| abs(ws, t));
    out.push(if learned {
        step(
            "template",
            "매뉴얼 양식",
            true,
            "학습한 양식이 있습니다".into(),
            "settings",
        )
    } else {
        match sample {
            Some(p) if p.is_file() => step(
                "template",
                "매뉴얼 양식",
                true,
                format!("견본: {}", p.display()),
                "settings",
            ),
            Some(p) => step(
                "template",
                "매뉴얼 양식",
                false,
                format!("견본 파일이 없습니다: {}", p.display()),
                "settings",
            ),
            // Optional: old manuals (①) or finished manuals are the model for the format.
            None => step(
                "template",
                "매뉴얼 양식",
                true,
                "선택 — 견본이 없으면 기존 매뉴얼과 이미 만든 매뉴얼을 본보기로 씁니다".into(),
                "settings",
            ),
        }
    });

    let has_map = ws.join("map/product-map.json").is_file();
    out.push(step(
        "map",
        "제품 지도",
        has_map,
        if has_map {
            "만들었습니다".into()
        } else {
            "코드에서 메뉴·화면·API를 뽑습니다 (LLM 없음)".into()
        },
        "command:map",
    ));
    let shots = mwa_capture::menus::load_manifest(ws);
    out.push(step(
        "shots",
        "기능 첫 화면 촬영",
        shots.is_some(),
        match &shots {
            Some(m) => format!("{}장", m.shots.iter().filter(|s| s.ok).count()),
            None => {
                "기능마다 첫 화면을 찍어 새 매뉴얼의 화면 그림으로 씁니다 (쓰기 요청은 막음)".into()
            }
        },
        "command:shot",
    ));
    let manuals = std::fs::read_dir(ws.join("manual/docs"))
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| e.path().join("doc.json").is_file())
                .count()
        })
        .unwrap_or(0);
    out.push(step(
        "manual",
        "첫 매뉴얼",
        manuals > 0,
        if manuals > 0 {
            format!("{manuals}개 기능")
        } else {
            "아래 매뉴얼 현황에서 기능을 골라 ①~④ 순서로 만듭니다".into()
        },
        "",
    ));
    out
}

/// One folder level for the picker. `ext` (e.g. "toml,md") also lists matching files.
pub fn browse(path: Option<&str>, ext: Option<&str>) -> Result<serde_json::Value, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/".into());
    let dir = match path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => abs(&home, p),
        None => home,
    };
    let exts: Vec<String> = ext
        .unwrap_or("")
        .split(',')
        .map(|e| e.trim().trim_start_matches('.').to_lowercase())
        .filter(|e| !e.is_empty())
        .collect();
    let rd = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut entries: Vec<serde_json::Value> = Vec::new();
    for e in rd.filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let p = e.path();
        let is_dir = p.is_dir();
        let matches = !is_dir
            && (exts.iter().any(|e| e == "*")
                || p.extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| exts.contains(&x.to_lowercase())));
        if is_dir || matches {
            entries.push(serde_json::json!({
                "name": name,
                "dir": is_dir,
                "git": is_dir && p.join(".git").exists(),
            }));
        }
    }
    entries.sort_by(|a, b| {
        (
            !a["dir"].as_bool().unwrap_or(false),
            a["name"].as_str().unwrap_or(""),
        )
            .cmp(&(
                !b["dir"].as_bool().unwrap_or(false),
                b["name"].as_str().unwrap_or(""),
            ))
    });
    Ok(serde_json::json!({
        "path": dir.display().to_string(),
        "parent": dir.parent().map(|p| p.display().to_string()),
        "entries": entries,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_and_file_checks() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        assert!(!check_folder(ws, "nope").ok);
        std::fs::create_dir_all(ws.join("web")).unwrap();
        std::fs::write(
            ws.join("web/package.json"),
            r#"{"dependencies":{"next":"15"}}"#,
        )
        .unwrap();
        let c = check_folder(ws, "web");
        assert!(!c.ok && c.detail.contains("Next.js"), "{c:?}");
        assert!(check_file(ws, "web/package.json").ok);
        assert!(!check_adapter(ws, "web/package.json").ok);
        assert!(!check_command("definitely-not-a-command-xyz").ok);
        std::fs::create_dir_all(ws.join("api/lib")).unwrap();
        std::fs::write(ws.join("api/lib/main.py"), "from fastapi import FastAPI\n").unwrap();
        assert_eq!(code_kind(&ws.join("api")), Some("FastAPI 백엔드"));
        let b = browse(Some(ws.to_str().unwrap()), Some("json")).unwrap();
        let names: Vec<&str> = b["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["name"].as_str())
            .collect();
        assert_eq!(names, ["api", "web"]);
    }

    #[test]
    fn checklist_for_a_new_project() {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, _) = Config::parse(crate::TEMPLATE).unwrap();
        let s = steps(dir.path(), &cfg);
        assert_eq!(s[0].key, "code");
        // Only the optional format sample counts as done in a new project.
        let done: Vec<_> = s
            .iter()
            .filter(|x| x.done)
            .map(|x| x.key.as_str())
            .collect();
        assert_eq!(done, ["template"]);
    }
}
