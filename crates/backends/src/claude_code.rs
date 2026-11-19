//! `cli:claude-code`: runs the installed `claude` binary with the user's own login.
//! Flags are the ones verified in docs/cli-flags.md (Claude Code 2.1.326).

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use mwa_protocol::{Quota, Request, Response, Usage};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::{Backend, BackendError, DeltaFn, Probe, load_image, truncate, validate_json};

pub struct ClaudeCode {
    command: String,
    timeout: Duration,
}

impl ClaudeCode {
    pub fn new(command: String, timeout_secs: u64) -> Self {
        // Resolve once so a minimal PATH (ssh command, desktop app) still finds the CLI;
        // if it is nowhere, keep the name and let the spawn error explain.
        let command = crate::find_command(&command)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(command);
        Self {
            command,
            timeout: Duration::from_secs(timeout_secs),
        }
    }

    async fn run(
        &self,
        req: &Request,
        on_delta: Option<DeltaFn<'_>>,
    ) -> Result<Response, BackendError> {
        let input = Self::stdin_message(req)?;
        // Empty scratch directory: no project files or CLAUDE.md for the CLI to pick up,
        // and an easy check that nothing was written.
        let workdir = tempfile::tempdir()
            .map_err(|e| BackendError::Failed(format!("임시 폴더를 만들 수 없습니다: {e}")))?;

        let mut args = Self::args(req);
        if on_delta.is_some() {
            args.push("--include-partial-messages".into());
        }
        let mut child = Command::new(&self.command)
            .args(args)
            .current_dir(workdir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => BackendError::NotInstalled(self.command.clone()),
                _ => BackendError::Failed(format!("{} 실행 실패: {e}", self.command)),
            })?;

        let mut stdin = child.stdin.take().expect("stdin is piped");
        stdin
            .write_all(input.as_bytes())
            .await
            .map_err(|e| BackendError::Failed(format!("stdin 쓰기 실패: {e}")))?;
        drop(stdin);

        let stdout_pipe = child.stdout.take().expect("stdout is piped");
        let mut stderr_pipe = child.stderr.take().expect("stderr is piped");
        // Drain stderr concurrently so a chatty CLI cannot block on a full pipe.
        let stderr_task = tokio::spawn(async move {
            let mut buf = String::new();
            let _ = stderr_pipe.read_to_string(&mut buf).await;
            buf
        });

        let read_all = async {
            let mut stdout = String::new();
            let mut lines = BufReader::new(stdout_pipe).lines();
            while let Some(line) = lines
                .next_line()
                .await
                .map_err(|e| BackendError::Failed(format!("출력 읽기 실패: {e}")))?
            {
                if let Some(cb) = on_delta
                    && let Some(text) = text_delta(&line)
                {
                    cb(&text);
                }
                stdout.push_str(&line);
                stdout.push('\n');
            }
            let status = child
                .wait()
                .await
                .map_err(|e| BackendError::Failed(format!("{} 대기 실패: {e}", self.command)))?;
            Ok::<_, BackendError>((stdout, status))
        };
        let (stdout, status) = tokio::time::timeout(self.timeout, read_all)
            .await
            .map_err(|_| BackendError::Timeout(self.timeout.as_secs()))??;
        let stderr = stderr_task.await.unwrap_or_default();

        let written: Vec<String> = std::fs::read_dir(workdir.path())
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        if !written.is_empty() {
            return Err(BackendError::SandboxViolation(written));
        }

        parse_stream(&stdout, req.schema.as_ref()).map_err(|e| match e {
            BackendError::Failed(msg) if msg.is_empty() => BackendError::Failed(format!(
                "결과 이벤트가 없습니다 (종료 코드 {:?}): {}",
                status.code(),
                truncate(stderr.trim(), 500)
            )),
            other => other,
        })
    }

    fn args(req: &Request) -> Vec<String> {
        let mut args: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--tools",
            "",
            "--no-session-persistence",
            "--strict-mcp-config",
            "--setting-sources",
            "",
            "--disable-slash-commands",
            "--permission-prompts",
            "none",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        args.extend(["--model".into(), req.model.clone()]);
        args.extend(["--system-prompt".into(), req.system.clone()]);
        if let Some(schema) = &req.schema {
            args.extend(["--json-schema".into(), schema.to_string()]);
        }
        args
    }

    fn stdin_message(req: &Request) -> Result<String, BackendError> {
        let mut content = Vec::new();
        for path in &req.images {
            let img = load_image(path)?;
            content.push(json!({
                "type": "image",
                "source": {"type": "base64", "media_type": img.media_type, "data": img.base64}
            }));
        }
        content.push(json!({"type": "text", "text": req.prompt}));
        let msg = json!({"type": "user", "message": {"role": "user", "content": content}});
        Ok(format!("{msg}\n"))
    }
}

