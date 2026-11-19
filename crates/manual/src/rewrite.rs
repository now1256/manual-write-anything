//! Writing a feature's manual from the code (D-043): the old manual (converted from the PDF) is
//! the model for outline, wording and format, not the source of truth. Every block records why it
//! says what it says. Features without an old manual are written after the project's finished
//! manuals.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use crate::ManualError;
use crate::document::{self, Doc, blocks, text_of};
use crate::grounding::{Ground, Grounding};
use crate::model::Fact;
use crate::provenance::{self, Entry, LogItem, LogVersion, bid_of, set_bid};

const SECTION_SYSTEM: &str = concat!(
    "You rewrite one section of a Korean user manual so that it matches \
the product's current code. You get the section as it is in the old manual (HTML blocks with \
data-bid) and facts read from the current code (each with an id).

Keep the old manual's outline, headings, table layout, panels, pictures and wording style \
(합니다체 or the list style it uses). Change the content to what the facts say:
- a statement the facts contradict is corrected (basis changed);
- what the facts cover is kept or restated (basis code);
- an element, option, rule, column or step in the facts that belongs in this section but is \
missing is added where it fits (basis new); only what a user sees or needs, not internals;
- operating guidance the code cannot show (roles and responsibilities, recommendations, \
procedures) is kept as it is (basis kept);
- a statement about the product that the facts neither support nor contradict is kept (basis \
kept); never drop it just because no fact mentions it;
- headings, outlines, captions and plain connective text have basis none.
When pictures are attached, they are the current screens of this section's figures (listed with \
what each shows and the screen changes found against the old picture). The text must match them: \
every column, card, button, field, option, tab and message a user works with that is visible \
there and missing from the old text is added (basis screen); what the old text says differently \
is corrected (basis changed); each listed screen change is reflected in the text. Values in the \
pictures (names, numbers, dates) are sample data: never write them.
",
    user_rule!(),
    " Return the section as blocks in order, one top-level HTML block each. Keep the data-bid of every \
block you keep or change; new blocks have none. Copy <figure> blocks unchanged. Use only h2 h3 h4 \
p ul ol li strong em u s code table tr th td br hr, div data-panel, span data-status, figure. \
facts: the ids each block rests on. why: one Korean sentence (합니다체) on why the block says \
this, e.g. 'F012에 내보내기 형식이 CSV와 XLSX로 나와 있어 XLSX를 추가했습니다'."
);

const EXTRA_SYSTEM: &str = concat!(
    "Some facts from the product's current code are not covered by its \
Korean user manual yet. Decide which matter to a user (screens, buttons, fields, options, rules, \
messages; not internals) and write blocks for them in the manual's style (합니다체, its table \
and list conventions). Put each group after the heading it belongs under (exact heading text \
from the list). ",
    user_rule!(),
    " Skip facts that do not matter to a user. basis is new; facts: the ids each block \
rests on; why: one Korean sentence."
);

const OUTLINE_SYSTEM: &str = "You plan a Korean user manual for one feature of a software \
product, from facts read from its current code (each with an id), in the house format of the \
example manuals of other features. Return its parts in order. The first part has level 0: what \
comes before the first heading (the overview panel and the outline list). Then the headings the \
examples have, in their order, with one sub-part per screen \
element (cards, search, list and its columns) and per action (register, view details and each \
of its tabs, edit, activate or deactivate, delete, …) that the facts show, at the heading levels \
the examples use. Each part: heading (Korean, as the examples name such parts), level (2-4, 0 \
for the first), facts (the ids it covers: every fact that matters to a user goes to exactly one \
part; leave out internals), note (one Korean sentence on what to write there), picture (what its \
picture must show, as '화면 — 상태 — 자를 부분', e.g. '사용자 관리 — 사용자 등록 창이 열리고 입력 \
칸이 비어 있음 — 창 하나', when the part explains the first screen or a dialog, drawer, tab or \
confirmation a user reaches by clicking, typing or choosing; it is shot automatically on the real \
product, so never a state that needs saving, deleting or an error from the server; else empty).";

