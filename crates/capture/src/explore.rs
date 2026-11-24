//! One browser page driven a command at a time (D-057): a model looks at each screen and picks
//! the next step, the way a person finds their way through a wizard. The sidecar keeps the page
//! open between commands; writes stay blocked as in every capture.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::detail::Sidecar;
use crate::shoot::{Blocked, Job, ShootError, Step};

/// What the screen shows after a command.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Obs {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    /// Accessibility tree (open dialogs first).
    #[serde(default)]
    pub aria: String,
    /// Path and query of the page.
    #[serde(default)]
    pub url: String,
    /// Screenshot of the viewport, when one was asked for.
    #[serde(default)]
    pub look: Option<PathBuf>,
    /// Write requests the screen tried and the sidecar stopped.
    #[serde(default)]
    pub blocked: Vec<Blocked>,
    /// The picture, after `shot`.
    #[serde(default)]
    pub out: Option<PathBuf>,
    #[serde(default, rename = "regionMissed")]
    pub region_missed: bool,
}

pub struct Session {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
}

impl Session {
    pub async fn start(sc: Sidecar<'_>, job: &Job) -> Result<Self, ShootError> {
        let mut job = job.clone();
        job.explore = true;
        job.shots.clear();
        let mut child = Command::new(sc.node)
            .arg(sc.script)
            .current_dir(sc.script.parent().unwrap_or(Path::new(".")))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ShootError::Spawn(format!("{} {}", sc.node, sc.script.display()), e))?;
        let stdin = child.stdin.take().expect("piped");
        let lines = BufReader::new(child.stdout.take().expect("piped")).lines();
        let mut s = Self {
            child,
            stdin,
            lines,
        };
        s.send(&serde_json::to_value(&job).expect("job serializes"))
            .await?;
        Ok(s)
    }

    async fn send(&mut self, v: &Value) -> Result<(), ShootError> {
        let mut line = v.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn ask(&mut self, cmd: Value) -> Result<Obs, ShootError> {
        self.send(&cmd).await?;
        while let Some(line) = self.lines.next_line().await? {
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            match v["type"].as_str() {
                Some("obs") => {
                    return serde_json::from_value(v)
                        .map_err(|e| ShootError::Fatal(format!("탐색 응답: {e}")));
                }
                Some("fatal") => {
                    return Err(ShootError::Fatal(
                        v["error"].as_str().unwrap_or("촬영기 오류").to_string(),
                    ));
                }
                _ => {}
            }
        }
        Err(ShootError::Fatal("촬영기가 응답 없이 끝났습니다".into()))
    }

    /// Opens `path` on the product; `look` gets a screenshot of the viewport.
    pub async fn open(&mut self, path: &str, look: Option<&Path>) -> Result<Obs, ShootError> {
        self.ask(json!({"cmd": "open", "path": path, "look": look}))
            .await
    }

    pub async fn step(&mut self, step: &Step, look: Option<&Path>) -> Result<Obs, ShootError> {
        self.ask(json!({"cmd": "step", "step": step, "look": look}))
            .await
    }

    /// Cuts the picture as a normal capture does; the last command of a scene.
    pub async fn shot(
        &mut self,
        out: &Path,
        capture: &str,
        region: &[String],
        ratio: Option<f64>,
    ) -> Result<Obs, ShootError> {
        self.ask(json!({"cmd": "shot", "shot": {
            "out": out, "capture": capture, "region": region, "ratio": ratio
        }}))
        .await
    }

    pub async fn end(mut self) {
        let _ = self.send(&json!({"cmd": "end"})).await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), self.child.wait()).await;
    }
}
