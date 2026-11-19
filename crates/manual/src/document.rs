//! Editable manual documents (D-032): the old manual (PDF) turned into Confluence-style HTML
//! that people edit, with pictures replaced by current screens where they were reproduced.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ManualError;
use crate::reference;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    pub feature: String,
    pub title: String,
    /// Where it started: `pdf:<reference id>` or `generated`.
    pub origin: String,
    /// Reference doc whose pictures `data-ref` points at.
    #[serde(default)]
    pub reference: Option<String>,
    /// Editor HTML (elements listed in `SYSTEM`).
    pub html: String,
    /// Saved version number (0 = the first one).
    pub version: u32,
    pub updated_at: u64,
    /// The version that is the old manual as imported, what rewrites start from and the
    /// document is compared with (D-047).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<u32>,
}

fn dir(ws: &Path, feature: &str) -> PathBuf {
    ws.join("manual/docs").join(feature)
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> ManualError + '_ {
    move |e| ManualError::Io(path.to_path_buf(), e)
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn load(ws: &Path, feature: &str) -> Option<Doc> {
    let text = std::fs::read_to_string(dir(ws, feature).join("doc.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// A document made before block ids (D-037): gives its blocks ids and an origin entry, without
/// a new version. Does nothing once it has them.
pub fn migrate(ws: &Path, feature: &str) -> Result<Option<Doc>, ManualError> {
    let Some(mut doc) = load(ws, feature) else {
        return Ok(None);
    };
    if doc.html.contains("data-bid=") {
        return Ok(Some(doc));
    }
    let (html, ids) = crate::provenance::with_bids(&doc.html, "b");
    let (kind, summary) = match doc.origin.split(':').next() {
        Some("pdf") => (
            "pdf",
            "기존 매뉴얼(PDF)에서 옮김 (쪽 정보는 기록 이전에 옮겨 없음)",
        ),
        Some("md") => ("md", "기존 매뉴얼(MD)에서 옮김"),
        _ => ("draft", "코드로 새로 씀"),
    };
    let origin: Vec<_> = ids
        .into_iter()
        .map(|b| {
            (
                b,
                origin_entry(kind, None, doc.version, doc.updated_at, summary.into()),
            )
        })
        .collect();
    doc.html = html;
    write(ws, &doc)?;
    if crate::provenance::load(ws, feature).blocks.is_empty() {
        crate::provenance::start(ws, feature, doc.version, &origin)?;
    }
    Ok(Some(doc))
}

/// Saves the edited HTML as the next version; the previous one stays as `v<n>.json`.
pub fn save(
    ws: &Path,
    feature: &str,
    html: &str,
    changes: &[crate::provenance::Change],
) -> Result<Doc, ManualError> {
    // Without a document yet, this starts one from the generated manual.
    let mut doc = match load(ws, feature) {
        Some(old) => {
            let d = dir(ws, feature);
            let path = d.join(format!("v{}.json", old.version));
            std::fs::write(
                &path,
                serde_json::to_string_pretty(&old).expect("serializes"),
            )
            .map_err(io(&path))?;
            old
        }
        None => Doc {
            feature: feature.to_string(),
            title: feature.to_string(),
            origin: "generated".into(),
            reference: None,
            html: String::new(),
            version: 0,
            updated_at: 0,
            base: None,
        },
    };
    let started = doc.updated_at == 0;
    let old_html = std::mem::take(&mut doc.html);
    doc.version += 1;
    doc.updated_at = now();
    let (html, ids) = crate::provenance::with_bids(html, &format!("e{}-", doc.version));
    doc.html = html;
    write(ws, &doc)?;
    if started {
        let origin: Vec<_> = ids
            .into_iter()
            .map(|b| {
                (
                    b,
                    origin_entry(
                        "draft",
                        None,
                        doc.version,
                        doc.updated_at,
                        "코드로 새로 씀".into(),
                    ),
                )
            })
            .collect();
        crate::provenance::start(ws, feature, doc.version, &origin)?;
    } else {
        crate::provenance::record(
            ws,
            feature,
            &old_html,
            &doc.html,
            doc.version,
            doc.updated_at,
            changes,
        )?;
    }
    Ok(doc)
}

pub(crate) fn write(ws: &Path, doc: &Doc) -> Result<(), ManualError> {
    let d = dir(ws, &doc.feature);
    std::fs::create_dir_all(&d).map_err(io(&d))?;
    let path = d.join("doc.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(doc).expect("serializes"),
    )
    .map_err(io(&path))
}

/// Stores an uploaded picture and returns its file name (content-addressed, so re-uploads of
/// the same picture share one file).
pub fn save_asset(
    ws: &Path,
    feature: &str,
    name: &str,
    data: &[u8],
) -> Result<String, ManualError> {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| ["png", "jpg", "jpeg", "gif", "webp"].contains(&e.as_str()))
        .ok_or_else(|| {
            ManualError::Invalid("그림 파일(png, jpg, gif, webp)만 올릴 수 있습니다".into())
        })?;
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    data.hash(&mut h);
    let file = format!("{:016x}.{ext}", h.finish());
    let d = dir(ws, feature).join("assets");
    std::fs::create_dir_all(&d).map_err(io(&d))?;
    let path = d.join(&file);
    std::fs::write(&path, data).map_err(io(&path))?;
    Ok(file)
}

pub fn asset(ws: &Path, feature: &str, file: &str) -> Option<PathBuf> {
    if file.contains('/') || file.contains("..") {
        return None;
    }
    let p = dir(ws, feature).join("assets").join(file);
    p.is_file().then_some(p)
}

const SYSTEM: &str = "You convert one page of an old Korean user manual (PDF) into HTML for a \
Confluence-like editor. You get the page image, the page's text layer, and the list of pictures \
cropped from this page.

Rules:
- Copy the Korean text from the text layer verbatim. Join lines the page width broke in the \
middle of a sentence; where a sentence ends and the next one starts on a new line inside the \
same block, keep the line break as <br>. \
Do not summarize, translate or add text. Drop page headers, footers and page numbers.
- Use only these elements: <h1> <h2> <h3> <p> <strong> <em> <u> <s> <code> <ul> <ol> <li> \
<table> <tr> <th> <td> <br> <hr>, panels <div data-panel=\"info|note|warning|success\"> with <p> \
inside, chosen by the box colour: blue = info, purple/lavender = note, yellow/orange = warning, \
green = success. Status labels <span data-status=\"grey|blue|green|yellow|red\">텍스트</span> \
(small colored badges such as 활성, 비활성), and pictures \
<figure data-ref=\"FILE\"><figcaption>caption</figcaption></figure>.
- Put every listed picture exactly once, where it appears on the page, with FILE from the list. \
A caption is only a short line right under the picture that describes it; a heading or a \
paragraph that starts the next part is not a caption. Use an empty <figcaption></figcaption> \
when there is none. Never invent pictures.
- Headings: the page title at the top of page 1 is NOT output (the editor shows it), but the \
outline (the list of section names under it) IS output as a nested <ul>. Section \
levels follow the outline list on page 1: its top-level entries are <h2>, the next level <h3>, \
the next <h4>; the same entry is always the same level, whatever its size on the page. A thin \
grey line across the page is <hr>. Tables keep their rows and columns; the first row is <th> when it is a header.
- Text that continues from the previous page starts with a <p> as usual.
- Tables cut by a page break: when the previous page ended inside a table and this page starts \
with its remaining rows, output those rows as a <table> with exactly the same number of columns \
and NO header row (do not repeat the header). When a table runs past the bottom of this page, \
output the rows you see as they are. The pieces are joined into one table afterwards.";

/// Converts an imported old manual into an editable document (one worker call per page).
pub async fn import(
    d: &mwa_core::Dispatcher,
    ws: &Path,
    feature: &str,
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<Doc, ManualError> {
    let rdoc = reference::for_feature(ws, feature).ok_or_else(|| {
        ManualError::Invalid(format!(
            "이 기능에 짝지은 기존 매뉴얼이 없습니다: {feature}"
        ))
    })?;
    let text = std::fs::read_to_string(reference::doc_dir(ws, &rdoc.id).join("text.txt"))
        .unwrap_or_default();
    // Versions only go up: importing again stacks on the document's past.
    let next = load(ws, feature).map_or(0, |d| d.version + 1);
    if reference::is_markdown(&rdoc.source) {
        // Markdown is already structured text: convert it, no model needed.
        progress("마크다운을 문서로 옮기는 중".into());
        let doc = Doc {
            feature: feature.to_string(),
            title: rdoc.title.clone(),
            origin: format!("md:{}", rdoc.id),
            reference: Some(rdoc.id.clone()),
            html: String::new(),
            version: next,
            updated_at: now(),
            base: Some(next),
        };
        let (html, ids) = crate::provenance::with_bids(&markdown_html(&text), "m");
        let origin: Vec<_> = ids
            .into_iter()
            .map(|b| {
                (
                    b,
                    origin_entry(
                        "md",
                        None,
                        next,
                        doc.updated_at,
                        "기존 매뉴얼(MD)에서 옮김".into(),
                    ),
                )
            })
            .collect();
        let doc = Doc { html, ..doc };
        keep_previous(ws, feature)?;
        write(ws, &doc)?;
        crate::provenance::start(ws, feature, next, &origin)?;
        return Ok(doc);
    }
    let pages: Vec<&str> = text.split('\u{c}').collect();
    let schema = serde_json::json!({
        "type": "object",
        "properties": {"html": {"type": "string"}},
        "required": ["html"]
    });
    let done = std::sync::atomic::AtomicUsize::new(0);
    let total = rdoc.pages as usize;
    let calls = (1..=rdoc.pages).map(|n| {
        let pics: Vec<String> = rdoc
            .images
            .iter()
            .filter(|i| i.page == n)
            .map(|i| {
                format!(
                    "- FILE={} (제목: {} / 캡션: {})",
                    i.file, i.heading, i.caption
                )
            })
            .collect();
        let page_text = pages.get(n as usize - 1).copied().unwrap_or("");
        // The end of the previous page, so a table cut by the page break is recognised.
        let prev_tail: String = pages
            .get((n as usize).wrapping_sub(2))
            .map(|p| {
                let lines: Vec<&str> = p.lines().filter(|l| !l.trim().is_empty()).collect();
                lines[lines.len().saturating_sub(6)..].join("\n")
            })
            .unwrap_or_default();
        let prompt = format!(
            "매뉴얼: {}\n{}쪽 / {}쪽\n\n## 앞 쪽의 끝부분 (참고만, 옮기지 않음)\n{}\n\n## 이 쪽의 그림\n{}\n\n## 이 쪽의 글자\n{}",
            rdoc.title,
            n,
            total,
            if prev_tail.is_empty() { "(없음)" } else { &prev_tail },
            if pics.is_empty() {
                "(없음)".to_string()
            } else {
                pics.join("\n")
            },
            page_text
        );
        let (schema, done, id) = (&schema, &done, &rdoc.id);
        async move {
            let image = reference::page(ws, id, n)?;
            let out = d
                .call(
                    mwa_protocol::Role::Worker,
                    SYSTEM,
                    &prompt,
                    &[image],
                    Some(schema),
                )
                .await;
            let k = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            progress(format!("기존 매뉴얼 옮기는 중 {k}/{total}쪽"));
            let v = out
                .map_err(|e| ManualError::Invalid(format!("{n}쪽 변환 실패: {e}")))?
                .response
                .json
                .unwrap_or_default();
            Ok::<String, ManualError>(
                v.get("html")
                    .and_then(|h| h.as_str())
                    .unwrap_or("")
                    .to_string(),
            )
        }
    });
    let parts = futures::future::join_all(calls).await;
    let mut paged: Vec<(u32, u32, String)> = Vec::new();
    for (i, p) in parts.into_iter().enumerate() {
        let page = i as u32 + 1;
        for b in blocks(&p?) {
            paged.push((page, page, b));
        }
    }
    let paged = merge_split_tables(paged);
    // Pages are converted one by one, so heading levels can disagree; the outline on page 1 says
    // which level each section is.
    let mut depths = pdf_outline_depths(&text);
    if depths.len() < 3 {
        depths = paged
            .iter()
            .find(|(_, _, b)| b.starts_with("<ul"))
            .map(|(_, _, b)| outline_depths(b))
            .unwrap_or_default();
    }
    let paged: Vec<_> = if depths.len() >= 3 {
        paged
            .into_iter()
            .map(|(f, l, b)| (f, l, set_heading_level(&b, &depths)))
            .collect()
    } else {
        paged
    };
    let at = now();
    let mut html = String::new();
    let mut origin = Vec::new();
    for (k, (first, last, b)) in paged.into_iter().enumerate() {
        let bid = format!("b{k}");
        html.push_str(&crate::provenance::set_bid(&b, &bid));
        html.push('\n');
        let summary = if first == last {
            format!("기존 매뉴얼 {first}쪽에서 옮김")
        } else {
            format!("기존 매뉴얼 {first}~{last}쪽에서 옮김 (쪽에 걸쳐 잘린 표를 합침)")
        };
        origin.push((bid, origin_entry("pdf", Some(first), next, at, summary)));
    }
    let doc = Doc {
        feature: feature.to_string(),
        title: rdoc.title.clone(),
        origin: format!("pdf:{}", rdoc.id),
        reference: Some(rdoc.id.clone()),
        html,
        version: next,
        updated_at: at,
        base: Some(next),
    };
    keep_previous(ws, feature)?;
    write(ws, &doc)?;
    crate::provenance::start(ws, feature, next, &origin)?;
    Ok(doc)
}

fn origin_entry(
    kind: &str,
    page: Option<u32>,
    version: u32,
    at: u64,
    summary: String,
) -> crate::provenance::Entry {
    crate::provenance::Entry {
        kind: kind.into(),
        version,
        at,
        page,
        facts: Vec::new(),
        request: None,
        summary,
    }
}

fn squash(t: &str) -> String {
    t.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The outline on page 1 of a Confluence PDF export, read from the layout text: each entry's
/// depth comes from its indentation (one step ≈ two spaces).
fn pdf_outline_depths(text: &str) -> std::collections::HashMap<String, usize> {
    let first = text.split('\u{c}').next().unwrap_or("");
    let mut entries: Vec<(usize, String)> = Vec::new();
    for line in first.lines().skip(1) {
        let t = line.trim();
        let entry = !t.is_empty()
            && t.chars().count() <= 40
            && !t.ends_with('.')
            && !t.ends_with('다')
            && !t.contains("   ");
        if !entry {
            // The outline is one run of short lines; stop at the first gap after it started.
            if !entries.is_empty() && t.is_empty() {
                break;
            }
            continue;
        }
        let key = squash(t);
        if entries.iter().any(|(_, k)| *k == key) {
            break;
        }
        entries.push((line.len() - line.trim_start().len(), key));
    }
    let base = entries.iter().map(|(i, _)| *i).min().unwrap_or(0);
    entries
        .into_iter()
        .map(|(indent, key)| (key, (indent - base).div_ceil(2)))
        .collect()
}

/// Entries of an outline list (`<ul>` of section names) with their nesting depth (0 = top).
fn outline_depths(list: &str) -> std::collections::HashMap<String, usize> {
    let mut out = std::collections::HashMap::new();
    let mut depth = 0usize;
    let mut text: Option<String> = None;
    let mut i = 0;
    let flush = |text: &mut Option<String>,
                 depth: usize,
                 out: &mut std::collections::HashMap<String, usize>| {
        if let Some(t) = text.take() {
            let t = squash(&t);
            if !t.is_empty() {
                out.entry(t).or_insert(depth.saturating_sub(1));
            }
        }
    };
    while i < list.len() {
        if list[i..].starts_with('<') {
            let end = list[i..].find('>').map_or(list.len(), |e| i + e + 1);
            let tag = &list[i..end];
            if tag.starts_with("<ul") || tag.starts_with("<ol") {
                flush(&mut text, depth, &mut out);
                depth += 1;
            } else if tag.starts_with("</ul") || tag.starts_with("</ol") {
                flush(&mut text, depth, &mut out);
                depth = depth.saturating_sub(1);
            } else if tag.starts_with("<li") {
                flush(&mut text, depth, &mut out);
                text = Some(String::new());
            } else if tag.starts_with("</li") {
                flush(&mut text, depth, &mut out);
            }
            i = end;
        } else {
            let next = list[i..].find('<').map_or(list.len(), |n| i + n);
            if let Some(t) = text.as_mut() {
                t.push_str(&list[i..next]);
            }
            i = next;
        }
    }
    out
}

/// A heading block re-levelled by its outline depth (depth 0 → h2, 1 → h3, 2+ → h4).
fn set_heading_level(block: &str, depths: &std::collections::HashMap<String, usize>) -> String {
    let b = block.as_bytes();
    if !(block.starts_with("<h") && b.len() > 3 && b[2].is_ascii_digit()) {
        return block.to_string();
    }
    let Some(&d) = depths.get(&squash(&text_of(block))) else {
        return block.to_string();
    };
    let old = b[2] as char;
    let new = char::from_digit((2 + d).min(4) as u32, 10).expect("digit");
    if old == new {
        return block.to_string();
    }
    let open_end = block.find('>').unwrap_or(block.len());
    let close = format!("</h{old}>");
    let body_end = block.rfind(&close).unwrap_or(block.len());
    format!(
        "<h{new}{}{}</h{new}>{}",
        &block[3..open_end + 1],
        &block[open_end + 1..body_end],
        &block[(body_end + close.len()).min(block.len())..]
    )
}

/// Rows of one table inside `<table>`, without `<tbody>` wrappers.
fn table_rows(t: &str) -> &str {
    let inner = &t[t.find('>').map_or(0, |i| i + 1)..t.rfind("</table>").unwrap_or(t.len())];
    let inner = inner.trim();
    let inner = inner.strip_prefix("<tbody>").unwrap_or(inner);
    inner.strip_suffix("</tbody>").unwrap_or(inner)
}

/// Rows of a table, each as its HTML.
fn rows_of(t: &str) -> Vec<&str> {
    let body = table_rows(t);
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find("<tr") {
        let end = rest[i..].find("</tr>").map_or(rest.len(), |e| i + e + 5);
        out.push(&rest[i..end]);
        rest = &rest[end..];
    }
    out
}

/// Columns of a row, counting merged cells (`colspan`).
fn row_columns(row: &str) -> usize {
    let mut n = 0;
    for (i, _) in row.match_indices("<t") {
        let tag = &row[i..row[i..].find('>').map_or(row.len(), |e| i + e)];
        if !(tag.starts_with("<td") || tag.starts_with("<th")) {
            continue;
        }
        let span = tag
            .find("colspan=\"")
            .and_then(|j| tag[j + 9..].split('"').next()?.parse::<usize>().ok())
            .unwrap_or(1);
        n += span;
    }
    n
}

fn columns(t: &str) -> usize {
    rows_of(t).first().map_or(0, |r| row_columns(r))
}

/// A table cut by a page break comes back as two tables on two pages; the second has no
/// header row. Joins them (keeping the page range).
fn merge_split_tables(blocks: Vec<(u32, u32, String)>) -> Vec<(u32, u32, String)> {
    let mut out: Vec<(u32, u32, String)> = Vec::new();
    for (first, last, b) in blocks {
        if let Some(prev) = out.last_mut()
            && prev.2.starts_with("<table")
            && b.starts_with("<table")
            && first == prev.1 + 1
            && columns(&prev.2) == columns(&b)
        {
            let head = rows_of(&prev.2)
                .first()
                .map(|r| text_of(r))
                .unwrap_or_default();
            let rows = rows_of(&b);
            // A repeated header on the next page is dropped; any other header means a new table.
            let (same_head, rest) = match rows.first() {
                Some(r) if r.contains("<th") && text_of(r) == head => (true, &rows[1..]),
                Some(r) if r.contains("<th") => (false, &rows[..]),
                _ => (true, &rows[..]),
            };
            if same_head {
                prev.2 = format!("<table>{}{}</table>", table_rows(&prev.2), rest.concat());
                prev.1 = last;
                continue;
            }
        }
        out.push((first, last, b));
    }
    out
}

/// Keeps the current document as a version before it is replaced, so no edits are lost.
pub(crate) fn keep_previous(ws: &Path, feature: &str) -> Result<(), ManualError> {
    if let Some(old) = load(ws, feature) {
        let path = dir(ws, feature).join(format!("v{}.json", old.version));
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&old).expect("serializes"),
        )
        .map_err(io(&path))?;
    }
    Ok(())
}

/// Editor HTML from Markdown (tables and strikethrough included; raw HTML dropped).
pub fn markdown_html(md: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser};
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser =
        Parser::new_ext(md, opts).filter(|e| !matches!(e, Event::Html(_) | Event::InlineHtml(_)));
    let mut out = String::new();
    pulldown_cmark::html::push_html(&mut out, parser);
    out
}

/// Stored versions, oldest first: `(version, saved_at)`, the current document last.
pub fn versions(ws: &Path, feature: &str) -> Vec<(u32, u64)> {
    let d = dir(ws, feature);
    let mut out: Vec<(u32, u64)> = std::fs::read_dir(&d)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().starts_with('v'))
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .filter_map(|t| serde_json::from_str::<Doc>(&t).ok())
                .map(|d| (d.version, d.updated_at))
                .collect()
        })
        .unwrap_or_default();
    if let Some(cur) = load(ws, feature) {
        out.push((cur.version, cur.updated_at));
    }
    out.sort();
    out.dedup_by_key(|v| v.0);
    out
}

