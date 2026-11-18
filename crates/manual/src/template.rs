//! Manual template learned from an example manual (F4): section plan and style rules.

use mwa_core::Dispatcher;
use mwa_protocol::Role;
use serde_json::json;

use crate::ManualError;
use crate::model::Template;

const SYSTEM: &str = "You analyse an example Korean product manual page and describe its template so that \
another feature's page can be written the same way. List the sections in order with their purpose and typical \
block types (paragraph, table, steps, callout, image), and mark which sections every feature page should have. \
Then list concrete style rules you observe (sentence ending, how menu paths are written, how UI labels are \
emphasised, how screenshots are referenced, table conventions, separators). Write titles exactly as in the \
example; write purposes and rules in Korean. Ignore content specific to the example's feature.";

pub async fn learn(d: &Dispatcher, example: &str, source: &str) -> Result<Template, ManualError> {
    let schema = json!({
        "type": "object",
        "properties": {
            "sections": {"type": "array", "items": {"type": "object", "properties": {
                "title": {"type": "string"},
                "purpose": {"type": "string"},
                "typical_blocks": {"type": "array", "items": {"type": "string"}},
                "required": {"type": "boolean"}
            }, "required": ["title", "purpose", "typical_blocks", "required"]}},
            "style_rules": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["sections", "style_rules"]
    });
    let out = d
        .call(
            Role::Writer,
            SYSTEM,
            &format!("[예시 매뉴얼: {source}]\n\n{example}"),
            &[],
            Some(&schema),
        )
        .await?;
    let mut v = out.response.json.unwrap_or_default();
    v["source"] = json!(source);
    serde_json::from_value(v).map_err(|e| ManualError::Shape(format!("양식: {e}")))
}
