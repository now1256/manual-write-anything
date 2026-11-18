//! Fake executables and a one-shot HTTP server for backend tests.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::mpsc;

pub struct FakeCli {
    _dir: tempfile::TempDir,
    script: PathBuf,
    args_file: PathBuf,
    stdin_file: PathBuf,
}

impl FakeCli {
    pub fn command(&self) -> String {
        self.script.to_string_lossy().into_owned()
    }
    /// Arguments of the last invocation, one per element (empty strings preserved).
    pub fn args(&self) -> Vec<String> {
        let raw = std::fs::read(&self.args_file).unwrap();
        let mut v: Vec<String> = raw
            .split(|b| *b == 0)
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        v.pop(); // trailing NUL
        v
    }
    pub fn stdin(&self) -> String {
        std::fs::read_to_string(&self.stdin_file).unwrap()
    }
}

/// Script that records args and stdin outside its cwd, prints `stdout`, writes `stderr`, exits `code`.
pub fn fake_cli(stdout: &str, code: i32, stderr: &str) -> FakeCli {
    fake_cli_script(&format!(
        "cat <<'MWA_EOF'\n{stdout}\nMWA_EOF\necho '{stderr}' >&2\nexit {code}"
    ))
}

pub fn fake_cli_script(body: &str) -> FakeCli {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake-cli");
    let args_file = dir.path().join("args");
    let stdin_file = dir.path().join("stdin");
    let text = format!(
        "#!/bin/sh\nprintf '%s\\0' \"$@\" > '{}'\ncat > '{}'\n{body}\n",
        args_file.display(),
        stdin_file.display()
    );
    std::fs::write(&script, text).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    FakeCli {
        _dir: dir,
        script,
        args_file,
        stdin_file,
    }
}

pub struct Captured {
    pub head: String,
    pub body: String,
}

/// Serves exactly one HTTP request with the given status line, extra headers and body.
/// Returns the base URL and a receiver for the captured request.
pub fn one_shot_server(
    status: &str,
    headers: &str,
    body: &str,
) -> (String, mpsc::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n{headers}\r\n{body}",
        body.len()
    );
    std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let (head, body) = loop {
            let n = sock.read(&mut chunk).unwrap();
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..pos]).into_owned();
                let len: usize = head
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse().ok())?
                    })
                    .unwrap_or(0);
                while buf.len() < pos + 4 + len {
                    let n = sock.read(&mut chunk).unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                }
                break (
                    head,
                    String::from_utf8_lossy(&buf[pos + 4..pos + 4 + len]).into_owned(),
                );
            }
        };
        sock.write_all(response.as_bytes()).unwrap();
        tx.send(Captured { head, body }).unwrap();
    });
    (url, rx)
}