pub fn version(ws: &Path, feature: &str, n: u32) -> Option<Doc> {
    match load(ws, feature) {
        Some(cur) if cur.version == n => Some(cur),
        _ => serde_json::from_str(
            &std::fs::read_to_string(dir(ws, feature).join(format!("v{n}.json"))).ok()?,
        )
        .ok(),
    }
}

/// Makes an earlier version the current one again (as a new version, so nothing is lost).
pub fn restore(ws: &Path, feature: &str, n: u32) -> Result<Doc, ManualError> {
    let old = version(ws, feature, n)
        .ok_or_else(|| ManualError::Invalid(format!("버전 {n}이 없습니다")))?;
    let bids = blocks(&old.html)
        .iter()
        .filter_map(|b| crate::provenance::bid_of(b))
        .collect();
    let change = crate::provenance::Change {
        bids,
        kind: "restore".into(),
        summary: format!("버전 {n}의 내용으로 되돌림"),
        ..Default::default()
    };
    save(ws, feature, &old.html, &[change])
}

const VOID: [&str; 6] = ["br", "img", "hr", "col", "input", "wbr"];

/// Top-level elements of editor HTML, in order (the editor's top-level nodes).
pub fn blocks(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0usize, None::<usize>);
    let mut i = 0;
    let b = html.as_bytes();
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let end = html[i..].find('>').map_or(b.len(), |e| i + e + 1);
        let tag = &html[i + 1..end.saturating_sub(1)];
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if tag.starts_with('/') {
            depth = depth.saturating_sub(1);
            if depth == 0
                && let Some(s) = start.take()
            {
                out.push(html[s..end].to_string());
            }
        } else if !tag.starts_with('!') {
            let closes = tag.ends_with('/') || VOID.contains(&name.as_str());
            if depth == 0 {
                if closes {
                    out.push(html[i..end].to_string());
                } else {
                    start = Some(i);
                }
            }
            if !closes {
                depth += 1;
            }
        }
        i = end;
    }
    out
}

