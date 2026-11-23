//! Code on another machine (D-054): reached over SSH, through jump hosts, and into a Docker
//! container when the code lives there. Only fixed read-only commands run remotely (`tar -c`,
//! `git rev-parse`, `docker ps`, `ss -ltn`); the copy is kept here as a git repository so the
//! rest of mwa sees commits and changed files as with any other source.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

use crate::SourceError;

/// Where the code is: `target` (`user@host`), hops on the way, an optional container, the folder.
#[derive(Debug, Clone)]
pub struct Remote<'a> {
    pub target: &'a str,
    pub jump: &'a [String],
    pub docker: Option<&'a str>,
    pub path: &'a str,
    /// Passwords by hop (`user@host`, as ssh asks for them), only where no key works; each is
    /// tried once.
    pub passwords: &'a [(String, String)],
    /// mwa's own host keys (the user's `known_hosts` is left alone).
    pub known_hosts: &'a Path,
}

/// Answers ssh's password prompt from `MWA_SSH_PASSWORDS` ("user@host<TAB>password" lines):
/// the line whose user@host is in the prompt, else the first.
const ASKPASS: &str = r#"#!/bin/sh
tab=$(printf '	')
match=$(printf '%s' "$MWA_SSH_PASSWORDS" | while IFS="$tab" read -r who pw; do
  case "$1" in *"$who'"*|*"$who:"*) printf '%s' "$pw"; break;; esac
done)
[ -n "$match" ] || match=$(printf '%s' "$MWA_SSH_PASSWORDS" | head -n 1 | cut -f2-)
printf '%s
' "$match"
"#;

/// `user@host` of a hop as ssh's password prompt names it (no port).
pub fn who(hop: &str) -> String {
    let (user, rest) = hop.rsplit_once('@').unwrap_or(("", hop));
    let host = rest
        .rsplit_once(':')
        .filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit()))
        .map_or(rest, |(h, _)| h);
    if user.is_empty() {
        host.to_string()
    } else {
        format!("{user}@{host}")
    }
}

/// Folders never copied: dependencies and build output.
const SKIP: &[&str] = &[
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
];

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

impl Remote<'_> {
    /// A command for the host, run inside the container when there is one.
    fn inner(&self, cmd: &str) -> String {
        match self.docker {
            Some(c) => format!("docker exec {} sh -c {}", quote(c), quote(cmd)),
            None => cmd.to_string(),
        }
    }

    fn ssh(&self, remote_cmd: &str) -> Result<Command, SourceError> {
        // Tests put a stand-in here (it runs the command locally).
        let program = std::env::var("MWA_SSH").unwrap_or_else(|_| "ssh".into());
        let mut c = Command::new(program);
        c.args([
            "-F",
            &self.config()?.display().to_string(),
            "-o",
            if self.passwords.is_empty() {
                "BatchMode=yes"
            } else {
                "BatchMode=no"
            },
        ]);
        if !self.jump.is_empty() {
            c.args(["-J", &self.jump.join(",")]);
        }
        c.arg(destination(self.target)).arg(remote_cmd);
        if !self.passwords.is_empty() {
            // OpenSSH asks this program, with a prompt like "root@host's password:"; it answers
            // with that hop's password. Never typed by hand, never on the command line.
            let script = self.known_hosts.with_file_name("askpass.sh");
            std::fs::write(&script, ASKPASS)
                .map_err(|e| SourceError::GitUnavailable(e.to_string()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700));
            }
            let list: String = self
                .passwords
                .iter()
                .map(|(who, pw)| format!("{who}\t{pw}\n"))
                .collect();
            c.env("SSH_ASKPASS", &script)
                .env("SSH_ASKPASS_REQUIRE", "force")
                .env("DISPLAY", ":0")
                .env("MWA_SSH_PASSWORDS", list);
        }
        c.stdin(Stdio::null());
        Ok(c)
    }

    /// mwa's SSH settings, for every hop (jump hosts too): its own host keys, its own key
    /// besides the user's, one password try. The user's `~/.ssh/config` still applies after it
    /// (host aliases, ports, users).
    fn config(&self) -> Result<PathBuf, SourceError> {
        let dir = self.known_hosts.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| SourceError::GitUnavailable(e.to_string()))?;
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let mut text = format!(
            "Host *\n  ConnectTimeout 15\n  StrictHostKeyChecking accept-new\n  UserKnownHostsFile {}\n  NumberOfPasswordPrompts 1\n",
            self.known_hosts.display()
        );
        let keys = ["id_ed25519", "id_ecdsa", "id_rsa"]
            .iter()
            .map(|k| home.join(".ssh").join(k))
            .chain([dir.join("id_ed25519")]);
        for k in keys.filter(|k| k.is_file()) {
            text.push_str(&format!("  IdentityFile {}\n", k.display()));
        }
        let user_cfg = home.join(".ssh/config");
        if user_cfg.is_file() {
            text.push_str(&format!("\nInclude {}\n", user_cfg.display()));
        }
        let path = dir.join("config");
        std::fs::write(&path, text).map_err(|e| SourceError::GitUnavailable(e.to_string()))?;
        Ok(path)
    }

    fn run(&self, remote_cmd: &str) -> Result<String, SourceError> {
        let out = self
            .ssh(remote_cmd)?
            .output()
            .map_err(|e| SourceError::GitUnavailable(format!("ssh: {e}")))?;
        if !out.status.success() {
            return Err(ssh_error(out.status.code(), &out.stderr));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// The host part of `user@host`.
    /// The host a browser would use: the target's, or the last jump host's when the target is
    /// "localhost" seen from that jump host.
    fn host(&self) -> String {
        let bare = |t: &str| {
            let h = t.rsplit('@').next().unwrap_or(t);
            h.rsplit_once(':')
                .filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit()))
                .map_or(h, |(h, _)| h)
                .to_string()
        };
        let h = bare(self.target);
        match self.jump.last() {
            Some(j) if h == "localhost" || h.starts_with("127.") => bare(j),
            _ => h,
        }
    }
}

