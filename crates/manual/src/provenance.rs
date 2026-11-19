//! Where each part of an editable manual came from (D-037): every top-level block carries a
//! `data-bid`, and each block keeps a history (old manual page, code facts, a request, a person's
//! edit). Saves add a change log per version.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ManualError;
use crate::document::{blocks, text_of};

/// One step in a block's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// `pdf`, `md`, `draft`, `proposal`, `request`, `edit`.
    pub kind: String,
    /// Document version the step was saved in (0 = as imported).
    pub version: u32,
    pub at: u64,
    /// Old manual page it was taken from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    /// Code fact ids it rests on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<String>,
    /// What the person asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogItem {
    pub bid: String,
    pub kind: String,
    pub summary: String,
}

/// What changed in one saved version, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogVersion {
    pub version: u32,
    pub at: u64,
    pub items: Vec<LogItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub blocks: BTreeMap<String, Vec<Entry>>,
    pub log: Vec<LogVersion>,
}

/// A change the editor knows the reason for (accepted proposal, applied request).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Change {
    /// Blocks it produced.
    pub bids: Vec<String>,
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub request: Option<String>,
    #[serde(default)]
    pub facts: Vec<String>,
    /// Blocks it replaced: their history carries over.
    #[serde(default)]
    pub inherit: Vec<String>,
}

fn path(ws: &Path, feature: &str) -> PathBuf {
    ws.join("manual/docs").join(feature).join("provenance.json")
}

pub fn load(ws: &Path, feature: &str) -> Provenance {
    std::fs::read_to_string(path(ws, feature))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(ws: &Path, feature: &str, p: &Provenance) -> Result<(), ManualError> {
    let file = path(ws, feature);
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d).map_err(|e| ManualError::Io(d.to_path_buf(), e))?;
    }
    std::fs::write(&file, serde_json::to_string_pretty(p).expect("serializes"))
        .map_err(|e| ManualError::Io(file, e))
}

/// The `data-bid` of a block's opening tag.
pub fn bid_of(block: &str) -> Option<String> {
    let open = &block[..block.find('>').unwrap_or(block.len())];
    let i = open.find("data-bid=\"")? + 10;
    let j = open[i..].find('"')? + i;
    Some(open[i..j].to_string())
}

/// Gives every top-level block without a `data-bid` one (`<prefix><n>`); returns the HTML and
/// the ids in block order.
pub fn with_bids(html: &str, prefix: &str) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut ids = Vec::new();
    for (n, b) in blocks(html).into_iter().enumerate() {
        match bid_of(&b) {
            Some(id) => {
                ids.push(id);
                out.push_str(&b);
            }
            None => {
                let id = format!("{prefix}{n}");
                out.push_str(&set_bid(&b, &id));
                ids.push(id);
            }
        }
        out.push('\n');
    }
    (out, ids)
}

/// The block with `data-bid` set on its opening tag (replacing any it had).
pub fn set_bid(block: &str, id: &str) -> String {
    let name_end = block
        .char_indices()
        .skip(1)
        .find(|(_, c)| !c.is_ascii_alphanumeric())
        .map_or(block.len(), |(i, _)| i);
    let rest = &block[name_end..];
    let rest = match bid_of(block) {
        Some(old) => rest.replacen(&format!(" data-bid=\"{old}\""), "", 1),
        None => rest.to_string(),
    };
    format!("{} data-bid=\"{id}\"{rest}", &block[..name_end])
}

/// History for freshly imported blocks, as version `version`. Earlier history stays: an old
/// manual imported again is a new version on top of the document's past (D-047).
pub fn start(
    ws: &Path,
    feature: &str,
    version: u32,
    origin: &[(String, Entry)],
) -> Result<(), ManualError> {
    let mut p = load(ws, feature);
    let at = origin.first().map_or(0, |(_, e)| e.at);
    for (bid, e) in origin {
        p.blocks.entry(bid.clone()).or_default().push(e.clone());
    }
    p.log.push(LogVersion {
        version,
        at,
        items: vec![LogItem {
            bid: origin.first().map(|(b, _)| b.clone()).unwrap_or_default(),
            kind: origin.first().map_or("pdf".into(), |(_, e)| e.kind.clone()),
            summary: format!("문서를 만들었습니다 (블록 {}개)", origin.len()),
        }],
    });
    save(ws, feature, &p)
}

