//! Model backends (FB): subscription CLIs run as subprocesses, plus HTTP APIs.
//! Each call is single-shot and stateless; orchestration lives in `mwa-core`.

mod anthropic;
mod claude_code;
mod openai;
#[cfg(test)]
mod testutil;

use std::path::Path;

use async_trait::async_trait;
use base64::Engine as _;
use mwa_protocol::{BackendCfg, Request, Response};
use serde_json::Value;

pub use anthropic::Anthropic;
pub use claude_code::ClaudeCode;
pub use openai::OpenAiCompat;

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("실행 파일을 찾을 수 없습니다: {0}")]
    NotInstalled(String),
    /// `resets_at` is a Unix timestamp in seconds when known.
    #[error("사용량 한도에 걸렸습니다 (재설정 시각: {resets_at:?})")]
    RateLimited { resets_at: Option<u64> },
    #[error("{0}초 안에 끝나지 않았습니다")]
    Timeout(u64),
    #[error("출력이 요구 형식과 맞지 않습니다: {0}")]
    InvalidOutput(String),
    #[error("작업 폴더에 파일이 생겼습니다: {0:?}")]
    SandboxViolation(Vec<String>),
    #[error("{0}")]
    Failed(String),
}

/// Result of a cheap installation check (no model call).
#[derive(Debug, Clone)]
pub struct Probe {
    pub available: bool,
    pub version: Option<String>,
    pub detail: String,
}

/// Receives text as it is generated.
pub type DeltaFn<'a> = &'a (dyn Fn(&str) + Send + Sync);

#[async_trait]
pub trait Backend: Send + Sync {
    fn kind(&self) -> &'static str;
    async fn complete(&self, req: &Request) -> Result<Response, BackendError>;
    /// Like `complete`, reporting text as it arrives. Backends without streaming
    /// deliver the whole text once at the end.
    async fn complete_streaming(
        &self,
        req: &Request,
        on_delta: DeltaFn<'_>,
    ) -> Result<Response, BackendError> {
        let resp = self.complete(req).await?;
        on_delta(&resp.text);
        Ok(resp)
    }
    async fn probe(&self) -> Probe;
}

pub fn build(cfg: &BackendCfg) -> Box<dyn Backend> {
    match cfg {
        BackendCfg::ClaudeCode {
            command,
            timeout_secs,
            ..
        } => Box::new(ClaudeCode::new(command.clone(), *timeout_secs)),
        BackendCfg::Anthropic {
            base_url,
            api_key_env,
            max_tokens,
            timeout_secs,
            ..
        } => Box::new(Anthropic::new(
            base_url.clone(),
            api_key_env.clone(),
            *max_tokens,
            *timeout_secs,
        )),
        BackendCfg::OpenAiCompat {
            base_url,
            api_key_env,
            timeout_secs,
            ..
        } => Box::new(OpenAiCompat::new(
            "api:openai-compat",
            base_url.clone(),
            api_key_env.clone(),
            *timeout_secs,
        )),
        BackendCfg::Ollama {
            base_url,
            timeout_secs,
            ..
        } => Box::new(OpenAiCompat::new(
            "ollama",
            base_url.clone(),
            None,
            *timeout_secs,
        )),
    }
}

pub(crate) fn validate_json(schema: &Value, value: &Value) -> Result<(), BackendError> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|e| BackendError::Failed(format!("스키마 자체가 잘못되었습니다: {e}")))?;
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|e| e.to_string())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(BackendError::InvalidOutput(errors.join("; ")))
    }
}

pub(crate) struct Image {
    pub media_type: &'static str,
    pub base64: String,
}

pub(crate) fn load_image(path: &Path) -> Result<Image, BackendError> {
    let bytes = std::fs::read(path).map_err(|e| {
        BackendError::Failed(format!("이미지를 읽을 수 없습니다 {}: {e}", path.display()))
    })?;
    let media_type = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        return Err(BackendError::Failed(format!(
            "지원하지 않는 이미지 형식입니다: {}",
            path.display()
        )));
    };
    Ok(Image {
        media_type,
        base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

/// Keeps error messages readable when a backend dumps a large body.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_validation_reports_errors() {
        let schema =
            json!({"type":"object","properties":{"a":{"type":"integer"}},"required":["a"]});
        assert!(validate_json(&schema, &json!({"a": 1})).is_ok());
        assert!(matches!(
            validate_json(&schema, &json!({"a": "x"})),
            Err(BackendError::InvalidOutput(_))
        ));
    }

    #[test]
    fn image_type_detected_from_magic_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("a.bin");
        std::fs::write(&png, b"\x89PNG\r\n\x1a\nrest").unwrap();
        assert_eq!(load_image(&png).unwrap().media_type, "image/png");
        let txt = dir.path().join("b.png");
        std::fs::write(&txt, b"not an image").unwrap();
        assert!(load_image(&txt).is_err());
    }

    #[test]
    fn truncate_counts_chars_not_bytes() {
        assert_eq!(truncate("가나다라", 2), "가나…");
        assert_eq!(truncate("ab", 5), "ab");
    }
}
