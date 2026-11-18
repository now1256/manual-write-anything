use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One single-shot model call. Backends must not keep state between calls.
#[derive(Debug, Clone)]
pub struct Request {
    pub system: String,
    pub prompt: String,
    /// Image files sent alongside the prompt (PNG/JPEG/GIF/WebP).
    pub images: Vec<PathBuf>,
    /// When set, the backend must return a JSON value that validates against this schema.
    pub schema: Option<Value>,
    pub model: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// List-price estimate. For subscription CLIs this is a quota indicator, not a bill.
    pub cost_usd: Option<f64>,
}

/// Subscription usage window reported by a CLI backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quota {
    /// e.g. `seven_day`, `five_hour`.
    pub window: String,
    pub status: String,
    /// 0.0–1.0 when reported.
    pub utilization: Option<f64>,
    /// Unix seconds.
    pub resets_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub text: String,
    /// Schema-validated JSON when the request carried a schema.
    pub json: Option<Value>,
    /// Model id as reported by the backend (aliases resolved), if known.
    pub model: Option<String>,
    pub usage: Usage,
    /// Raw backend output kept for the run log.
    pub raw: String,
    pub quota: Option<Quota>,
}