/// Records why the blocks of a newly saved version changed: the editor's known changes, then
/// person edits found by comparing block texts.
pub fn record(
    ws: &Path,
    feature: &str,
    old_html: &str,
    new_html: &str,
    version: u32,
    at: u64,
    changes: &[Change],
) -> Result<(), ManualError> {
    let mut p = load(ws, feature);
    let texts = |html: &str| -> BTreeMap<String, String> {
        blocks(html)
            .into_iter()
            .filter_map(|b| Some((bid_of(&b)?, text_of(&b))))
            .collect()
    };
    let (old, new) = (texts(old_html), texts(new_html));
    let mut items = Vec::new();
    let mut covered = BTreeSet::new();
    for c in changes {
        let carried: Vec<Entry> = c
            .inherit
            .iter()
            .flat_map(|b| p.blocks.get(b).cloned().unwrap_or_default())
            .collect();
        let entry = Entry {
            kind: c.kind.clone(),
            version,
            at,
            page: None,
            facts: c.facts.clone(),
            request: c.request.clone(),
            summary: c.summary.clone(),
        };
        for bid in c.bids.iter().filter(|b| new.contains_key(*b)) {
            let h = p.blocks.entry(bid.clone()).or_default();
            if h.is_empty() {
                h.extend(carried.iter().cloned());
            }
            h.push(entry.clone());
            covered.insert(bid.clone());
        }
        if let Some(bid) = c.bids.iter().find(|b| new.contains_key(*b)) {
            items.push(LogItem {
                bid: bid.clone(),
                kind: c.kind.clone(),
                summary: c.summary.clone(),
            });
        }
        covered.extend(c.inherit.iter().cloned());
    }
    for (bid, text) in &new {
        if covered.contains(bid) {
            continue;
        }
        let summary = match old.get(bid) {
            None => "사람이 새로 씀",
            Some(t) if t != text => "사람이 고침",
            Some(_) => continue,
        };
        p.blocks.entry(bid.clone()).or_default().push(Entry {
            kind: "edit".into(),
            version,
            at,
            page: None,
            facts: Vec::new(),
            request: None,
            summary: summary.into(),
        });
        items.push(LogItem {
            bid: bid.clone(),
            kind: "edit".into(),
            summary: format!("{summary}: {}", short(text)),
        });
    }
    for (bid, text) in &old {
        if !new.contains_key(bid) && !covered.contains(bid) {
            items.push(LogItem {
                bid: bid.clone(),
                kind: "removed".into(),
                summary: format!("지움: {}", short(text)),
            });
        }
    }
    p.log.push(LogVersion { version, at, items });
    save(ws, feature, &p)
}

fn short(t: &str) -> String {
    let s: String = t.chars().take(40).collect();
    if t.chars().count() > 40 {
        format!("{s}…")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_history() {
        let (html, ids) = with_bids(
            "<h2>제목</h2><p>가</p><table><tr><td>칸</td></tr></table>",
            "b",
        );
        assert_eq!(ids, ["b0", "b1", "b2"]);
        assert!(html.contains("<h2 data-bid=\"b0\">"));
        assert_eq!(with_bids(&html, "x").1, ids, "existing ids stay");

        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let e = |page| Entry {
            kind: "pdf".into(),
            version: 0,
            at: 1,
            page: Some(page),
            facts: vec![],
            request: None,
            summary: "옮김".into(),
        };
        start(
            ws,
            "f",
            0,
            &[
                ("b0".into(), e(1)),
                ("b1".into(), e(1)),
                ("b2".into(), e(2)),
            ],
        )
        .unwrap();

        // b1 edited by hand, b2 replaced by a request (new id n1), b0 untouched.
        let next = "<h2 data-bid=\"b0\">제목</h2><p data-bid=\"b1\">가나</p><ul data-bid=\"n1\"><li>칸</li></ul>";
        let c = Change {
            bids: vec!["n1".into()],
            kind: "request".into(),
            summary: "표를 목록으로".into(),
            request: Some("목록으로 바꿔 줘".into()),
            facts: vec![],
            inherit: vec!["b2".into()],
        };
        record(ws, "f", &html, next, 1, 2, &[c]).unwrap();
        let p = load(ws, "f");
        let n1 = &p.blocks["n1"];
        assert_eq!(
            (n1[0].kind.as_str(), n1[0].page),
            ("pdf", Some(2)),
            "history carried over"
        );
        assert_eq!(n1[1].kind, "request");
        assert_eq!(p.blocks["b1"].last().unwrap().summary, "사람이 고침");
        assert_eq!(p.blocks["b0"].len(), 1);
        let log = p.log.last().unwrap();
        assert_eq!(log.version, 1);
        assert_eq!(
            log.items.len(),
            2,
            "request + hand edit; the replaced table is not 'removed'"
        );
    }
}