/// An attribute of a block's opening tag, unescaped.
pub fn attr_of(block: &str, name: &str) -> Option<String> {
    let open = &block[..block.find('>').unwrap_or(block.len())];
    let key = format!(" {name}=\"");
    let i = open.find(&key)? + key.len();
    let j = open[i..].find('"')? + i;
    Some(
        open[i..j]
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

/// The block with an attribute of its opening tag set (or removed with `None`).
pub fn set_attr(block: &str, name: &str, value: Option<&str>) -> String {
    let mut b = block.to_string();
    if attr_of(block, name).is_some() {
        let open_end = b.find('>').unwrap_or(b.len());
        let key = format!(" {name}=\"");
        if let Some(i) = b[..open_end].find(&key) {
            let j = b[i + key.len()..]
                .find('"')
                .map_or(open_end, |j| i + key.len() + j + 1);
            b.replace_range(i..j, "");
        }
    }
    let Some(v) = value else { return b };
    let name_end = b
        .char_indices()
        .skip(1)
        .find(|(_, c)| !c.is_ascii_alphanumeric())
        .map_or(b.len(), |(i, _)| i);
    let v = v
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("{} {name}=\"{v}\"{}", &b[..name_end], &b[name_end..])
}

/// A picture slot of a written manual: what the picture must show (D-046).
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub bid: String,
    pub want: String,
    /// Picture it has (a capture under `shots/`), if any.
    pub shot: Option<String>,
}

pub fn picture_slots(html: &str) -> Vec<Slot> {
    blocks(html)
        .iter()
        .filter(|b| b.starts_with("<figure"))
        .filter_map(|b| {
            Some(Slot {
                bid: crate::provenance::bid_of(b)?,
                want: attr_of(b, "data-want")?,
                shot: attr_of(b, "data-shot"),
            })
        })
        .collect()
}

/// Puts captured pictures into their slots, or why a slot could not be shot; saved as a version.
pub fn fill_slots(
    ws: &Path,
    feature: &str,
    results: &[(String, Result<String, String>)],
) -> Result<Doc, ManualError> {
    let doc = load(ws, feature)
        .ok_or_else(|| ManualError::Invalid(format!("매뉴얼 문서가 없습니다: {feature}")))?;
    let mut html = String::new();
    let mut shot = Vec::new();
    for b in blocks(&doc.html) {
        let bid = crate::provenance::bid_of(&b);
        let r = bid
            .as_ref()
            .and_then(|id| results.iter().find(|(x, _)| x == id));
        let b = match r {
            Some((id, Ok(path))) => {
                shot.push(id.clone());
                set_attr(&set_attr(&b, "data-fail", None), "data-shot", Some(path))
            }
            Some((_, Err(why))) => set_attr(&b, "data-fail", Some(why)),
            None => b,
        };
        html.push_str(&b);
        html.push('\n');
    }
    save(
        ws,
        feature,
        &html,
        &[crate::provenance::Change {
            bids: shot,
            kind: "picture".into(),
            summary: "그림 자리의 장면을 찍어 넣음".into(),
            request: None,
            facts: Vec::new(),
            inherit: Vec::new(),
        }],
    )
}

/// Visible text of an HTML fragment (tags dropped, whitespace collapsed).
pub fn text_of(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A suggested change from the code (D-032 E6): people accept or reject it in the editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    /// `replace` the block, or `insert_after` it.
    pub action: String,
    /// Block index in the document the proposal was made from.
    pub block: usize,
    /// That block's text then, to find it again after edits.
    pub before: String,
    /// That block's `data-bid`, when it had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bid: Option<String>,
    pub html: String,
    pub reason: String,
    /// Fact ids it rests on.
    #[serde(default)]
    pub facts: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Proposals {
    /// Document version they were made from.
    pub version: u32,
    pub items: Vec<Proposal>,
}

pub fn load_proposals(ws: &Path, feature: &str) -> Proposals {
    std::fs::read_to_string(dir(ws, feature).join("proposals.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_proposals(ws: &Path, feature: &str, p: &Proposals) -> Result<(), ManualError> {
    let path = dir(ws, feature).join("proposals.json");
    std::fs::write(&path, serde_json::to_string_pretty(p).expect("serializes")).map_err(io(&path))
}

/// Removes a proposal once it was accepted or rejected.
pub fn resolve_proposal(ws: &Path, feature: &str, id: &str) -> Result<Proposals, ManualError> {
    let mut p = load_proposals(ws, feature);
    p.items.retain(|x| x.id != id);
    save_proposals(ws, feature, &p)?;
    Ok(p)
}

const PROPOSE_SYSTEM: &str = concat!(
    "You keep a Korean user manual up to date with the product. You get \
the manual as numbered blocks (HTML) and facts read from the current code (each with an id).

Propose changes only where the code says something the manual does not:
- a block states something the facts contradict (button or field names, columns, options, \
rules, messages, who may do what) -> action \"replace\" with the corrected block;
- a screen element, option, rule or step in the facts is missing -> action \"insert_after\" \
the block it belongs after, with a new block in the manual's style.
",
    user_rule!(),
    " Do not rewrite for style, do not reorder or restructure, do not touch blocks the facts do not \
cover, never remove figures. Keep the manual's wording and tone (합니다체 or the list style it \
uses) and change only what is needed. Use only the elements the manual uses (h2 h3 p ul ol li \
table tr th td strong em code, div data-panel, span data-status). A replaced <figure> is not \
allowed. reason: one Korean sentence on what changed in the product. Cite fact ids. At most 15 \
proposals; none is fine when the manual is current."
);

/// Asks the writer model where the document disagrees with the code facts.
pub async fn propose(
    d: &mwa_core::Dispatcher,
    ws: &Path,
    feature: &str,
    facts: &[crate::model::Fact],
) -> Result<Proposals, ManualError> {
    let doc = load(ws, feature)
        .ok_or_else(|| ManualError::Invalid(format!("매뉴얼 문서가 없습니다: {feature}")))?;
    let list = blocks(&doc.html);
    let numbered: Vec<String> = list
        .iter()
        .enumerate()
        .map(|(i, b)| format!("[{i}] {b}"))
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
                "action": {"type": "string", "enum": ["replace", "insert_after"]},
                "block": {"type": "integer"},
                "html": {"type": "string"},
                "reason": {"type": "string"},
                "facts": {"type": "array", "items": {"type": "string"}}
            },
            "required": ["action", "block", "html", "reason", "facts"]
        }}},
        "required": ["items"]
    });
    let prompt = format!(
        "## 매뉴얼 ({})\n{}\n\n## 현재 코드의 사실\n{}",
        doc.title,
        numbered.join("\n"),
        fact_lines.join("\n")
    );
    let out = d
        .call(
            mwa_protocol::Role::Writer,
            PROPOSE_SYSTEM,
            &prompt,
            &[],
            Some(&schema),
        )
        .await
        .map_err(|e| ManualError::Invalid(format!("갱신 제안 실패: {e}")))?;
    #[derive(Deserialize)]
    struct Raw {
        items: Vec<RawItem>,
    }
    #[derive(Deserialize)]
    struct RawItem {
        action: String,
        block: usize,
        html: String,
        reason: String,
        #[serde(default)]
        facts: Vec<String>,
    }
    let raw: Raw = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("갱신 제안: {e}")))?;
    let known: std::collections::HashSet<&str> = facts.iter().map(|f| f.id.as_str()).collect();
    let items = raw
        .items
        .into_iter()
        .filter(|r| r.block < list.len() && !r.html.trim().is_empty())
        // A replacement may not drop a picture.
        .filter(|r| !(r.action == "replace" && list[r.block].starts_with("<figure")))
        .enumerate()
        .map(|(n, r)| Proposal {
            id: format!("p{}-{n}", doc.version),
            before: text_of(&list[r.block]),
            bid: crate::provenance::bid_of(&list[r.block]),
            action: r.action,
            block: r.block,
            html: r.html,
            reason: r.reason,
            facts: r
                .facts
                .into_iter()
                .filter(|f| known.contains(f.as_str()))
                .collect(),
        })
        .collect();
    let p = Proposals {
        version: doc.version,
        items,
    };
    save_proposals(ws, feature, &p)?;
    Ok(p)
}

