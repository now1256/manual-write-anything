//! Facts read from code by the worker model. Every fact must quote the code it rests on;
//! quotes not found at the cited lines are dropped (F5: no invented evidence).

use mwa_core::Dispatcher;
use mwa_protocol::Role;
use serde::Deserialize;
use serde_json::json;

use crate::ManualError;
use crate::model::{Fact, FactKind, Repo, Source};
use crate::slice::Slice;

const SYSTEM: &str = "You read source code of a web product screen and list facts a Korean user manual needs. \
Write each fact as one short Korean sentence a user would understand (합니다체 not required; plain statement). \
Only state what the code shows. Cover: visible labels (buttons, columns, cards, tabs, field names), input rules \
(required, length, format, what cannot be edited), behaviors (what happens on click/submit, conditions, \
confirmation dialogs, what is disabled when), and messages shown to users (toasts, dialog texts, error texts). \
For every fact cite the file exactly as shown after '===' (without the 'frontend:'/'backend:' prefix), the line \
range, and copy a short exact quote (≤160 chars) from those lines. Do not cite lines you did not see. \
Do not describe implementation details (React hooks, SQL) unless they change what the user experiences.";

#[derive(Deserialize)]
struct Raw {
    facts: Vec<RawFact>,
}

#[derive(Deserialize)]
struct RawFact {
    statement: String,
    kind: String,
    repo: String,
    file: String,
    line_start: usize,
    line_end: usize,
    quote: String,
}

fn schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "facts": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "statement": {"type": "string"},
                        "kind": {"type": "string", "enum": ["label", "rule", "behavior", "message"]},
                        "repo": {"type": "string", "enum": ["frontend", "backend"]},
                        "file": {"type": "string"},
                        "line_start": {"type": "integer"},
                        "line_end": {"type": "integer"},
                        "quote": {"type": "string"}
                    },
                    "required": ["statement", "kind", "repo", "file", "line_start", "line_end", "quote"]
                }
            }
        },
        "required": ["facts"]
    })
}

pub struct Extracted {
    pub facts: Vec<Fact>,
    pub dropped: Vec<String>,
}

/// Runs one worker call per chunk (bounded by the backend's concurrency) and validates quotes.
pub async fn extract(
    d: &Dispatcher,
    slice: &Slice,
    feature: &str,
    chunk_chars: usize,
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<Extracted, ManualError> {
    let chunks = slice.chunks(chunk_chars);
    let total = chunks.len();
    let schema = schema();
    let calls = chunks.iter().enumerate().map(|(i, chunk)| {
        let prompt = format!(
            "기능: {feature}\n아래 코드 조각 {}/{total}에서 사실을 뽑으세요.\n\n{chunk}",
            i + 1
        );
        let schema = &schema;
        async move {
            let out = d
                .call(Role::Worker, SYSTEM, &prompt, &[], Some(schema))
                .await;
            progress(format!("사실 추출 {}/{total} 완료", i + 1));
            out
        }
    });
    let results = futures::future::join_all(calls).await;

    let mut facts = Vec::new();
    let mut dropped = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for r in results {
        let out = r?;
        let raw: Raw = serde_json::from_value(out.response.json.unwrap_or_default())
            .map_err(|e| ManualError::Shape(format!("사실 목록: {e}")))?;
        for f in raw.facts {
            let repo = if f.repo == "backend" {
                Repo::Backend
            } else {
                Repo::Frontend
            };
            let file = f
                .file
                .trim_start_matches("frontend:")
                .trim_start_matches("backend:")
                .to_string();
            if !slice.quote_holds(repo, &file, f.line_start, f.line_end, &f.quote) {
                dropped.push(format!("{} ({file}:{})", f.statement, f.line_start));
                continue;
            }
            let key: String = f.statement.split_whitespace().collect();
            if !seen.insert(key) {
                continue;
            }
            let kind = match f.kind.as_str() {
                "label" => FactKind::Label,
                "rule" => FactKind::Rule,
                "message" => FactKind::Message,
                _ => FactKind::Behavior,
            };
            facts.push(Fact {
                id: String::new(),
                statement: f.statement,
                kind,
                source: Some(Source {
                    repo,
                    file,
                    line_start: f.line_start,
                    line_end: f.line_end,
                    quote: f.quote,
                }),
                basis: "code".into(),
            });
        }
    }
    for (i, f) in facts.iter_mut().enumerate() {
        f.id = format!("F{:03}", i + 1);
    }
    Ok(Extracted { facts, dropped })
}
