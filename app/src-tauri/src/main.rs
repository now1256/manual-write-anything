//! Desktop shell for mwa (D-029): starts the bundled `mwa` engine on a free loopback port
//! for the local project and shows its web UI in the app window. The engine is the same
//! binary as the CLI; the window talks to it over HTTP/WebSocket like a browser would.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::File;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

/// The running engine, killed when the app exits.
struct Engine(Mutex<Option<Child>>);

const TITLE: &str = "Manual Write Anything";

fn main() {
    let app = tauri::Builder::default()
        .manage(Engine(Mutex::new(None)))
        .setup(|app| {
            // Show the bundled loading page at once, then move to the engine's UI when ready.
            let window =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title(TITLE)
                    .inner_size(1440.0, 900.0)
                    .min_inner_size(960.0, 640.0)
                    .build()?;
            let handle = app.handle().clone();
            std::thread::spawn(move || match start_engine(&handle) {
                Ok(port) => {
                    let url = format!("http://127.0.0.1:{port}/")
                        .parse()
                        .expect("valid url");
                    let _ = window.navigate(url);
                }
                Err(e) => {
                    let msg = serde_json::Value::String(e).to_string();
                    let _ = window.eval(format!("window.showError({msg})"));
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the app");
    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            if let Some(mut child) = handle.state::<Engine>().0.lock().unwrap().take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    });
}

/// Starts `mwa serve` for the default project and waits until it accepts connections.
fn start_engine(app: &AppHandle) -> Result<u16, String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("앱 데이터 폴더를 찾을 수 없습니다: {e}"))?;
    let project = data.join("projects").join("default");
    std::fs::create_dir_all(&project).map_err(|e| format!("{}: {e}", project.display()))?;
    let engine = engine_path()?;
    let path = user_path();
    let log_path = data.join("engine.log");
    let log = File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let err_log = log
        .try_clone()
        .map_err(|e| format!("{}: {e}", log_path.display()))?;

    let command = |args: &[&str]| {
        let mut c = Command::new(&engine);
        c.arg("-C").arg(&project).args(args).env("PATH", &path);
        // The capture runner is bundled next to the app's resources (see build-mac.sh).
        if let Ok(res) = app.path().resource_dir() {
            let script = res.join("capture").join("index.mjs");
            if script.is_file() {
                c.env("MWA_SIDECAR", script);
            }
        }
        c
    };

    if !project.join("mwa.toml").is_file() {
        let out = command(&["init"])
            .output()
            .map_err(|e| format!("엔진을 실행할 수 없습니다 ({}): {e}", engine.display()))?;
        if !out.status.success() {
            return Err(format!(
                "프로젝트를 만들지 못했습니다: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }

    let port = free_port()?;
    let addr = format!("127.0.0.1:{port}");
    let mut child = command(&["serve", "--addr", &addr])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err_log))
        .spawn()
        .map_err(|e| format!("엔진을 실행할 수 없습니다 ({}): {e}", engine.display()))?;

    let target: SocketAddr = addr.parse().expect("valid address");
    let started = Instant::now();
    loop {
        if TcpStream::connect_timeout(&target, Duration::from_millis(200)).is_ok() {
            break;
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "엔진이 시작하자마자 종료됐습니다({status}). 기록: {}",
                log_path.display()
            ));
        }
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            return Err(format!(
                "엔진이 30초 안에 준비되지 않았습니다. 기록: {}",
                log_path.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    *app.state::<Engine>().0.lock().unwrap() = Some(child);
    Ok(port)
}

/// The engine is bundled as an external binary next to the app executable.
fn engine_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("실행 파일 위치를 알 수 없습니다")?;
    let name = if cfg!(windows) { "mwa.exe" } else { "mwa" };
    let p = dir.join(name);
    if p.is_file() {
        Ok(p)
    } else {
        Err(format!("내장 엔진이 없습니다: {}", p.display()))
    }
}

fn free_port() -> Result<u16, String> {
    let l = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    Ok(l.local_addr().map_err(|e| e.to_string())?.port())
}

/// Apps started from Finder get a minimal PATH, so `claude` and `node` are not found.
/// Ask the user's login shell for its PATH (bounded wait) and add the usual install dirs.
fn user_path() -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(p) = login_shell_path() {
        parts.extend(p.split(':').map(String::from));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for d in [".local/bin", ".cargo/bin", ".npm-global/bin", ".volta/bin"] {
            parts.push(home.join(d).display().to_string());
        }
    }
    for d in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        parts.push(d.to_string());
    }
    if let Ok(p) = std::env::var("PATH") {
        parts.extend(p.split(':').map(String::from));
    }
    let mut seen = std::collections::HashSet::new();
    parts.retain(|p| !p.is_empty() && seen.insert(p.clone()));
    parts.join(":")
}

fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    if !Path::new(&shell).is_file() {
        return None;
    }
    let mut child = Command::new(&shell)
        .args(["-ilc", "printf '__MWA_PATH__%s' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // Shell startup files may print banners; take what follows our marker.
    text.rsplit_once("__MWA_PATH__")
        .map(|(_, p)| p.trim().to_string())
}