#[async_trait]
impl Backend for ClaudeCode {
    fn kind(&self) -> &'static str {
        "cli:claude-code"
    }

    async fn complete(&self, req: &Request) -> Result<Response, BackendError> {
        self.run(req, None).await
    }

    async fn complete_streaming(
        &self,
        req: &Request,
        on_delta: DeltaFn<'_>,
    ) -> Result<Response, BackendError> {
        self.run(req, Some(on_delta)).await
    }

    async fn probe(&self) -> Probe {
        let out = Command::new(&self.command)
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => Probe {
                available: true,
                version: Some(String::from_utf8_lossy(&o.stdout).trim().to_string()),
                detail: String::new(),
            },
            Ok(o) => Probe {
                available: false,
                version: None,
                detail: truncate(String::from_utf8_lossy(&o.stderr).trim(), 200),
            },
            Err(_) => Probe {
                available: false,
                version: None,
                detail: "미설치".into(),
            },
        }
    }
}

/// Text of a `stream_event` / `content_block_delta` / `text_delta` line.
fn text_delta(line: &str) -> Option<String> {
    if !line.contains("text_delta") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let e = &v["event"];
    (v["type"] == "stream_event"
        && e["type"] == "content_block_delta"
        && e["delta"]["type"] == "text_delta")
        .then(|| e["delta"]["text"].as_str().map(String::from))?
}

