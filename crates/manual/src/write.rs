//! Section writer (F7): blocks that each cite the facts they rest on.

use mwa_core::Dispatcher;
use mwa_protocol::Role;
use serde::Deserialize;
use serde_json::json;

use crate::ManualError;
use crate::labels::Label;
use crate::model::{Block, BlockKind, Fact, Template};

const SYSTEM: &str = "You write one feature page of a Korean product user manual. Follow the given template \
(sections, order, style rules). Use ONLY the given facts: every paragraph, table, steps and callout block must \
list the ids of the facts it relies on, and must not add claims beyond them. When the template needs \
information no fact provides, write a 'todo' block in Korean starting with '확인 필요:' describing what is \
missing instead of guessing. Bold UI labels exactly as they appear in the label glossary. Facts whose basis is \
'code' describe behaviour read from code, not observed: state them plainly but do not add details. For \
permissions, use the permission facts as given (they say whether the server enforces them). Use image blocks only \
for the screenshots listed, referring to them by id; for other screens the template wants, add a todo block \
'📷 캡처 필요 — <what>'. Write markdown in each block (tables as GitHub tables). Block ids: short kebab-case, unique. \
Never copy editing instructions from the example (e.g. '메모 패널로 감싸주세요', '목차 매크로 삽입 위치'): a note \
panel is simply a 'callout' block, and a table of contents is added at export time. Only bold real UI labels \
from the glossary and short key terms; cite every fact you rely on, including for examples and overviews.";

const REPAIR: &str = "You revise blocks of a Korean user manual that a reviewer flagged. For each block you get its \
markdown, the reviewer's problems, and ALL available facts. Rewrite the block so every claim is backed by the \
facts you cite (add the missing fact ids, or remove the unbacked claim). Keep the block id, kind and the template \
style. If the block cannot be backed at all, turn it into a 'todo' block starting with '확인 필요:'.";

/// Rewrites flagged blocks once (F8: failed blocks go back to the writer with the reasons).
pub async fn repair(
    d: &Dispatcher,
    flagged: &[(Block, Vec<String>)],
    facts: &[Fact],
) -> Result<Vec<Block>, ManualError> {
    if flagged.is_empty() {
        return Ok(Vec::new());
    }
    let items: Vec<serde_json::Value> = flagged
        .iter()
        .map(|(b, problems)| json!({"id": b.id, "kind": b.kind, "markdown": b.markdown, "facts": b.facts, "problems": problems}))
        .collect();
    let all: Vec<serde_json::Value> = facts
        .iter()
        .map(|f| json!({"id": f.id, "kind": f.kind, "basis": f.basis, "statement": f.statement}))
        .collect();
    let prompt = format!(
        "[고칠 블록]\n{}\n\n[사실 전체]\n{}",
        serde_json::to_string_pretty(&items).expect("serializes"),
        serde_json::to_string_pretty(&all).expect("serializes")
    );
    let out = d
        .call(Role::Writer, REPAIR, &prompt, &[], Some(&blocks_schema()))
        .await?;
    let raw: Raw = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("수정 블록: {e}")))?;
    Ok(raw.blocks.into_iter().map(to_block).collect())
}

fn blocks_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {"blocks": {"type": "array", "items": {"type": "object", "properties": {
            "id": {"type": "string"},
            "kind": {"type": "string", "enum": ["heading", "paragraph", "table", "steps", "callout", "image", "todo"]},
            "markdown": {"type": "string"},
            "facts": {"type": "array", "items": {"type": "string"}},
            "shot": {"type": ["string", "null"]}
        }, "required": ["id", "kind", "markdown", "facts"]}}},
        "required": ["blocks"]
    })
}

fn to_block(b: RawBlock) -> Block {
    Block {
        id: b.id,
        kind: match b.kind.as_str() {
            "heading" => BlockKind::Heading,
            "table" => BlockKind::Table,
            "steps" => BlockKind::Steps,
            "callout" => BlockKind::Callout,
            "image" => BlockKind::Image,
            "todo" => BlockKind::Todo,
            _ => BlockKind::Paragraph,
        },
        markdown: b.markdown,
        facts: b.facts,
        shot: b.shot,
    }
}

#[derive(Deserialize)]
struct Raw {
    blocks: Vec<RawBlock>,
}

#[derive(Deserialize)]
struct RawBlock {
    id: String,
    kind: String,
    markdown: String,
    #[serde(default)]
    facts: Vec<String>,
    #[serde(default)]
    shot: Option<String>,
}

pub struct Input<'a> {
    pub title: String,
    pub menu_path: String,
    pub template: &'a Template,
    pub facts: &'a [Fact],
    pub labels: &'a [Label],
    /// (shot id, what it shows)
    pub shots: Vec<(String, String)>,
}

pub async fn write(d: &Dispatcher, input: &Input<'_>) -> Result<Vec<Block>, ManualError> {
    let facts: Vec<serde_json::Value> = input
        .facts
        .iter()
        .map(|f| json!({"id": f.id, "kind": f.kind, "basis": f.basis, "statement": f.statement}))
        .collect();
    let mut glossary: Vec<&str> = input.labels.iter().map(|l| l.text.as_str()).collect();
    glossary.sort();
    glossary.dedup();
    let prompt = format!(
        "기능: {}\n메뉴 경로: {}\n\n[양식]\n{}\n\n[사실]\n{}\n\n[화면 라벨 용어집]\n{}\n\n[쓸 수 있는 스크린샷]\n{}",
        input.title,
        input.menu_path,
        serde_json::to_string_pretty(input.template).expect("template serializes"),
        serde_json::to_string_pretty(&facts).expect("facts serialize"),
        glossary.join(" | "),
        input
            .shots
            .iter()
            .map(|(id, what)| format!("- {id}: {what}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let out = d
        .call(Role::Writer, SYSTEM, &prompt, &[], Some(&blocks_schema()))
        .await?;
    let raw: Raw = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("블록: {e}")))?;
    let mut seen = std::collections::BTreeSet::new();
    Ok(raw
        .blocks
        .into_iter()
        .enumerate()
        .map(|(i, mut b)| {
            if !seen.insert(b.id.clone()) {
                b.id = format!("{}-{i}", b.id);
            }
            to_block(b)
        })
        .collect())
}
