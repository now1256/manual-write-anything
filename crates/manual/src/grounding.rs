//! Why each block of an editable manual says what it says (D-039): every text block is checked
//! against the code facts, with a verdict, the facts it rests on and a short explanation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ManualError;
use crate::document::{blocks, text_of};
use crate::provenance::bid_of;

/// The basis of one block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ground {
    /// `supported`, `partial`, `contradicted`, `unsupported` (no code fact covers it),
    /// `no_claim` (headings, navigation, wording with nothing to check).
    pub verdict: String,
    #[serde(default)]
    pub facts: Vec<String>,
    /// Why the block can (or cannot) say this, in Korean.
    pub why: String,
    /// The block's text when checked; a different text now means the check is stale.
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Grounding {
    /// Document version checked.
    pub version: u32,
    pub at: u64,
    pub blocks: BTreeMap<String, Ground>,
}

fn path(ws: &Path, feature: &str) -> PathBuf {
    ws.join("manual/docs").join(feature).join("grounding.json")
}

pub fn save(ws: &Path, feature: &str, g: &Grounding) -> Result<(), ManualError> {
    let file = path(ws, feature);
    std::fs::write(&file, serde_json::to_string_pretty(g).expect("serializes"))
        .map_err(|e| ManualError::Io(file, e))
}

pub fn load(ws: &Path, feature: &str) -> Grounding {
    std::fs::read_to_string(path(ws, feature))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

const SYSTEM: &str = "You check a Korean user manual against facts read from the product's \
current code. For each numbered block decide what supports it:
- supported: the facts back every product statement in the block;
- partial: some statements are backed, others are not covered;
- contradicted: a fact says otherwise (name, option, rule, message, who may do it);
- unsupported: it states product behavior no fact covers (it may come from the old manual);
- no_claim: nothing to check (a heading, an outline, a picture caption, plain guidance).
facts: the ids that back or contradict it. why: one or two Korean sentences (합니다체) explaining \
the basis, quoting what the fact says when it matters, e.g. 'F012에 내보내기 형식이 CSV와 XLSX로 \
나와 있어 이렇게 썼습니다' or '코드 사실에 이 규칙이 없어 기존 매뉴얼 내용 그대로입니다'. Return \
one item per block, with its id.";

/// Checks every text block of the feature's document against `facts`.
pub async fn ground(
    d: &mwa_core::Dispatcher,
    ws: &Path,
    feature: &str,
    facts: &[crate::model::Fact],
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<Grounding, ManualError> {
    let doc = crate::document::load(ws, feature)
        .ok_or_else(|| ManualError::Invalid(format!("매뉴얼 문서가 없습니다: {feature}")))?;
    let items: Vec<(String, String)> = blocks(&doc.html)
        .iter()
        .filter(|b| !b.starts_with("<figure"))
        .filter_map(|b| Some((bid_of(b)?, text_of(b))))
        .filter(|(_, t)| !t.is_empty())
        .collect();
    let fact_lines: Vec<String> = facts
        .iter()
        .map(|f| format!("{}: {}", f.id, f.statement))
        .collect();
    let schema = serde_json::json!({
        "type": "object",
        "properties": {"items": {"type": "array", "items": {
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "verdict": {"type": "string", "enum": ["supported", "partial", "contradicted", "unsupported", "no_claim"]},
                "facts": {"type": "array", "items": {"type": "string"}},
                "why": {"type": "string"}
            },
            "required": ["id", "verdict", "facts", "why"]
        }}},
        "required": ["items"]
    });
    let batches: Vec<&[(String, String)]> = items.chunks(12).collect();
    let total = batches.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let calls = batches.iter().map(|batch| {
        let list: Vec<String> = batch.iter().map(|(id, t)| format!("[{id}] {t}")).collect();
        let prompt = format!(
            "매뉴얼: {}\n\n## 블록\n{}\n\n## 현재 코드의 사실\n{}",
            doc.title,
            list.join("\n"),
            if fact_lines.is_empty() {
                "(없음)".to_string()
            } else {
                fact_lines.join("\n")
            }
        );
        let (schema, done) = (&schema, &done);
        async move {
            let out = d
                .call(
                    mwa_protocol::Role::Verifier,
                    SYSTEM,
                    &prompt,
                    &[],
                    Some(schema),
                )
                .await;
            let k = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            progress(format!("근거 확인 {k}/{total}"));
            out
        }
    });
    let results = futures::future::join_all(calls).await;
    #[derive(Deserialize)]
    struct Raw {
        items: Vec<RawItem>,
    }
    #[derive(Deserialize)]
    struct RawItem {
        id: String,
        verdict: String,
        #[serde(default)]
        facts: Vec<String>,
        why: String,
    }
    let known: std::collections::HashSet<&str> = facts.iter().map(|f| f.id.as_str()).collect();
    let text: BTreeMap<&str, &str> = items
        .iter()
        .map(|(b, t)| (b.as_str(), t.as_str()))
        .collect();
    let mut g = Grounding {
        version: doc.version,
        at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        blocks: BTreeMap::new(),
    };
    for r in results {
        let raw: Raw = serde_json::from_value(
            r.map_err(|e| ManualError::Invalid(format!("근거 확인 실패: {e}")))?
                .response
                .json
                .unwrap_or_default(),
        )
        .map_err(|e| ManualError::Shape(format!("근거 확인: {e}")))?;
        for it in raw.items {
            let Some(t) = text.get(it.id.as_str()) else {
                continue;
            };
            g.blocks.insert(
                it.id,
                Ground {
                    verdict: it.verdict,
                    facts: it
                        .facts
                        .into_iter()
                        .filter(|f| known.contains(f.as_str()))
                        .collect(),
                    why: it.why,
                    text: (*t).to_string(),
                },
            );
        }
    }
    let file = path(ws, feature);
    std::fs::write(&file, serde_json::to_string_pretty(&g).expect("serializes"))
        .map_err(|e| ManualError::Io(file, e))?;
    Ok(g)
}

/// "확인됨 30 · 일부 5 · 다름 2 · 근거 없음 8".
pub fn summarize(g: &Grounding) -> String {
    let n = |v: &str| g.blocks.values().filter(|x| x.verdict == v).count();
    format!(
        "확인됨 {} · 일부만 {} · 코드와 다름 {} · 코드에 근거 없음 {} · 확인할 내용 없음 {}",
        n("supported"),
        n("partial"),
        n("contradicted"),
        n("unsupported"),
        n("no_claim")
    )
}