const PART_SYSTEM: &str = concat!(
    "You write one part of a Korean user manual for one feature of a software product, from \
facts read from its current code (each with an id), in the house format of the example manuals: \
their heading levels, tables, lists, panels and wording (합니다체, or the list style they use). \
You get the whole outline and the part to write. Start with the part's heading as <hN> (level \
0 has none: it is the overview panel and the outline list of the headings, as the examples \
have). Write every fact given for the part that matters to a user, with tables for fields, \
columns, cards and messages where the examples use them. ",
    user_rule!(),
    "Where the house format needs something the facts do not give (e.g. roles), write a short \
<div data-panel=\"warning\"><p>확인 필요: …</p></div> instead of inventing it. When the part \
has a picture, put <figure data-want=\"PICTURE\"><figcaption>CAPTION</figcaption></figure> where \
the examples put a picture in such a part, PICTURE exactly as given. Write only this part. Return \
blocks in order, one top-level HTML block each; basis: code for what facts back, none for \
headings and the outline. facts: ids; why: one Korean sentence."
);

#[derive(Deserialize)]
struct RawBlock {
    html: String,
    basis: String,
    #[serde(default)]
    facts: Vec<String>,
    #[serde(default)]
    why: String,
}

fn blocks_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {"blocks": {"type": "array", "items": {
            "type": "object",
            "properties": {
                "html": {"type": "string"},
                "basis": {"type": "string", "enum": ["code", "changed", "new", "screen", "kept", "none"]},
                "facts": {"type": "array", "items": {"type": "string"}},
                "why": {"type": "string"}
            },
            "required": ["html", "basis", "facts", "why"]
        }}},
        "required": ["blocks"]
    })
}

fn fact_lines(facts: &[Fact]) -> String {
    facts
        .iter()
        .map(|f| format!("{}: {}", f.id, f.statement))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The old manual split at each `<h2>` (the part before the first one is a section too).
fn sections(html: &str) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = vec![Vec::new()];
    for b in blocks(html) {
        if b.starts_with("<h2") && !out.last().is_some_and(Vec::is_empty) {
            out.push(Vec::new());
        }
        out.last_mut().expect("one section").push(b);
    }
    out.retain(|s| !s.is_empty());
    out
}

/// One written block, with its reason.
struct Written {
    html: String,
    basis: String,
    facts: Vec<String>,
    why: String,
    /// What proofreading fixed in this block.
    polish: Option<String>,
}

/// One written block per top-level element: models sometimes put a heading, its text and a
/// picture in one block, and every element needs its own id, history and basis.
fn split(written: Vec<Written>) -> Vec<Written> {
    written
        .into_iter()
        .flat_map(|w| {
            let parts = blocks(&w.html);
            if parts.len() <= 1 {
                return vec![w];
            }
            parts
                .into_iter()
                .map(|html| Written {
                    html,
                    basis: w.basis.clone(),
                    facts: w.facts.clone(),
                    why: w.why.clone(),
                    polish: w.polish.clone(),
                })
                .collect()
        })
        .collect()
}

async fn call_blocks(
    d: &mwa_core::Dispatcher,
    system: &str,
    prompt: &str,
    images: &[std::path::PathBuf],
) -> Result<Vec<Written>, ManualError> {
    let out = d
        .call(
            mwa_protocol::Role::Writer,
            system,
            prompt,
            images,
            Some(&blocks_schema()),
        )
        .await
        .map_err(|e| ManualError::Invalid(format!("매뉴얼 쓰기 실패: {e}")))?;
    #[derive(Deserialize)]
    struct Raw {
        blocks: Vec<RawBlock>,
    }
    let raw: Raw = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("매뉴얼 쓰기: {e}")))?;
    Ok(raw
        .blocks
        .into_iter()
        .filter(|b| !b.html.trim().is_empty())
        .map(|b| Written {
            html: b.html,
            basis: b.basis,
            facts: b.facts,
            why: b.why,
            polish: None,
        })
        .collect())
}

