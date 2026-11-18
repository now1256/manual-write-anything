//! Verification (F8): deterministic checks, then a judge model reading each block with its evidence.

use std::collections::{BTreeMap, BTreeSet};

use mwa_core::Dispatcher;
use mwa_protocol::Role;
use serde::Deserialize;
use serde_json::json;

use crate::ManualError;
use crate::labels::Label;
use crate::model::{Block, BlockCheck, BlockKind, Fact};

/// Bold spans that look like UI labels: outside table header rows, short, not a sentence.
/// Longer bold text is emphasis of a statement, which the judge checks instead.
pub fn label_like_bold(md: &str) -> Vec<String> {
    let lines: Vec<&str> = md.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let is_header = lines
            .get(i + 1)
            .is_some_and(|n| n.trim_start().starts_with("|") && n.contains("---"));
        if is_header {
            continue;
        }
        for b in bold_spans(line) {
            let sentence =
                b.chars().count() > 20 || b.ends_with(['다', '음', '함', '됨', '요', '.']);
            if !sentence {
                out.push(b);
            }
        }
    }
    out
}

/// Bold spans `**...**` in markdown.
pub fn bold_spans(md: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = md;
    while let Some(i) = rest.find("**") {
        let after = &rest[i + 2..];
        let Some(j) = after.find("**") else { break };
        let t = after[..j].trim();
        if !t.is_empty() {
            out.push(t.to_string());
        }
        rest = &after[j + 2..];
    }
    out
}

/// Text a bold label may legitimately come from: UI labels, menu names, fact statements.
fn label_known(label: &str, labels: &BTreeSet<String>, facts: &[Fact], extra: &[String]) -> bool {
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<String>()
            .trim_matches(['[', ']', '"', '\''])
            .to_string()
    };
    let l = norm(label);
    labels.iter().any(|x| norm(x) == l || norm(x).contains(&l))
        || extra.iter().any(|x| norm(x).contains(&l))
        || facts.iter().any(|f| norm(&f.statement).contains(&l))
}

pub fn deterministic(
    blocks: &[Block],
    facts: &[Fact],
    labels: &[Label],
    shots: &[String],
    extra_terms: &[String],
) -> Vec<BlockCheck> {
    let ids: BTreeSet<&str> = facts.iter().map(|f| f.id.as_str()).collect();
    let label_set: BTreeSet<String> = labels.iter().map(|l| l.text.clone()).collect();
    blocks
        .iter()
        .map(|b| {
            let mut issues = Vec::new();
            let needs_evidence = matches!(
                b.kind,
                BlockKind::Paragraph | BlockKind::Table | BlockKind::Steps | BlockKind::Callout
            );
            if needs_evidence && b.facts.is_empty() {
                issues.push("근거 사실이 없습니다".into());
            }
            for f in &b.facts {
                if !ids.contains(f.as_str()) {
                    issues.push(format!("없는 사실 {f}을 인용했습니다"));
                }
            }
            if b.kind == BlockKind::Image {
                match &b.shot {
                    Some(s) if shots.contains(s) => {}
                    other => issues.push(format!("없는 스크린샷 {other:?}")),
                }
            }
            if needs_evidence {
                for l in label_like_bold(&b.markdown) {
                    if !label_known(&l, &label_set, facts, extra_terms) {
                        issues.push(format!("굵은 라벨 '{l}'이 화면 라벨·사실에 없습니다"));
                    }
                }
            }
            BlockCheck {
                id: b.id.clone(),
                issues,
                verdict: if needs_evidence {
                    "pending".into()
                } else {
                    "skipped".into()
                },
                reason: String::new(),
            }
        })
        .collect()
}

const SYSTEM: &str = "You are a strict reviewer of a Korean user manual. For each block you get the markdown and \
the facts (with code quotes) it cites. Decide whether every claim in the block is supported by its cited facts: \
'supported' (all claims backed), 'partial' (some claim goes beyond the facts), 'unsupported' (main claim not \
backed or contradicted). In 'reason' (Korean, one or two sentences) name the exact claim that is not backed. \
Do not judge style. Facts with basis 'code' count as evidence.";

#[derive(Deserialize)]
struct Raw {
    blocks: Vec<RawVerdict>,
}

