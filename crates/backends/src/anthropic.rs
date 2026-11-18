//! `api:anthropic`: Messages API with an API key read from an environment variable.
//! Structured output uses a forced tool call whose input schema is the requested schema.

use std::time::Duration;

use async_trait::async_trait;
use mwa_protocol::{Request, Response, Usage};
use serde_json::{Value, json};

use crate::{Backend, BackendError, Probe, load_image, now_secs, truncate, validate_json};

const TOOL_NAME: &str = "respond";

pub struct Anthropic {
    base_url: String,
    api_key_env: String,
    max_tokens: u32,
    timeout_secs: u64,
    client: reqwest::Client,
}

impl Anthropic {
    pub fn new(base_url: String, api_key_env: String, max_tokens: u32, timeout_secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .expect("reqwest client");
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key_env,
            max_tokens,
            timeout_secs,
            client,
        }
    }

    fn body(&self, req: &Request) -> Result<Value, BackendError> {
        let mut content = Vec::new();
        for path in &req.images {
            let img = load_image(path)?;
            content.push(json!({
                "type": "image",
                "source": {"type": "base64", "media_type": img.media_type, "data": img.base64}
            }));
        }
        content.push(json!({"type": "text", "text": req.prompt}));
        let mut body = json!({
            "model": req.model,
            "max_tokens": self.max_tokens,
            "system": req.system,
            "messages": [{"role": "user", "content": content}],
        });
        if let Some(schema) = &req.schema {
            body["tools"] = json!([{
                "name": TOOL_NAME,
                "description": "Return the answer in the required structure.",
                "input_schema": schema,
            }]);
            body["tool_choice"] = json!({"type": "tool", "name": TOOL_NAME});
        }
        Ok(body)
    }
}

#[async_trait]
impl Backend for Anthropic {
    fn kind(&self) -> &'static str {
        "api:anthropic"
    }

    async fn complete(&self, req: &Request) -> Result<Response, BackendError> {
        let key = std::env::var(&self.api_key_env).map_err(|_| {
            BackendError::Failed(format!("환경 변수 {}가 없습니다", self.api_key_env))
        })?;
        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .json(&self.body(req)?)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    BackendError::Timeout(self.timeout_secs)
                } else {
                    BackendError::Failed(format!("요청 실패: {e}"))
                }
            })?;

        let status = resp.status();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let raw = resp
            .text()
            .await
            .map_err(|e| BackendError::Failed(format!("응답 읽기 실패: {e}")))?;
        if status.as_u16() == 429 {
            return Err(BackendError::RateLimited {
                resets_at: retry_after.map(|s| now_secs() + s),
            });
        }
        if !status.is_success() {
            return Err(BackendError::Failed(format!(
                "HTTP {status}: {}",
                truncate(&raw, 300)
            )));
        }
        let v: Value = serde_json::from_str(&raw)
            .map_err(|e| BackendError::Failed(format!("JSON이 아닌 응답: {e}")))?;

        let blocks = v["content"].as_array().cloned().unwrap_or_default();
        let text: String = blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect();
        let json = match &req.schema {
            Some(schema) => {
                let input = blocks
                    .iter()
                    .find(|b| b["type"] == "tool_use" && b["name"] == TOOL_NAME)
                    .map(|b| b["input"].clone())
                    .ok_or_else(|| {
                        BackendError::InvalidOutput("tool_use 블록이 없습니다".into())
                    })?;
                validate_json(schema, &input)?;
                Some(input)
            }
            None => None,
        };
        let u = &v["usage"];
        let input_tokens = [
            "input_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
        ]
        .iter()
        .filter_map(|k| u[*k].as_u64())
        .reduce(|a, b| a + b);
        Ok(Response {
            text: match &json {
                Some(j) if text.is_empty() => j.to_string(),
                _ => text,
            },
            json,
            model: v["model"].as_str().map(String::from),
            usage: Usage {
                input_tokens,
                output_tokens: u["output_tokens"].as_u64(),
                cost_usd: None,
            },
            raw,
            quota: None,
        })
    }

    async fn probe(&self) -> Probe {
        let has_key = std::env::var(&self.api_key_env).is_ok_and(|k| !k.is_empty());
        Probe {
            available: has_key,
            version: None,
            detail: if has_key {
                String::new()
            } else {
                format!("환경 변수 {} 없음", self.api_key_env)
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::one_shot_server;

    // Tests use a process-unique env var name so they never read a real key.
    const KEY_ENV: &str = "MWA_TEST_ANTHROPIC_KEY_7f3a";

    fn set_key() {
        // SAFETY: only these tests touch this variable, always with the same value.
        unsafe { std::env::set_var(KEY_ENV, "test-key") };
    }

    fn req(schema: Option<Value>) -> Request {
        Request {
            system: "sys".into(),
            prompt: "hi".into(),
            images: vec![],
            schema,
            model: "claude-haiku-5-5".into(),
        }
    }

    #[tokio::test]
    async fn forced_tool_call_returns_validated_json() {
        set_key();
        let body = r#"{"model":"claude-haiku-5-5","content":[{"type":"tool_use","name":"respond","input":{"answer":2}}],"usage":{"input_tokens":10,"output_tokens":5}}"#;
        let (url, rx) = one_shot_server("200 OK", "", body);
        let b = Anthropic::new(url, KEY_ENV.into(), 100, 10);
        let schema = json!({"type":"object","properties":{"answer":{"type":"integer"}},"required":["answer"]});
        let resp = b.complete(&req(Some(schema))).await.unwrap();
        assert_eq!(resp.json, Some(json!({"answer": 2})));
        assert_eq!(resp.usage.input_tokens, Some(10));
        assert_eq!(resp.model.as_deref(), Some("claude-haiku-5-5"));

        let cap = rx.recv().unwrap();
        assert!(cap.head.starts_with("POST /v1/messages"));
        assert!(cap.head.to_lowercase().contains("x-api-key: test-key"));
        let sent: Value = serde_json::from_str(&cap.body).unwrap();
        assert_eq!(sent["tool_choice"]["name"], "respond");
        assert_eq!(sent["system"], "sys");
    }

    #[tokio::test]
    async fn status_429_maps_to_rate_limited() {
        set_key();
        let (url, _rx) = one_shot_server("429 Too Many Requests", "retry-after: 30\r\n", "{}");
        let err = Anthropic::new(url, KEY_ENV.into(), 100, 10)
            .complete(&req(None))
            .await
            .unwrap_err();
        assert!(
            matches!(err, BackendError::RateLimited { resets_at: Some(_) }),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn missing_key_fails_before_request() {
        let b = Anthropic::new(
            "http://127.0.0.1:9".into(),
            "MWA_TEST_UNSET_VAR_91c2".into(),
            100,
            10,
        );
        let err = b.complete(&req(None)).await.unwrap_err();
        assert!(err.to_string().contains("MWA_TEST_UNSET_VAR_91c2"));
        assert!(!b.probe().await.available);
    }
}