const ASSIST_SYSTEM: &str = concat!(
    "You help a person edit a Korean user manual for a software \
product, in a side panel of the editor. You get their request, the whole manual as HTML blocks \
(each top-level block has a data-bid), the blocks the person is looking at, the conversation so \
far, facts read from the product's current code (each with an id), and the house format.

Work out which blocks the request is about. Start from the blocks the person is looking at, but \
look at the whole manual: a table cut in two by a page break continues in the next table, a list \
may continue after a picture, a section named in the request may be elsewhere.

- answer: what you did or your answer, in Korean (합니다체), as long as needed. Refer to parts by \
their headings or contents; never mention data-bid values or HTML. Name the fact ids you relied \
on. Never invent product behavior: when the facts do not cover something, say so.
- edits: the changes, none when the request only asks a question. Each edit replaces a run of \
consecutive blocks (replace: their data-bid values in order) with html, or inserts html after a \
block (replace empty, after: that block's data-bid). Keep the data-bid attribute on blocks you \
keep or change; new blocks have none. Empty html deletes. Keep <figure> blocks as they are \
unless asked. Use only the manual's elements (h1 h2 h3 p ul ol li strong em u s code table tr th \
td, div data-panel=\"info|note|warning|success\", span data-status=\"grey|blue|green|yellow|red\", \
figure). Structure and format may change when asked (tables, lists, panels, section order).
",
    user_rule!(),
    " - facts: ids of the code facts the answer or edits rest on."
);

/// One change to the manual: replace consecutive blocks, or insert after one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Edit {
    #[serde(default)]
    pub replace: Vec<String>,
    #[serde(default)]
    pub after: String,
    #[serde(default)]
    pub html: String,
}

/// What the helper returned: an answer to read and the edits it proposes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Assist {
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub edits: Vec<Edit>,
    #[serde(default)]
    pub facts: Vec<String>,
}