#[derive(Deserialize)]
struct RawVerdict {
    id: String,
    verdict: String,
    reason: String,
}

pub async fn judge(
    d: &Dispatcher,
    blocks: &[Block],
    facts: &[Fact],
    checks: &mut [BlockCheck],
) -> Result<(), ManualError> {
    let by_id: BTreeMap<&str, &Fact> = facts.iter().map(|f| (f.id.as_str(), f)).collect();
    let items: Vec<serde_json::Value> = blocks
        .iter()
        .zip(checks.iter())
        .filter(|(_, c)| c.verdict == "pending")
        .map(|(b, _)| {
            let cited: Vec<serde_json::Value> = b
                .facts
                .iter()
                .filter_map(|id| by_id.get(id.as_str()))
                .map(|f| {
                    json!({"id": f.id, "statement": f.statement, "basis": f.basis,
                                "quote": f.source.as_ref().map(|s| s.quote.clone())})
                })
                .collect();
            json!({"id": b.id, "markdown": b.markdown, "facts": cited})
        })
        .collect();
    if items.is_empty() {
        return Ok(());
    }
    let schema = json!({
        "type": "object",
        "properties": {"blocks": {"type": "array", "items": {"type": "object", "properties": {
            "id": {"type": "string"},
            "verdict": {"type": "string", "enum": ["supported", "partial", "unsupported"]},
            "reason": {"type": "string"}
        }, "required": ["id", "verdict", "reason"]}}},
        "required": ["blocks"]
    });
    let out = d
        .call(
            Role::Verifier,
            SYSTEM,
            &serde_json::to_string_pretty(&items).expect("serializes"),
            &[],
            Some(&schema),
        )
        .await?;
    let raw: Raw = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("검증: {e}")))?;
    for v in raw.blocks {
        if let Some(c) = checks.iter_mut().find(|c| c.id == v.id) {
            c.verdict = v.verdict;
            c.reason = v.reason;
        }
    }
    for c in checks.iter_mut().filter(|c| c.verdict == "pending") {
        c.verdict = "unsupported".into();
        c.reason = "판정자가 이 블록에 답하지 않았습니다".into();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(id: &str, s: &str) -> Fact {
        Fact {
            id: id.into(),
            statement: s.into(),
            kind: crate::model::FactKind::Behavior,
            source: None,
            basis: "code".into(),
        }
    }

    #[test]
    fn deterministic_checks() {
        let facts = vec![fact("F001", "등록한 계정은 활성 상태로 생성됩니다")];
        let labels = vec![Label {
            text: "사용자 등록".into(),
            origin: "jsx".into(),
            file: "a.tsx".into(),
            line: 1,
        }];
        let blocks = vec![
            Block {
                id: "a".into(),
                kind: BlockKind::Paragraph,
                markdown: "**사용자 등록**을 누릅니다.".into(),
                facts: vec!["F001".into()],
                shot: None,
            },
            Block {
                id: "b".into(),
                kind: BlockKind::Paragraph,
                markdown: "**일괄 업로드**로 올립니다.".into(),
                facts: vec![],
                shot: None,
            },
            Block {
                id: "c".into(),
                kind: BlockKind::Image,
                markdown: "".into(),
                facts: vec![],
                shot: Some("menus/x".into()),
            },
            Block {
                id: "d".into(),
                kind: BlockKind::Table,
                markdown: "x".into(),
                facts: vec!["F999".into()],
                shot: None,
            },
        ];
        let c = deterministic(&blocks, &facts, &labels, &["menus/y".into()], &[]);
        assert!(c[0].issues.is_empty(), "{:?}", c[0]);
        assert_eq!(c[1].issues.len(), 2, "{:?}", c[1]);
        assert_eq!(c[2].verdict, "skipped");
        assert_eq!(c[2].issues.len(), 1);
        assert!(c[3].issues[0].contains("F999"));
        assert_eq!(bold_spans("a **b** c ** d **"), vec!["b", "d"]);
        let table = "| **입력 항목** | **비고** |\n| --- | --- |\n| **아이디** | **등록 후 변경할 수 없음** |";
        assert_eq!(
            label_like_bold(table),
            vec!["아이디"],
            "header row and sentence-like bold skipped"
        );
    }
}