/// `user@host:port` as ssh takes it (`ssh://user@host:port`); anything else as written (a host
/// alias from `~/.ssh/config` included).
fn destination(target: &str) -> String {
    match target.rsplit_once(':') {
        Some((head, port))
            if !port.is_empty()
                && port.chars().all(|c| c.is_ascii_digit())
                && !head.contains(':') =>
        {
            format!("ssh://{head}:{port}")
        }
        _ => target.to_string(),
    }
}

fn ssh_error(code: Option<i32>, stderr: &[u8]) -> SourceError {
    let msg = String::from_utf8_lossy(stderr).trim().to_string();
    let hint = if code == Some(255) {
        if msg.contains("Permission denied") {
            " — 접속이 거절됐습니다. SSH 키를 등록하거나 비밀번호를 저장하세요 (다시 시도하지 않습니다)"
        } else if msg.contains("Host key verification failed")
            || msg.contains("REMOTE HOST IDENTIFICATION")
        {
            " — 서버의 호스트 키가 예전과 다릅니다. 서버가 바뀐 게 맞으면 mwa 데이터 폴더의 ssh/known_hosts에서 그 줄을 지우세요"
        } else {
            " — 서버에 접속하지 못했습니다 (주소·경유 서버·네트워크 확인)"
        }
    } else {
        ""
    };
    SourceError::Git {
        args: "ssh".into(),
        stderr: format!("{msg}{hint}"),
    }
}

