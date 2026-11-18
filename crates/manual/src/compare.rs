//! Comparison with an existing manual page (M3 acceptance, and the "what changed" report).

use mwa_core::Dispatcher;
use mwa_protocol::Role;
use serde::Deserialize;
use serde_json::json;

use crate::ManualError;
use crate::model::{CompareItem, Fact};

const SYSTEM: &str = "You compare two Korean manual pages for the same product feature: GENERATED (written from \
the current code) and EXISTING (an older manual). Break both into factual claims about the product (what exists, \
rules, behaviours, permissions, messages) and align them by topic. For each topic give status: 'same' (both say \
the same), 'only_existing', 'only_generated', or 'conflict' (they disagree). For conflicts and for \
'only_existing' claims, check the CODE FACTS: in 'code_says' state in Korean which side the code supports, or \
'코드 사실로 판단 불가' if no fact covers it, set code_supports to 'generated', 'existing' or 'unknown', and list \
the fact ids used. Ignore wording and layout differences. \
Write topic and claims in Korean, briefly.";

#[derive(Deserialize)]
struct Raw {
    items: Vec<CompareItem>,
}

pub async fn compare(
    d: &Dispatcher,
    generated: &str,
    existing: &str,
    facts: &[Fact],
) -> Result<Vec<CompareItem>, ManualError> {
    let facts: Vec<serde_json::Value> = facts
        .iter()
        .map(|f| json!({"id": f.id, "statement": f.statement, "basis": f.basis}))
        .collect();
    let prompt = format!(
        "[GENERATED]\n{generated}\n\n[EXISTING]\n{existing}\n\n[CODE FACTS]\n{}",
        serde_json::to_string_pretty(&facts).expect("serializes")
    );
    let schema = json!({
        "type": "object",
        "properties": {"items": {"type": "array", "items": {"type": "object", "properties": {
            "topic": {"type": "string"},
            "generated": {"type": ["string", "null"]},
            "existing": {"type": ["string", "null"]},
            "status": {"type": "string", "enum": ["same", "only_existing", "only_generated", "conflict"]},
            "code_says": {"type": ["string", "null"]},
            "code_supports": {"type": ["string", "null"], "enum": ["generated", "existing", "unknown", null]},
            "facts": {"type": "array", "items": {"type": "string"}}
        }, "required": ["topic", "status"]}}},
        "required": ["items"]
    });
    let out = d
        .call(Role::Verifier, SYSTEM, &prompt, &[], Some(&schema))
        .await?;
    let raw: Raw = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("비교: {e}")))?;
    Ok(raw.items)
}