const POLISH_SYSTEM: &str = concat!(
    "You proofread a Korean user manual that was just written for one feature, reading it as \
its user would. You get the whole manual as HTML blocks with data-bid, and the facts read from \
the product's code (each with an id). ",
    user_rule!(),
    "Fix only real problems:
- content that breaks the rule above: drop that sentence, list item or table row (the whole \
block when nothing is left);
- a broken, cut or garbled sentence; a menu path not written as names joined by ' > ';
- the same thing said twice (a panel repeating the text above it, two blocks with one content, \
the same rule in two sections): keep the clearer one;
- a quoted message cut into a fragment ('자 이하로 입력해주세요'): complete it from the facts or \
describe it;
- one screen element called by different names: use the screen's own label from the facts;
- a '확인 필요' panel whose question the facts answer: replace it with the answer;
- mixed endings inside one list or table (follow the style the manual uses most);
- a listed screen change (found by comparing the old manual's pictures with the product now) \
that the text does not reflect: fix the block it belongs in.
Do not add claims the facts do not support, do not reorder, do not restyle text that is fine, \
never touch <figure> blocks. Return edits: bid (the block), html (the whole corrected block; \
empty deletes it), why (one Korean sentence, 합니다체, on what was wrong). No edits when nothing \
is wrong."
);

/// Words that in a user manual usually describe the developer's side, not the screen.
const SUSPECT_WORDS: &[&str] = &[
    "서버",
    "API",
    "응답",
    "반환",
    "세션",
    "토큰",
    "로그에",
    "콘솔",
    "캐시",
    "네트워크",
    "이력이 기록",
    "호출",
    "DB",
    "데이터베이스",
    "검증은",
];

/// Reads the written manual once more as its reader and fixes what slipped through (D-046).
/// Returns how many blocks it changed.
async fn polish(
    d: &mwa_core::Dispatcher,
    title: &str,
    facts: &[Fact],
    written: &mut Vec<Written>,
    version: u32,
    screen_changes: &[String],
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<usize, ManualError> {
    progress("사용자 입장에서 교정하는 중".into());
    *written = split(std::mem::take(written));
    for (n, w) in written.iter_mut().enumerate() {
        if bid_of(&w.html).is_none() {
            w.html = set_bid(&w.html, &format!("w{version}-{n}"));
        }
    }
    let schema = serde_json::json!({
        "type": "object",
        "properties": {"edits": {"type": "array", "items": {
            "type": "object",
            "properties": {
                "bid": {"type": "string"},
                "html": {"type": "string"},
                "why": {"type": "string"}
            },
            "required": ["bid", "html", "why"]
        }}},
        "required": ["edits"]
    });
    // Words that usually mean developer-side content: those blocks are named so they get a look.
    let suspects: Vec<String> = written
        .iter()
        .filter_map(|w| {
            let t = text_of(&w.html);
            let hit: Vec<&str> = SUSPECT_WORDS
                .iter()
                .copied()
                .filter(|x| t.contains(x))
                .collect();
            let bid = bid_of(&w.html)?;
            (!hit.is_empty()).then(|| format!("{bid}: {}", hit.join(", ")))
        })
        .collect();
    let prompt = format!(
        "매뉴얼: {title}\n\n## 매뉴얼\n{}\n\n## 코드 사실\n{}\n\n## 꼭 확인할 블록 (내부 내용일 때가 많은 낱말)\n{}\n\n## 글에 반영되어야 할 화면 변화\n{}",
        written
            .iter()
            .map(|w| w.html.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        fact_lines(facts),
        if suspects.is_empty() {
            "(없음)".to_string()
        } else {
            suspects.join("\n")
        },
        if screen_changes.is_empty() {
            "(없음)".to_string()
        } else {
            screen_changes.join("\n")
        }
    );
    let out = d
        .call(
            mwa_protocol::Role::Writer,
            POLISH_SYSTEM,
            &prompt,
            &[],
            Some(&schema),
        )
        .await
        .map_err(|e| ManualError::Invalid(format!("교정 실패: {e}")))?;
    #[derive(Deserialize)]
    struct Edit {
        bid: String,
        html: String,
        why: String,
    }
    #[derive(Deserialize)]
    struct Raw {
        edits: Vec<Edit>,
    }
    let raw: Raw = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("교정: {e}")))?;
    let mut changed = 0;
    let mut gone: BTreeSet<String> = BTreeSet::new();
    for e in raw.edits {
        let Some(w) = written
            .iter_mut()
            .find(|w| bid_of(&w.html).as_deref() == Some(e.bid.as_str()))
        else {
            continue;
        };
        if w.html.starts_with("<figure") {
            continue;
        }
        if e.html.trim().is_empty() {
            gone.insert(e.bid);
        } else {
            w.html = set_bid(e.html.trim(), &e.bid);
            w.polish = Some(e.why);
        }
        changed += 1;
    }
    written.retain(|w| !bid_of(&w.html).is_some_and(|b| gone.contains(&b)));
    Ok(changed)
}

/// Writes the document from blocks with reasons: new version, history and basis for every block.
fn store(
    ws: &Path,
    mut doc: Doc,
    written: Vec<Written>,
    known: &BTreeSet<&str>,
    summary: &str,
) -> Result<Doc, ManualError> {
    let at = document::now();
    document::keep_previous(ws, &doc.feature)?;
    doc.version += 1;
    doc.updated_at = at;
    let mut prov = provenance::load(ws, &doc.feature);
    let mut ground = Grounding {
        version: doc.version,
        at,
        blocks: BTreeMap::new(),
    };
    let mut html = String::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (n, w) in split(written).into_iter().enumerate() {
        let facts: Vec<String> = w
            .facts
            .into_iter()
            .filter(|f| known.contains(f.as_str()))
            .collect();
        let bid = bid_of(&w.html).unwrap_or_else(|| format!("w{}-{n}", doc.version));
        let block = set_bid(&w.html, &bid);
        let (summary, verdict) = match w.basis.as_str() {
            "changed" => (format!("코드에 맞게 고침: {}", w.why), "supported"),
            "new" => (format!("코드에 있어 새로 씀: {}", w.why), "supported"),
            "screen" => (format!("지금 화면에 있어 씀: {}", w.why), "supported"),
            "code" => (format!("코드로 확인해 씀: {}", w.why), "supported"),
            "kept" => (format!("기존 매뉴얼 내용 유지: {}", w.why), "unsupported"),
            _ => (w.why.clone(), "no_claim"),
        };
        *counts
            .entry(match w.basis.as_str() {
                "changed" => "고침",
                "new" => "새로 씀",
                "screen" => "화면 반영",
                "code" => "코드로 확인",
                "kept" => "기존 유지",
                _ => "구성",
            })
            .or_default() += 1;
        prov.blocks.entry(bid.clone()).or_default().push(Entry {
            kind: "rewrite".into(),
            version: doc.version,
            at,
            page: None,
            facts: facts.clone(),
            request: None,
            summary,
        });
        if let Some(fix) = &w.polish {
            prov.blocks.entry(bid.clone()).or_default().push(Entry {
                kind: "polish".into(),
                version: doc.version,
                at,
                page: None,
                facts: Vec::new(),
                request: None,
                summary: format!("교정: {fix}"),
            });
        }
        if !block.starts_with("<figure") {
            ground.blocks.insert(
                bid,
                Ground {
                    verdict: verdict.into(),
                    facts,
                    why: w.why,
                    text: text_of(&block),
                },
            );
        }
        html.push_str(&block);
        html.push('\n');
    }
    doc.html = html;
    prov.log.push(LogVersion {
        version: doc.version,
        at,
        items: vec![LogItem {
            bid: String::new(),
            kind: "rewrite".into(),
            summary: format!(
                "{summary}: {}",
                counts
                    .iter()
                    .map(|(k, v)| format!("{k} {v}"))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ),
        }],
    });
    document::write(ws, &doc)?;
    provenance::save(ws, &doc.feature, &prov)?;
    crate::grounding::save(ws, &doc.feature, &ground)?;
    Ok(doc)
}

/// Rewrites the feature's document (the old manual as converted) from the code facts.
pub async fn from_code(
    d: &mwa_core::Dispatcher,
    ws: &Path,
    feature: &str,
    facts: &[Fact],
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<Doc, ManualError> {
    let doc = document::load(ws, feature)
        .ok_or_else(|| ManualError::Invalid(format!("기존 매뉴얼 문서가 없습니다: {feature}")))?;
    // Written from the old manual as imported, on top of the current document.
    let source = match doc.base {
        Some(b) if b != doc.version => document::version(ws, feature, b)
            .map(|d| d.html)
            .ok_or_else(|| ManualError::Invalid(format!("기존 매뉴얼 버전 {b}이 없습니다")))?,
        _ => doc.html.clone(),
    };
    let parts = sections(&source);
    let all_facts = fact_lines(facts);
    let scenes = crate::reference::load_scene_shots(ws, feature);
    let total = parts.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let calls = parts.iter().map(|section| {
        // The section's pictures as the product shows them now, with what changed on screen.
        let mut images = Vec::new();
        let mut shown = Vec::new();
        for b in section.iter().filter(|b| b.starts_with("<figure")) {
            let Some(s) = document::attr_of(b, "data-ref")
                .and_then(|r| scenes.as_ref()?.scenes.iter().find(|s| s.image == r))
            else {
                continue;
            };
            let Some(path) = crate::reference::current_picture(ws, feature, s) else {
                continue;
            };
            images.push(path);
            let changes = if s.changes.is_empty() {
                "(대조에서 찾은 화면 변화 없음)".to_string()
            } else {
                s.changes.join(" / ")
            };
            shown.push(format!(
                "- 첨부 그림 {}: {} — 화면 변화: {changes}",
                images.len(),
                s.what
            ));
        }
        let pictures = if shown.is_empty() {
            String::new()
        } else {
            format!(
                "\n\n## 이 절의 그림 (지금 화면, 첨부 순서)\n{}",
                shown.join("\n")
            )
        };
        let prompt = format!(
            "매뉴얼: {}\n\n## 기존 매뉴얼의 이 절\n{}{pictures}\n\n## 현재 코드의 사실\n{}",
            doc.title,
            section.join("\n"),
            all_facts
        );
        let done = &done;
        async move {
            let out = call_blocks(d, SECTION_SYSTEM, &prompt, &images).await;
            let k = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            progress(format!("코드로 다시 쓰는 중 {k}/{total}절"));
            out
        }
    });
    let results = futures::future::join_all(calls).await;
    let mut written: Vec<Written> = Vec::new();
    for (section, r) in parts.iter().zip(results) {
        let mut w = r?;
        // Pictures are never lost: any the model left out go back at the end of their section.
        let have: BTreeSet<String> = w.iter().filter_map(|x| bid_of(&x.html)).collect();
        for b in section.iter().filter(|b| b.starts_with("<figure")) {
            if let Some(id) = bid_of(b)
                && !have.contains(&id)
            {
                w.push(Written {
                    html: b.clone(),
                    basis: "none".into(),
                    facts: Vec::new(),
                    why: "기존 매뉴얼의 그림 자리".into(),
                    polish: None,
                });
            }
        }
        written.extend(w);
    }

    // Facts no section used: new parts of the product the old manual does not have.
    let used: BTreeSet<&str> = written
        .iter()
        .flat_map(|w| w.facts.iter().map(String::as_str))
        .collect();
    let unused: Vec<&Fact> = facts
        .iter()
        .filter(|f| !used.contains(f.id.as_str()))
        .collect();
    if !unused.is_empty() {
        progress(format!("아직 안 쓴 코드 사실 {}개 확인 중", unused.len()));
        let headings: Vec<String> = written
            .iter()
            .filter(|w| w.html.starts_with("<h"))
            .map(|w| text_of(&w.html))
            .collect();
        #[derive(Deserialize)]
        struct Group {
            after_heading: String,
            blocks: Vec<RawBlock>,
        }
        #[derive(Deserialize)]
        struct Raw {
            groups: Vec<Group>,
        }
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"groups": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "after_heading": {"type": "string"},
                    "blocks": blocks_schema()["properties"]["blocks"].clone()
                },
                "required": ["after_heading", "blocks"]
            }}},
            "required": ["groups"]
        });
        let prompt = format!(
            "매뉴얼: {}\n\n## 제목 목록\n{}\n\n## 아직 다루지 않은 코드 사실\n{}\n\n## 매뉴얼 말투 예시\n{}",
            doc.title,
            headings.join("\n"),
            unused
                .iter()
                .map(|f| format!("{}: {}", f.id, f.statement))
                .collect::<Vec<_>>()
                .join("\n"),
            written
                .iter()
                .take(12)
                .map(|w| w.html.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
        let out = d
            .call(
                mwa_protocol::Role::Writer,
                EXTRA_SYSTEM,
                &prompt,
                &[],
                Some(&schema),
            )
            .await
            .map_err(|e| ManualError::Invalid(format!("새 내용 쓰기 실패: {e}")))?;
        let raw: Raw = serde_json::from_value(out.response.json.unwrap_or_default())
            .map_err(|e| ManualError::Shape(format!("새 내용 쓰기: {e}")))?;
        for g in raw.groups {
            let key: String = g
                .after_heading
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let at = written.iter().position(|w| {
                w.html.starts_with("<h")
                    && text_of(&w.html)
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect::<String>()
                        == key
            });
            let Some(h) = at else { continue };
            let level = written[h].html.as_bytes().get(2).copied().unwrap_or(b'2');
            // End of that heading's part: the next heading of the same or a higher level.
            let end = written[h + 1..]
                .iter()
                .position(|w| {
                    w.html.starts_with("<h")
                        && w.html.as_bytes().get(2).copied().unwrap_or(b'9') <= level
                })
                .map_or(written.len(), |i| h + 1 + i);
            let new: Vec<Written> = g
                .blocks
                .into_iter()
                .filter(|b| !b.html.trim().is_empty())
                .map(|b| Written {
                    html: b.html,
                    basis: "new".into(),
                    facts: b.facts,
                    why: b.why,
                    polish: None,
                })
                .collect();
            written.splice(end..end, new);
        }
    }
    let changes: Vec<String> = scenes
        .iter()
        .flat_map(|s| s.scenes.iter())
        .filter(|s| crate::reference::current_picture(ws, feature, s).is_some())
        .flat_map(|s| {
            s.changes
                .iter()
                .map(move |c| format!("- {}: {c}", s.heading))
        })
        .collect();
    let n = polish(
        d,
        &doc.title,
        facts,
        &mut written,
        doc.version + 1,
        &changes,
        progress,
    )
    .await?;
    let known: BTreeSet<&str> = facts.iter().map(|f| f.id.as_str()).collect();
    store(
        ws,
        doc,
        written,
        &known,
        &format!("코드로 다시 씀 (교정 {n}곳)"),
    )
}