fn local_git(dir: &Path, args: &[&str]) -> Result<String, SourceError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| SourceError::GitUnavailable(e.to_string()))?;
    if !out.status.success() {
        return Err(SourceError::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Copies the remote folder into `dest` and records it as a commit there. Returns a one-line
/// summary.
pub fn sync(r: &Remote, dest: &Path) -> Result<String, SourceError> {
    let excludes: String = SKIP
        .iter()
        .map(|d| format!(" --exclude={}", quote(d)))
        .collect();
    let tar = r.inner(&format!("tar -C {}{excludes} -cf - .", quote(r.path)));
    let incoming = dest.with_extension("incoming");
    let _ = std::fs::remove_dir_all(&incoming);
    std::fs::create_dir_all(&incoming).map_err(|_| SourceError::PathMissing(incoming.clone()))?;

    let mut ssh = r
        .ssh(&tar)?
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| SourceError::GitUnavailable(format!("ssh: {e}")))?;
    let stdout = ssh.stdout.take().expect("piped");
    let untar = Command::new("tar")
        .arg("-xf")
        .arg("-")
        .arg("-C")
        .arg(&incoming)
        .stdin(stdout)
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| SourceError::GitUnavailable(format!("tar: {e}")))?;
    let done = ssh
        .wait_with_output()
        .map_err(|e| SourceError::GitUnavailable(format!("ssh: {e}")))?;
    if !done.status.success() {
        let _ = std::fs::remove_dir_all(&incoming);
        return Err(ssh_error(done.status.code(), &done.stderr));
    }
    if !untar.status.success() {
        let _ = std::fs::remove_dir_all(&incoming);
        return Err(SourceError::Git {
            args: "tar".into(),
            stderr: String::from_utf8_lossy(&untar.stderr).trim().to_string(),
        });
    }
    let remote_sha = r
        .run(&r.inner(&format!(
            "git -C {} rev-parse --short HEAD 2>/dev/null || true",
            quote(r.path)
        )))
        .unwrap_or_default()
        .trim()
        .to_string();

    // The copy replaces what was there; the local history stays.
    std::fs::create_dir_all(dest).map_err(|_| SourceError::PathMissing(dest.to_path_buf()))?;
    for e in std::fs::read_dir(dest).into_iter().flatten().flatten() {
        if e.file_name() != ".git" {
            let p = e.path();
            let _ = if p.is_dir() {
                std::fs::remove_dir_all(&p)
            } else {
                std::fs::remove_file(&p)
            };
        }
    }
    for e in std::fs::read_dir(&incoming).into_iter().flatten().flatten() {
        std::fs::rename(e.path(), dest.join(e.file_name()))
            .map_err(|err| SourceError::GitUnavailable(err.to_string()))?;
    }
    let _ = std::fs::remove_dir_all(&incoming);

    if !dest.join(".git").exists() {
        local_git(dest, &["init", "--quiet"])?;
        local_git(dest, &["config", "user.name", "mwa"])?;
        local_git(dest, &["config", "user.email", "mwa@localhost"])?;
    }
    local_git(dest, &["add", "--all"])?;
    let changed = local_git(dest, &["status", "--porcelain"])?;
    if changed.is_empty() {
        let sha = local_git(dest, &["rev-parse", "--short", "HEAD"])?;
        return Ok(format!("최신입니다 ({sha})"));
    }
    let n = changed.lines().count();
    let origin = format!(
        "{}{}:{}{}",
        r.target,
        r.docker.map(|c| format!(" [{c}]")).unwrap_or_default(),
        r.path,
        if remote_sha.is_empty() {
            String::new()
        } else {
            format!(" @ {remote_sha}")
        }
    );
    local_git(
        dest,
        &[
            "commit",
            "--quiet",
            "-m",
            &format!("원격에서 받음: {origin}"),
        ],
    )?;
    let sha = local_git(dest, &["rev-parse", "--short", "HEAD"])?;
    Ok(format!("받았습니다 ({origin}, 파일 {n}개 바뀜 → {sha})"))
}

/// What a person sees when setting up the source: containers, listening ports, whether the
/// folder is there, and addresses the product may be served at.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Probe {
    pub containers: Vec<Container>,
    pub ports: Vec<u16>,
    /// The folder exists (inside the container when one is set).
    pub path_ok: bool,
    /// Top entries of the folder, to see it is the code.
    pub entries: Vec<String>,
    /// Addresses to try as the site to shoot: (url, what serves it).
    pub urls: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Container {
    pub name: String,
    pub image: String,
    pub ports: String,
}