/// One turn of the side panel conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub who: String,
    pub text: String,
}

/// The editor's side-panel helper: sees the whole manual, starts from `focus` (block ids the
/// person is looking at) and decides itself which blocks to change.
pub async fn assist(
    d: &mwa_core::Dispatcher,
    ws: &Path,
    request: &str,
    html: &str,
    focus: &[String],
    history: &[Turn],
    facts: &[crate::model::Fact],
) -> Result<Assist, ManualError> {
    let fact_lines: Vec<String> = facts
        .iter()
        .take(200)
        .map(|f| format!("{}: {}", f.id, f.statement))
        .collect();
    let template = std::fs::read_to_string(ws.join(crate::pipeline::TEMPLATE_FILE))
        .map(|t| t.chars().take(4000).collect::<String>())
        .unwrap_or_else(|_| "(없음)".into());
    let talk: Vec<String> = history
        .iter()
        .rev()
        .take(8)
        .rev()
        .map(|t| {
            format!(
                "{}: {}",
                if t.who == "user" {
                    "사람"
                } else {
                    "도우미"
                },
                t.text
            )
        })
        .collect();
    let prompt = format!(
        "## 요청\n{request}\n\n## 사람이 보고 있는 블록\n{}\n\n## 지금까지의 대화\n{}\n\n## 매뉴얼 전체\n{html}\n\n## 현재 코드의 사실\n{}\n\n## 프로젝트 양식\n{template}",
        if focus.is_empty() {
            "(없음)".to_string()
        } else {
            focus.join(", ")
        },
        if talk.is_empty() {
            "(처음)".to_string()
        } else {
            talk.join("\n")
        },
        if fact_lines.is_empty() {
            "(없음 — ③에서 코드를 읽으면 생깁니다)".to_string()
        } else {
            fact_lines.join("\n")
        }
    );
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "answer": {"type": "string"},
            "edits": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "replace": {"type": "array", "items": {"type": "string"}},
                    "after": {"type": "string"},
                    "html": {"type": "string"}
                },
                "required": ["replace", "after", "html"]
            }},
            "facts": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["answer", "edits", "facts"]
    });
    let out = d
        .call(
            mwa_protocol::Role::Chat,
            ASSIST_SYSTEM,
            &prompt,
            &[],
            Some(&schema),
        )
        .await
        .map_err(|e| ManualError::Invalid(format!("도우미 실패: {e}")))?;
    let mut a: Assist = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("도우미: {e}")))?;
    let known: std::collections::HashSet<&str> = facts.iter().map(|f| f.id.as_str()).collect();
    a.facts.retain(|f| known.contains(f.as_str()));
    // Drop edits that point at blocks the manual does not have.
    let bids: std::collections::HashSet<String> = blocks(html)
        .iter()
        .filter_map(|b| crate::provenance::bid_of(b))
        .collect();
    a.edits.retain(|e| {
        e.replace.iter().all(|b| bids.contains(b))
            && (!e.replace.is_empty() || bids.contains(&e.after))
    });
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_get_their_pictures() {
        let html = "<figure data-bid=\"w1-3\" data-want=\"주문 관리 — 첫 화면 — 목록 표\"><figcaption>목록</figcaption></figure><p data-bid=\"w1-4\">가</p>";
        let slots = picture_slots(html);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].want, "주문 관리 — 첫 화면 — 목록 표");
        let b = set_attr(&blocks(html)[0], "data-fail", Some("버튼이 \"없음\""));
        assert_eq!(attr_of(&b, "data-fail").as_deref(), Some("버튼이 \"없음\""));
        let b = set_attr(
            &set_attr(&b, "data-fail", None),
            "data-shot",
            Some("shots/doc/a/w1-3.png"),
        );
        assert_eq!(attr_of(&b, "data-fail"), None);
        assert_eq!(
            picture_slots(&b)[0].shot.as_deref(),
            Some("shots/doc/a/w1-3.png")
        );
        assert!(b.ends_with("<figcaption>목록</figcaption></figure>"));
    }

    #[test]
    fn save_keeps_versions_and_assets() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let first = save(ws, "f", "<p>하나</p>", &[]).unwrap();
        assert_eq!((first.version, first.origin.as_str()), (1, "generated"));
        let second = save(ws, "f", "<p>둘</p>", &[]).unwrap();
        assert_eq!(second.version, 2);
        assert_eq!(text_of(&load(ws, "f").unwrap().html), "둘");
        let v1: Doc = serde_json::from_str(
            &std::fs::read_to_string(ws.join("manual/docs/f/v1.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(text_of(&v1.html), "하나");

        let a = save_asset(ws, "f", "화면.PNG", b"png").unwrap();
        assert_eq!(a, save_asset(ws, "f", "다시.png", b"png").unwrap());
        assert!(asset(ws, "f", &a).is_some());
        assert!(asset(ws, "f", "../doc.json").is_none());
        assert!(save_asset(ws, "f", "x.svg", b"<svg/>").is_err());
        assert_eq!(
            versions(ws, "f").iter().map(|v| v.0).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(text_of(&version(ws, "f", 1).unwrap().html), "하나");
    }

    #[test]
    fn tables_cut_by_a_page_join() {
        let b = |p, h: &str| (p, p, h.to_string());
        let out = merge_split_tables(vec![
            b(
                2,
                "<table><tr><th>용어</th><th>의미</th></tr><tr><td>가</td><td>1</td></tr></table>",
            ),
            b(
                3,
                "<table><tbody><tr><td>나</td><td>2</td></tr></tbody></table>",
            ),
            b(3, "<p>끝</p>"),
            b(3, "<table><tr><th>새 표</th></tr></table>"),
        ]);
        assert_eq!(out.len(), 3);
        assert_eq!((out[0].0, out[0].1), (2, 3));
        assert!(out[0].2.contains("<td>가</td>") && out[0].2.contains("<td>나</td>"));
        assert_eq!(out[0].2.matches("<table").count(), 1);

        // Repeated header on the next page; colspan counted.
        let out = merge_split_tables(vec![
            b(
                4,
                "<table><tr><th>컬럼</th><th>내용</th></tr><tr><td colspan=\"2\">묶음</td></tr></table>",
            ),
            b(
                5,
                "<table><tr><th>컬럼</th><th>내용</th></tr><tr><td>상태</td><td>활성</td></tr></table>",
            ),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].2.matches("<th>").count(), 2, "header kept once");
        assert!(out[0].2.contains("<td>상태</td>"));
    }

    #[test]
    fn outline_depth_from_pdf_text() {
        let t = "주문 관리\n\n  소개 문단입니다.\n\n 권한 안내\n 화면 안내\n   첫 화면\n  검색창\n    등록 프로세스\n\n\n권한 안내\n\u{c}2쪽";
        let d = pdf_outline_depths(t);
        assert_eq!(
            (
                d["권한안내"],
                d["화면안내"],
                d["첫화면"],
                d["검색창"],
                d["등록프로세스"]
            ),
            (0, 0, 1, 1, 2)
        );
    }

    #[test]
    fn headings_follow_the_outline() {
        let toc = "<ul><li>권한</li><li>화면<ul><li>검색창</li><li>등록<ul><li>절차</li></ul></li></ul></li></ul>";
        let d = outline_depths(toc);
        assert_eq!(
            (d["권한"], d["화면"], d["검색창"], d["등록"], d["절차"]),
            (0, 0, 1, 1, 2)
        );
        assert_eq!(set_heading_level("<h2>검색창</h2>", &d), "<h3>검색창</h3>");
        assert_eq!(
            set_heading_level("<h3 data-bid=\"x\">권 한</h3>", &d),
            "<h2 data-bid=\"x\">권 한</h2>"
        );
        assert_eq!(set_heading_level("<h2>절차</h2>", &d), "<h4>절차</h4>");
        assert_eq!(
            set_heading_level("<h2>없는 제목</h2>", &d),
            "<h2>없는 제목</h2>"
        );
    }

    #[test]
    fn markdown_becomes_editor_html() {
        let h = markdown_html(
            "# 주문 관리\n\n| 역할 | 책임 |\n|---|---|\n| 관리자 | 등록 |\n\n<script>x</script>\n\n- ~~옛~~ 새",
        );
        assert!(h.contains("<h1>주문 관리</h1>"));
        assert!(h.contains("<table>") && h.contains("<th>역할</th>"));
        assert!(!h.contains("<script>"));
        assert!(h.contains("<del>옛</del>"));
    }

    #[test]
    fn splits_top_level_blocks() {
        let html = "<h2>제목</h2><p>가<br>나</p><table><tbody><tr><td><p>칸</p></td></tr></tbody></table><figure data-ref=\"img/a.png\"><figcaption>캡션</figcaption></figure><hr>";
        let b = blocks(html);
        assert_eq!(b.len(), 5);
        assert_eq!(b[1], "<p>가<br>나</p>");
        assert!(b[2].starts_with("<table") && b[2].ends_with("</table>"));
        assert_eq!(text_of(&b[1]), "가 나");
    }
}