const PICTURE_SYSTEM: &str = concat!(
    "You check one part of a Korean user manual against pictures of the product's current \
screen that belong to it (attached in order). Add what a user sees there and the text lacks: \
fields, columns, buttons, tabs, labels and messages shown (basis screen). Replace a '확인 필요' \
panel the pictures answer with the answer (basis screen). Correct what the pictures show \
differently (basis changed). Keep everything else as it is, with its data-bid, and keep the \
<figure> blocks unchanged. Values in the pictures (names, numbers, dates, hosts) are sample \
data: never write them. ",
    user_rule!(),
    "Return the whole part as blocks in order, one top-level HTML block each, in the part's own \
style. basis: screen or changed for what you added or corrected, kept for blocks left as they \
were. facts: ids of code facts a block also rests on; why: one Korean sentence on what you added \
or corrected."
);

/// After a written manual's picture slots are shot (D-047): each part with a picture is read
/// against it, so what the screen shows and the code facts did not say gets in. Returns the new
/// version, or none when no part has a picture.
pub async fn from_pictures(
    d: &mwa_core::Dispatcher,
    ws: &Path,
    feature: &str,
    facts: &[Fact],
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<Option<Doc>, ManualError> {
    let doc = document::load(ws, feature)
        .ok_or_else(|| ManualError::Invalid(format!("매뉴얼 문서가 없습니다: {feature}")))?;
    let all = blocks(&doc.html);
    let heading = |b: &str| ["<h2", "<h3", "<h4"].iter().any(|h| b.starts_with(h));
    // A part: a heading and the blocks up to the next heading; never overlapping.
    let mut parts: Vec<(usize, usize, Vec<std::path::PathBuf>)> = Vec::new();
    for (i, b) in all.iter().enumerate() {
        if !b.starts_with("<figure") {
            continue;
        }
        let Some(shot) = document::attr_of(b, "data-shot") else {
            continue;
        };
        let Some(start) = (0..i).rev().find(|&k| heading(&all[k])) else {
            continue;
        };
        let end = (i + 1..all.len())
            .find(|&k| heading(&all[k]))
            .unwrap_or(all.len());
        let path = ws.join(shot);
        match parts.iter_mut().find(|p| p.0 == start) {
            Some(p) => p.2.push(path),
            None => parts.push((start, end, vec![path])),
        }
    }
    if parts.is_empty() {
        return Ok(None);
    }
    progress(format!("찍은 그림 {}곳을 보고 글 보완 중", parts.len()));
    let all_facts = fact_lines(facts);
    let calls = parts.iter().map(|(start, end, images)| {
        let prompt = format!(
            "매뉴얼: {}\n\n## 이 부분\n{}\n\n## 코드 사실\n{all_facts}",
            doc.title,
            all[*start..*end].join("\n")
        );
        async move { call_blocks(d, PICTURE_SYSTEM, &prompt, images).await }
    });
    let results = futures::future::join_all(calls).await;
    let version = doc.version + 1;
    let old: BTreeMap<String, String> = all
        .iter()
        .filter_map(|b| Some((bid_of(b)?, text_of(b))))
        .collect();
    let known: BTreeSet<&str> = facts.iter().map(|f| f.id.as_str()).collect();
    let mut ground = crate::grounding::load(ws, feature);
    let mut changes = Vec::new();
    let mut out: Vec<String> = Vec::new();
    let mut at = 0;
    let mut n = 0;
    for ((start, end, _), r) in parts.iter().zip(results) {
        out.extend(all[at..*start].iter().cloned());
        at = *end;
        // A part that could not be read again stays as it was.
        let Ok(written) = r else {
            out.extend(all[*start..*end].iter().cloned());
            continue;
        };
        for w in split(written) {
            let bid = bid_of(&w.html).unwrap_or_else(|| {
                n += 1;
                format!("s{version}-{n}")
            });
            let block = set_bid(&w.html, &bid);
            let text = text_of(&block);
            if old.get(&bid) != Some(&text) && !block.starts_with("<figure") {
                let facts: Vec<String> = w
                    .facts
                    .into_iter()
                    .filter(|f| known.contains(f.as_str()))
                    .collect();
                changes.push(crate::provenance::Change {
                    bids: vec![bid.clone()],
                    kind: "screen".into(),
                    summary: format!("지금 화면을 보고 보완: {}", w.why),
                    request: None,
                    facts: facts.clone(),
                    inherit: Vec::new(),
                });
                ground.blocks.insert(
                    bid,
                    crate::grounding::Ground {
                        verdict: "supported".into(),
                        facts,
                        why: format!("지금 화면 그림에서 확인: {}", w.why),
                        text,
                    },
                );
            }
            out.push(block);
        }
    }
    out.extend(all[at..].iter().cloned());
    if changes.is_empty() {
        return Ok(None);
    }
    let doc = document::save(ws, feature, &out.join("\n"), &changes)?;
    crate::grounding::save(ws, feature, &ground)?;
    Ok(Some(doc))
}

/// An example manual as the writer needs it: the elements and their order, without ids, picture
/// sources or styles (they only cost length). Pictures become slots, as the new manual writes them.
fn skeleton(html: &str) -> String {
    const KEEP: &[&str] = &["data-panel", "data-status", "colspan", "rowspan"];
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('>') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let tag = &rest[i..i + j + 1];
        rest = &rest[i + j + 1..];
        if tag.starts_with("</") || tag.starts_with("<!") {
            out.push_str(tag);
            continue;
        }
        let name: String = tag[1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if name == "figure" {
            out.push_str("<figure data-want=\"화면 — 상태 — 자를 부분\">");
            continue;
        }
        out.push('<');
        out.push_str(&name);
        for a in KEEP {
            if let Some(v) = document::attr_of(tag, a) {
                out.push_str(&format!(" {a}=\"{v}\""));
            }
        }
        out.push('>');
    }
    out.push_str(rest);
    out
}

/// Writes a manual for a feature that has none, after the project's finished manuals.
pub async fn new_feature(
    d: &mwa_core::Dispatcher,
    ws: &Path,
    feature: &str,
    title: &str,
    facts: &[Fact],
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<Doc, ManualError> {
    // Examples: other features' documents, longest edited first (they show the house format).
    let mut examples: Vec<Doc> = std::fs::read_dir(ws.join("manual/docs"))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| document::load(ws, &e.file_name().to_string_lossy()))
                .filter(|d| d.feature != feature && !d.html.is_empty())
                .collect()
        })
        .unwrap_or_default();
    // Manuals rewritten from an old manual carry the house format best; then the fullest.
    examples.sort_by_key(|d| std::cmp::Reverse((d.base.is_some(), d.html.len())));
    let mut shown: Vec<String> = examples
        .iter()
        .take(2)
        .map(|e| {
            format!(
                "### {}\n{}",
                e.title,
                skeleton(&e.html).chars().take(16000).collect::<String>()
            )
        })
        .collect();
    // The very first manual: the format learned from the settings' sample, else the outline in
    // the instructions.
    if shown.is_empty()
        && let Ok(t) = std::fs::read_to_string(ws.join(crate::pipeline::TEMPLATE_FILE))
    {
        shown.push(format!("### 프로젝트 양식 (절 순서와 문체 규칙)\n{t}"));
    }
    if shown.is_empty() {
        shown.push("(본보기 없음 — 지시에 적힌 목차를 따르세요)".into());
    }
    let examples = shown.join("\n\n");
    // Each part sees one example: enough for the format, and it is sent once per part.
    let example = shown.first().cloned().unwrap_or_default();
    let all_facts = fact_lines(facts);

    // 1. The outline: parts, the facts each covers, and where pictures go.
    progress("목차와 그림 자리 정하는 중".into());
    #[derive(Deserialize)]
    struct Part {
        heading: String,
        level: u8,
        #[serde(default)]
        facts: Vec<String>,
        #[serde(default)]
        note: String,
        #[serde(default)]
        picture: String,
    }
    #[derive(Deserialize)]
    struct Outline {
        parts: Vec<Part>,
    }
    let schema = serde_json::json!({
        "type": "object",
        "properties": {"parts": {"type": "array", "items": {
            "type": "object",
            "properties": {
                "heading": {"type": "string"},
                "level": {"type": "integer"},
                "facts": {"type": "array", "items": {"type": "string"}},
                "note": {"type": "string"},
                "picture": {"type": "string"}
            },
            "required": ["heading", "level", "facts", "note", "picture"]
        }}},
        "required": ["parts"]
    });
    let out = d
        .call(
            mwa_protocol::Role::Writer,
            OUTLINE_SYSTEM,
            &format!(
                "기능: {title}\n\n## 본보기 매뉴얼\n{examples}\n\n## 이 기능의 현재 코드 사실\n{all_facts}"
            ),
            &[],
            Some(&schema),
        )
        .await
        .map_err(|e| ManualError::Invalid(format!("목차 정하기 실패: {e}")))?;
    let outline: Outline = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("목차: {e}")))?;
    let toc: Vec<String> = outline
        .parts
        .iter()
        .map(|p| {
            format!(
                "{}- {} (h{})",
                "  ".repeat(usize::from(p.level.saturating_sub(2))),
                p.heading,
                p.level
            )
        })
        .collect();
    let toc = toc.join("\n");

    // 2. Each part from its own facts, in parallel.
    let total = outline.parts.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let calls = outline.parts.iter().map(|p| {
        let mine: Vec<Fact> = facts
            .iter()
            .filter(|f| p.facts.contains(&f.id))
            .cloned()
            .collect();
        let prompt = format!(
            "기능: {title}\n\n## 본보기 매뉴얼\n{example}\n\n## 전체 목차\n{toc}\n\n## 쓸 부분\n제목: {} (h{})\n쓸 내용: {}\n그림: {}\n\n## 이 부분의 코드 사실\n{}\n\n## 참고: 이 기능의 모든 코드 사실\n{all_facts}",
            p.heading,
            p.level,
            p.note,
            if p.picture.is_empty() {
                "(없음)"
            } else {
                &p.picture
            },
            fact_lines(&mine)
        );
        let done = &done;
        async move {
            let out = call_blocks(d, PART_SYSTEM, &prompt, &[]).await;
            let k = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            progress(format!("새로 쓰는 중 {k}/{total}"));
            out
        }
    });
    let mut written = Vec::new();
    for r in futures::future::join_all(calls).await {
        written.extend(r?);
    }
    // Writing again keeps the earlier document as a version.
    let version = document::load(ws, feature).map_or(0, |d| d.version);
    let n = polish(d, title, facts, &mut written, version + 1, &[], progress).await?;
    let doc = Doc {
        feature: feature.to_string(),
        title: title.to_string(),
        origin: "generated".into(),
        reference: None,
        html: String::new(),
        version,
        updated_at: 0,
        base: None,
    };
    let known: BTreeSet<&str> = facts.iter().map(|f| f.id.as_str()).collect();
    store(
        ws,
        doc,
        written,
        &known,
        &format!("본보기 매뉴얼을 따라 코드로 새로 씀 (교정 {n}곳)"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_keep_only_structure() {
        assert_eq!(
            skeleton(
                "<h2 data-bid=\"b1\" style=\"x\">권한</h2><div data-panel=\"note\" data-bid=\"b2\"><p>가</p></div><figure data-bid=\"b3\" data-ref=\"img/a.png\" data-width=\"60\"><figcaption>목록</figcaption></figure><td colspan=\"2\">나</td>"
            ),
            "<h2>권한</h2><div data-panel=\"note\"><p>가</p></div><figure data-want=\"화면 — 상태 — 자를 부분\"><figcaption>목록</figcaption></figure><td colspan=\"2\">나</td>"
        );
    }

    #[test]
    fn one_block_per_element() {
        let w = Written {
            html:
                "<h3>등록</h3><p>가</p><figure data-want=\"x\"><figcaption></figcaption></figure>"
                    .into(),
            basis: "code".into(),
            facts: vec!["F1".into()],
            why: "이유".into(),
            polish: None,
        };
        let parts = split(vec![w]);
        assert_eq!(parts.len(), 3);
        assert!(parts[2].html.starts_with("<figure"));
        assert_eq!(parts[1].facts, ["F1"]);
    }

    #[test]
    fn splits_at_sections() {
        let s = sections(
            "<div data-panel=\"note\"><p>소개</p></div><ul><li>목차</li></ul><h2>권한</h2><p>가</p><h3>세부</h3><p>나</p><h2>용어</h2><table></table>",
        );
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].len(), 2);
        assert_eq!(s[1].len(), 4, "h3 stays inside its h2 section");
        assert!(s[2][0].starts_with("<h2>용어"));
    }
}