/// Looks around the host (read only).
pub fn probe(r: &Remote) -> Result<Probe, SourceError> {
    let host_part = "docker ps --format '{{.Names}}\t{{.Image}}\t{{.Ports}}' 2>/dev/null; echo '---mwa---'; (ss -ltnH 2>/dev/null || netstat -ltn 2>/dev/null)";
    let out = r.run(host_part)?;
    let (docker, listen) = out.split_once("---mwa---").unwrap_or((&out, ""));
    let mut p = Probe::default();
    for line in docker.lines().filter(|l| !l.trim().is_empty()) {
        let mut f = line.split('\t');
        p.containers.push(Container {
            name: f.next().unwrap_or("").to_string(),
            image: f.next().unwrap_or("").to_string(),
            ports: f.next().unwrap_or("").to_string(),
        });
    }
    for line in listen.lines() {
        // "LISTEN 0 4096 0.0.0.0:3001 0.0.0.0:*" — the local address is the 4th field.
        let Some(addr) = line
            .split_whitespace()
            .find(|w| w.contains(':') && !w.ends_with('*'))
        else {
            continue;
        };
        if addr.starts_with("127.") || addr.starts_with("[::1]") {
            continue;
        }
        if let Some(port) = addr.rsplit(':').next().and_then(|x| x.parse::<u16>().ok())
            && !p.ports.contains(&port)
        {
            p.ports.push(port);
        }
    }
    p.ports.sort_unstable();
    let host = r.host();
    for c in &p.containers {
        // "0.0.0.0:3001->3000/tcp, :::3001->3000/tcp"
        for part in c.ports.split(',') {
            // A port forwarded to the container's SSH is not a web page.
            if let Some((outside, inside)) = part.trim().split_once("->")
                && !inside.starts_with("22/")
                && let Some(port) = outside
                    .rsplit(':')
                    .next()
                    .and_then(|x| x.parse::<u16>().ok())
            {
                let url = format!("http://{host}:{port}");
                if !p.urls.iter().any(|(u, _)| *u == url) {
                    p.urls.push((url, format!("{} ({})", c.name, c.image)));
                }
            }
        }
    }
    for port in &p.ports {
        let url = format!("http://{host}:{port}");
        let ssh_forward = p
            .containers
            .iter()
            .any(|c| c.ports.contains(&format!(":{port}->22/")));
        if *port != 22 && !ssh_forward && !p.urls.iter().any(|(u, _)| *u == url) {
            p.urls.push((url, "열린 포트".into()));
        }
    }
    let listing = r
        .run(&r.inner(&format!(
            "test -d {0} && ls -A {0} | head -40",
            quote(r.path)
        )))
        .unwrap_or_default();
    p.entries = listing.lines().map(String::from).collect();
    p.path_ok = !p.entries.is_empty();
    Ok(p)
}

/// Whether the connection works: who and where we land, and (with a container) that it can be
/// entered.
pub fn check(r: &Remote) -> Result<String, SourceError> {
    let who = r.run("echo \"$(whoami)@$(hostname)\"")?.trim().to_string();
    if let Some(c) = r.docker {
        r.run(&r.inner("true")).map_err(|e| SourceError::Git {
            args: "docker".into(),
            stderr: format!(
                "서버에는 접속했지만 컨테이너 '{c}'에 들어가지 못했습니다 (이름, 또는 이 계정의 docker 권한 확인): {e}"
            ),
        })?;
    }
    Ok(who)
}

/// One folder level (read only): (name, is a folder), folders first.
pub fn list_dir(r: &Remote, path: &str) -> Result<Vec<(String, bool)>, SourceError> {
    let out = r.run(&r.inner(&format!("cd {} && ls -Ap", quote(path))))?;
    let mut v: Vec<(String, bool)> = out
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| match l.strip_suffix('/') {
            Some(d) => (d.to_string(), true),
            None => (l.to_string(), false),
        })
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(v)
}

/// Folders that look like the code: the container's working folder and the usual places,
/// with what marks them (`package.json`, Python).
pub fn guess(r: &Remote) -> Result<Vec<(String, String)>, SourceError> {
    let script = "for d in \"$(pwd)\" /app /usr/src/app /code /srv/app /srv /opt/app /workspace /var/www/app /home/*/app /home/*/*; do \
[ -d \"$d\" ] || continue; \
if [ -f \"$d/package.json\" ]; then echo \"$d|package.json\"; \
elif [ -f \"$d/pyproject.toml\" ] || [ -f \"$d/requirements.txt\" ]; then echo \"$d|Python\"; fi; done";
    let out = r.run(&r.inner(script))?;
    let mut v: Vec<(String, String)> = Vec::new();
    for l in out.lines() {
        if let Some((d, k)) = l.split_once('|')
            && !v.iter().any(|(x, _)| x == d)
        {
            v.push((d.to_string(), k.to_string()));
        }
    }
    Ok(v)
}

