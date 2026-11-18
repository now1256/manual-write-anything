//! `api:openai-compat` and `ollama`: OpenAI-style `/chat/completions`.
//! Ollama exposes the same API under `/v1`, so it shares this adapter without a key.

use std::time::Duration;

use async_trait::async_trait;
use mwa_protocol::{Request, Response, Usage};
use serde_json::{Value, json};

use crate::{Backend, BackendError, Probe, load_image, now_secs, truncate, validate_json};

pub struct OpenAiCompat {
    kind: &'static str,
    base_url: String,
    api_key_env: Option<String>,
    timeout_secs: u64,
    client: reqwest::Client,
}

impl OpenAiCompat {
    pub fn new(
        kind: &'static str,
        base_url: String,
        api_key_env: Option<String>,
        timeout_secs: u64,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .expect("reqwest client");
        Self {
            kind,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key_env,
            timeout_secs,
            client,
        }
    }

    fn key(&self) -> Result<Option<String>, BackendError> {
        match &self.api_key_env {
            None => Ok(None),
            Some(name) => std::env::var(name)
                .map(Some)
                .map_err(|_| BackendError::Failed(format!("환경 변수 {name}가 없습니다"))),
        }
    }

    fn body(req: &Request) -> Result<Value, BackendError> {
        let user = if req.images.is_empty() {
            json!(req.prompt)
        } else {
            let mut parts = vec![json!({"type": "text", "text": req.prompt})];
            for path in &req.images {
                let img = load_image(path)?;
                parts.push(json!({
                    "type": "image_url",
                    "image_url": {"url": format!("data:{};base64,{}", img.media_type, img.base64)}
                }));
            }
            json!(parts)
        };
        let mut body = json!({
            "model": req.model,
            "messages": [
                {"role": "system", "content": req.system},
                {"role": "user", "content": user},
            ],
        });
        if let Some(schema) = &req.schema {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {"name": "response", "schema": schema},
            });
        }
        Ok(body)
    }
}

#[async_trait]
impl Backend for OpenAiCompat {
    fn kind(&self) -> &'static str {
        self.kind
    }

    async fn complete(&self, req: &Request) -> Result<Response, BackendError> {
        let mut http = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&Self::body(req)?);
        if let Some(key) = self.key()? {
            http = http.bearer_auth(key);
        }
        let resp = http.send().await.map_err(|e| {
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
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let json = match &req.schema {
            Some(schema) => {
                let parsed: Value = serde_json::from_str(&text).map_err(|e| {
                    BackendError::InvalidOutput(format!(
                        "JSON 파싱 실패: {e}: {}",
                        truncate(&text, 200)
                    ))
                })?;
                validate_json(schema, &parsed)?;
                Some(parsed)
            }
            None => None,
        };
        Ok(Response {
            text,
            json,
            model: v["model"].as_str().map(String::from),
            usage: Usage {
                input_tokens: v["usage"]["prompt_tokens"].as_u64(),
                output_tokens: v["usage"]["completion_tokens"].as_u64(),
                cost_usd: None,
            },
            raw,
            quota: None,
        })
    }

    async fn probe(&self) -> Probe {
        if let Err(e) = self.key() {
            return Probe {
                available: false,
                version: None,
                detail: e.to_string(),
            };
        }
        let mut http = self
            .client
            .get(format!("{}/models", self.base_url))
            .timeout(Duration::from_secs(5));
        if let Ok(Some(key)) = self.key() {
            http = http.bearer_auth(key);
        }
        match http.send().await {
            Ok(r) if r.status().is_success() => Probe {
                available: true,
                version: None,
                detail: String::new(),
            },
            Ok(r) => Probe {
                available: false,
                version: None,
                detail: format!("{} → HTTP {}", self.base_url, r.status()),
            },
            Err(e) => Probe {
                available: false,
                version: None,
                detail: format!(
                    "{} 연결 실패: {}",
                    self.base_url,
                    truncate(&e.to_string(), 120)
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::one_shot_server;

    fn req(schema: Option<Value>) -> Request {
        Request {
            system: "sys".into(),
            prompt: "hi".into(),
            images: vec![],
            schema,
            model: "qwen3".into(),
        }
    }

    #[tokio::test]
    async fn json_schema_response_is_parsed_and_validated() {
        let body = r#"{"model":"qwen3","choices":[{"message":{"content":"{\"answer\":2}"}}],"usage":{"prompt_tokens":7,"completion_tokens":3}}"#;
        let (url, rx) = one_shot_server("200 OK", "", body);
        let b = OpenAiCompat::new("ollama", url, None, 10);
        let schema = json!({"type":"object","properties":{"answer":{"type":"integer"}},"required":["answer"]});
        let resp = b.complete(&req(Some(schema))).await.unwrap();
        assert_eq!(resp.json, Some(json!({"answer": 2})));
        assert_eq!(resp.usage.output_tokens, Some(3));

        let cap = rx.recv().unwrap();
        assert!(cap.head.starts_with("POST /chat/completions"));
        assert!(!cap.head.to_lowercase().contains("authorization"));
        let sent: Value = serde_json::from_str(&cap.body).unwrap();
        assert_eq!(sent["response_format"]["type"], "json_schema");
        assert_eq!(sent["messages"][1]["content"], "hi");
    }

    #[tokio::test]
    async fn non_json_content_with_schema_is_invalid() {
        let body = r#"{"choices":[{"message":{"content":"two"}}]}"#;
        let (url, _rx) = one_shot_server("200 OK", "", body);
        let schema = json!({"type":"object"});
        let err = OpenAiCompat::new("ollama", url, None, 10)
            .complete(&req(Some(schema)))
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::InvalidOutput(_)), "{err:?}");
    }

    #[tokio::test]
    async fn server_error_is_failed() {
        let (url, _rx) = one_shot_server("500 Internal Server Error", "", r#"{"error":"x"}"#);
        let err = OpenAiCompat::new("ollama", url, None, 10)
            .complete(&req(None))
            .await
            .unwrap_err();
        assert!(
            matches!(err, BackendError::Failed(ref m) if m.contains("500")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn unreachable_server_probe_is_unavailable() {
        let b = OpenAiCompat::new("ollama", "http://127.0.0.1:9".into(), None, 2);
        assert!(!b.probe().await.available);
    }
}