/// Parses stream-json stdout. An empty `Failed` message means "no result event".
fn parse_stream(stdout: &str, schema: Option<&Value>) -> Result<Response, BackendError> {
    let mut result: Option<Value> = None;
    let mut init_model: Option<String> = None;
    let mut limit_rejected = false;
    let mut resets_at: Option<u64> = None;
    let mut quota: Option<Quota> = None;

    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(ev) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match ev["type"].as_str() {
            Some("system") if ev["subtype"] == "init" => {
                init_model = ev["model"].as_str().map(String::from);
            }
            Some("rate_limit_event") => {
                let info = &ev["rate_limit_info"];
                resets_at = info["resetsAt"].as_u64().or(resets_at);
                let status = info["status"].as_str().unwrap_or("");
                if !status.starts_with("allowed") {
                    limit_rejected = true;
                }
                let window = info["rateLimitType"].as_str().unwrap_or("").to_string();
                quota = Some(Quota {
                    utilization: info["unifiedWindows"][&window]["utilization"].as_f64(),
                    window,
                    status: status.to_string(),
                    resets_at: info["resetsAt"].as_u64(),
                });
            }
            Some("result") => result = Some(ev),
            _ => {}
        }
    }

    let Some(res) = result else {
        if limit_rejected {
            return Err(BackendError::RateLimited { resets_at });
        }
        return Err(BackendError::Failed(String::new()));
    };

    // `subtype` can say "success" on API errors; `is_error` is the reliable flag.
    if res["is_error"].as_bool().unwrap_or(true) {
        if limit_rejected || res["api_error_status"].as_u64() == Some(429) {
            return Err(BackendError::RateLimited { resets_at });
        }
        return Err(BackendError::Failed(format!(
            "{} (status {:?}, {})",
            truncate(res["result"].as_str().unwrap_or("알 수 없는 오류"), 300),
            res["api_error_status"].as_u64(),
            res["terminal_reason"].as_str().unwrap_or("-"),
        )));
    }

    let text = res["result"].as_str().unwrap_or_default().to_string();
    let json = match schema {
        Some(schema) => {
            let value = res
                .get("structured_output")
                .filter(|v| !v.is_null())
                .cloned()
                .ok_or_else(|| {
                    BackendError::InvalidOutput("structured_output이 없습니다".into())
                })?;
            validate_json(schema, &value)?;
            Some(value)
        }
        None => None,
    };

    let u = &res["usage"];
    let input_tokens = [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ]
    .iter()
    .map(|k| u[*k].as_u64())
    .try_fold(0u64, |acc, v| v.map(|v| acc + v));
    let model = res["modelUsage"]
        .as_object()
        .and_then(|m| m.keys().next().cloned())
        .or(init_model);

    Ok(Response {
        text,
        json,
        model,
        usage: Usage {
            input_tokens,
            output_tokens: u["output_tokens"].as_u64(),
            cost_usd: res["total_cost_usd"].as_f64(),
        },
        raw: stdout.to_string(),
        quota,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fake_cli;
    use serde_json::json;

    fn req(schema: Option<Value>) -> Request {
        Request {
            system: "sys".into(),
            prompt: "1+1?".into(),
            images: vec![],
            schema,
            model: "haiku".into(),
        }
    }

    const INIT: &str =
        r#"{"type":"system","subtype":"init","model":"claude-haiku-5-5","apiKeySource":"none"}"#;
    const OK_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"{\"answer\":2}","structured_output":{"answer":2},"total_cost_usd":0.0002,"usage":{"input_tokens":2,"cache_creation_input_tokens":1135,"cache_read_input_tokens":0,"output_tokens":52},"modelUsage":{"claude-haiku-5-5":{}}}"#;

    fn schema() -> Value {
        json!({"type":"object","properties":{"answer":{"type":"integer"}},"required":["answer"]})
    }

    #[tokio::test]
    async fn success_with_schema_and_flags() {
        let fake = fake_cli(&format!("{INIT}\n{OK_RESULT}"), 0, "");
        let b = ClaudeCode::new(fake.command(), 10);
        let resp = b.complete(&req(Some(schema()))).await.unwrap();
        assert_eq!(resp.json, Some(json!({"answer": 2})));
        assert_eq!(resp.model.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(resp.usage.input_tokens, Some(1137));
        assert_eq!(resp.usage.output_tokens, Some(52));
        assert_eq!(resp.usage.cost_usd, Some(0.0002));

        let args = fake.args();
        for flag in [
            "-p",
            "--input-format",
            "--output-format",
            "--tools",
            "--no-session-persistence",
            "--setting-sources",
            "--json-schema",
            "--permission-prompts",
        ] {
            assert!(args.iter().any(|a| a == flag), "missing {flag}: {args:?}");
        }
        let tools = args.iter().position(|a| a == "--tools").unwrap();
        assert_eq!(args[tools + 1], "", "tools must be disabled");
        let stdin: Value = serde_json::from_str(fake.stdin().trim()).unwrap();
        assert_eq!(stdin["message"]["content"][0]["text"], "1+1?");
    }

    #[tokio::test]
    async fn streaming_reports_deltas_and_quota() {
        let out = format!(
            "{INIT}\n{}\n{}\n{}\n{OK_RESULT}",
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"안녕"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"하세요"}}}"#,
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":5,"rateLimitType":"seven_day","unifiedWindows":{"seven_day":{"utilization":0.02}}}}"#,
        );
        let fake = fake_cli(&out, 0, "");
        let got = std::sync::Mutex::new(String::new());
        let cb = |t: &str| got.lock().unwrap().push_str(t);
        let resp = ClaudeCode::new(fake.command(), 10)
            .complete_streaming(&req(None), &cb)
            .await
            .unwrap();
        assert_eq!(*got.lock().unwrap(), "안녕하세요");
        assert!(
            fake.args()
                .iter()
                .any(|a| a == "--include-partial-messages")
        );
        let q = resp.quota.unwrap();
        assert_eq!(
            (q.window.as_str(), q.utilization, q.resets_at),
            ("seven_day", Some(0.02), Some(5))
        );
    }

    #[tokio::test]
    async fn image_is_sent_as_base64_block() {
        let fake = fake_cli(&format!("{INIT}\n{OK_RESULT}"), 0, "");
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("x.png");
        std::fs::write(&img, b"\x89PNG\r\n\x1a\nabc").unwrap();
        let mut r = req(None);
        r.images.push(img);
        ClaudeCode::new(fake.command(), 10)
            .complete(&r)
            .await
            .unwrap();
        let stdin: Value = serde_json::from_str(fake.stdin().trim()).unwrap();
        let block = &stdin["message"]["content"][0];
        assert_eq!(block["type"], "image");
        assert_eq!(block["source"]["media_type"], "image/png");
    }

    #[tokio::test]
    async fn is_error_wins_over_success_subtype() {
        let out = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":404,"terminal_reason":"api_error","result":"There's an issue with the selected model"}"#;
        let fake = fake_cli(out, 1, "");
        let err = ClaudeCode::new(fake.command(), 10)
            .complete(&req(None))
            .await
            .unwrap_err();
        match err {
            BackendError::Failed(m) => {
                assert!(m.contains("404") && m.contains("selected model"), "{m}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn rate_limit_detected() {
        let out = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1795485600}}
{"type":"result","is_error":true,"result":"limit reached"}"#;
        let fake = fake_cli(out, 1, "");
        let err = ClaudeCode::new(fake.command(), 10)
            .complete(&req(None))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                BackendError::RateLimited {
                    resets_at: Some(1795485600)
                }
            ),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn allowed_rate_limit_event_is_not_an_error() {
        let out = format!(
            "{}\n{OK_RESULT}",
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1}}"#
        );
        let fake = fake_cli(&out, 0, "");
        assert!(
            ClaudeCode::new(fake.command(), 10)
                .complete(&req(None))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn missing_structured_output_is_invalid() {
        let out = r#"{"type":"result","is_error":false,"result":"two"}"#;
        let fake = fake_cli(out, 0, "");
        let err = ClaudeCode::new(fake.command(), 10)
            .complete(&req(Some(schema())))
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::InvalidOutput(_)), "{err:?}");
    }

    #[tokio::test]
    async fn structured_output_failing_schema_is_invalid() {
        let out = r#"{"type":"result","is_error":false,"result":"","structured_output":{"answer":"two"}}"#;
        let fake = fake_cli(out, 0, "");
        let err = ClaudeCode::new(fake.command(), 10)
            .complete(&req(Some(schema())))
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::InvalidOutput(_)), "{err:?}");
    }

    #[tokio::test]
    async fn no_result_event_reports_stderr() {
        let fake = fake_cli("garbage", 2, "boom on stderr");
        let err = ClaudeCode::new(fake.command(), 10)
            .complete(&req(None))
            .await
            .unwrap_err();
        match err {
            BackendError::Failed(m) => {
                assert!(m.contains("boom on stderr") && m.contains("2"), "{m}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn timeout_kills_child() {
        let fake = crate::testutil::fake_cli_script("sleep 5");
        let start = std::time::Instant::now();
        let err = ClaudeCode::new(fake.command(), 1)
            .complete(&req(None))
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::Timeout(1)), "{err:?}");
        assert!(start.elapsed() < Duration::from_secs(4));
    }

    #[tokio::test]
    async fn files_written_in_workdir_are_rejected() {
        let fake = crate::testutil::fake_cli_script(&format!(
            "touch leaked.txt\ncat <<'EOF'\n{OK_RESULT}\nEOF"
        ));
        let err = ClaudeCode::new(fake.command(), 10)
            .complete(&req(None))
            .await
            .unwrap_err();
        assert!(
            matches!(err, BackendError::SandboxViolation(ref f) if f == &["leaked.txt"]),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn missing_binary_is_not_installed() {
        let b = ClaudeCode::new("/nonexistent/claude-xyz".into(), 10);
        assert!(matches!(
            b.complete(&req(None)).await,
            Err(BackendError::NotInstalled(_))
        ));
        assert!(!b.probe().await.available);
    }

    /// Calls the real CLI with the user's login. Run with `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn real_claude_cli() {
        let b = ClaudeCode::new("claude".into(), 120);
        let resp = b.complete(&req(Some(schema()))).await.unwrap();
        assert_eq!(resp.json, Some(json!({"answer": 2})));
    }
}