/// mwa's own key (made once in its data folder); returns the public part to put on servers.
pub fn ensure_key(data_dir: &Path) -> Result<String, SourceError> {
    let key = data_dir.join("ssh/id_ed25519");
    if !key.is_file() {
        std::fs::create_dir_all(key.parent().expect("has parent"))
            .map_err(|e| SourceError::GitUnavailable(e.to_string()))?;
        let host = Command::new("hostname")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let out = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C"])
            .arg(format!("mwa@{host}"))
            .arg("-f")
            .arg(&key)
            .output()
            .map_err(|e| SourceError::GitUnavailable(format!("ssh-keygen: {e}")))?;
        if !out.status.success() {
            return Err(SourceError::GitUnavailable(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
    }
    std::fs::read_to_string(key.with_extension("pub"))
        .map(|k| k.trim().to_string())
        .map_err(|e| SourceError::GitUnavailable(e.to_string()))
}

/// The askpass helper and host keys live in mwa's data folder.
pub fn known_hosts(data_dir: &Path) -> PathBuf {
    data_dir.join("ssh/known_hosts")
}

/// Writes a stand-in for `ssh` that runs the remote command here (tests only).
pub fn write_test_ssh(path: &Path) -> std::io::Result<()> {
    let mut f = std::fs::File::create(path)?;
    // The command is the last argument; everything before it is options and the target.
    f.write_all(b"#!/bin/sh\nfor last; do :; done\nexec sh -c \"$last\"\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_a_folder_and_records_changes() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("app");
        std::fs::create_dir_all(src.join("node_modules/x")).unwrap();
        std::fs::create_dir_all(src.join("src")).unwrap();
        std::fs::write(src.join("src/page.tsx"), "a").unwrap();
        std::fs::write(src.join("node_modules/x/big.js"), "skip").unwrap();
        let fake = dir.path().join("ssh");
        write_test_ssh(&fake).unwrap();
        // SAFETY: tests in this crate do not read MWA_SSH concurrently with other env changes.
        unsafe { std::env::set_var("MWA_SSH", &fake) };
        let kh = dir.path().join("kh/known_hosts");
        let path = src.display().to_string();
        let r = Remote {
            target: "me@host",
            jump: &["hop@bastion".to_string()],
            docker: None,
            path: &path,
            passwords: &[],
            known_hosts: &kh,
        };
        let dest = dir.path().join("copy");
        let first = sync(&r, &dest).unwrap();
        assert!(first.starts_with("받았습니다"), "{first}");
        assert!(dest.join("src/page.tsx").is_file());
        assert!(!dest.join("node_modules").exists());
        assert!(sync(&r, &dest).unwrap().starts_with("최신입니다"));
        std::fs::write(src.join("src/page.tsx"), "b").unwrap();
        assert!(sync(&r, &dest).unwrap().contains("파일 1개 바뀜"));
        let snap = crate::snapshot_local("web", &dest).unwrap();
        assert_eq!(snap.dirty_entries, 0);

        let p = probe(&r).unwrap();
        assert!(p.path_ok && p.entries.contains(&"src".to_string()));
        assert!(check(&r).unwrap().contains('@'));
        let ls = list_dir(&r, &path).unwrap();
        assert_eq!(ls.first(), Some(&("node_modules".to_string(), true)));
        let config = std::fs::read_to_string(kh.with_file_name("config")).unwrap();
        assert!(
            config.contains("UserKnownHostsFile") && config.contains("NumberOfPasswordPrompts 1")
        );
    }

    #[test]
    fn askpass_answers_each_hop() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("askpass.sh");
        std::fs::write(&script, ASKPASS).unwrap();
        let ask = |prompt: &str| {
            let out = Command::new("sh")
                .arg(&script)
                .arg(prompt)
                .env(
                    "MWA_SSH_PASSWORDS",
                    "root@203.0.113.7\tfirst\nroot@localhost\tsec ret\n",
                )
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim_end().to_string()
        };
        assert_eq!(ask("root@203.0.113.7's password: "), "first");
        assert_eq!(ask("root@localhost's password: "), "sec ret");
        assert_eq!(ask("Password: "), "first");
        assert_eq!(who("root@203.0.113.7:2222"), "root@203.0.113.7");
        assert_eq!(who("acme-inner"), "acme-inner");
    }

    #[test]
    fn docker_commands_are_quoted() {
        let kh = PathBuf::from("/tmp/kh");
        let r = Remote {
            target: "u@h",
            jump: &[],
            docker: Some("api"),
            path: "/app",
            passwords: &[],
            known_hosts: &kh,
        };
        assert_eq!(
            destination("root@localhost:8022"),
            "ssh://root@localhost:8022"
        );
        assert_eq!(destination("acme-inner"), "acme-inner");
        assert_eq!(destination("u@h"), "u@h");
        let hops = ["root@203.0.113.7:2222".to_string()];
        let through = Remote {
            target: "root@localhost:8022",
            jump: &hops,
            ..r.clone()
        };
        assert_eq!(through.host(), "203.0.113.7");
        assert_eq!(r.host(), "h");
        assert_eq!(
            r.inner("ls -A '/app'"),
            "docker exec 'api' sh -c 'ls -A '\\''/app'\\'''"
        );
    }
}
