//! Existing manuals as reference (D-031): a PDF is split into text, screenshots and the place
//! each screenshot sits (page, nearest heading, text just before, caption just after), and
//! matched to a feature. Stored under `reference/<id>/` in the project. Uses poppler's
//! `pdftohtml -xml` and `pdftotext`; nothing here calls a model.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::ManualError;

pub const DIR: &str = "reference";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefImage {
    /// Relative to the document folder (`img/...`).
    pub file: String,
    pub page: u32,
    pub width: u32,
    pub height: u32,
    /// Nearest heading above the image.
    pub heading: String,
    /// Body text right before the image.
    pub context: String,
    /// Short line right below the image, if any (e.g. "비활성화").
    pub caption: String,
    /// What the picture shows, from the worker model looking at it (`describe`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<Scene>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// `screen`, `dialog`, `drawer`, `flow`, `diagram`, `icon` or `other`.
    pub kind: String,
    /// Which screen or component, in Korean.
    pub screen: String,
    /// Visible state (inputs filled, success banner, selected tab, ...), in Korean.
    pub state: String,
    /// A product UI state worth reproducing in a new screenshot.
    pub capture: bool,
    /// How a user reaches it, e.g. "[시스템 등록] → 값 입력 → [연결 테스트]".
    pub how: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefDoc {
    pub id: String,
    /// Title taken from the file name, e.g. "주문 관리".
    pub title: String,
    /// `(CRM)`/`(BI)` style tag in the file name, if any.
    #[serde(default)]
    pub tag: Option<String>,
    pub source: String,
    /// Feature key it documents; `None` until matched or chosen.
    #[serde(default)]
    pub feature: Option<String>,
    /// Marked as not documenting any feature (e.g. the login page).
    #[serde(default)]
    pub ignored: bool,
    pub pages: u32,
    pub imported_at: u64,
    pub images: Vec<RefImage>,
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> ManualError + '_ {
    move |e| ManualError::Io(path.to_path_buf(), e)
}

fn run(cmd: &mut Command, what: &str) -> Result<Vec<u8>, ManualError> {
    let out = cmd.output().map_err(|e| {
        ManualError::Invalid(format!(
            "{what}을 실행할 수 없습니다 (poppler 설치 필요: apt install poppler-utils / brew install poppler): {e}"
        ))
    })?;
    if !out.status.success() {
        return Err(ManualError::Invalid(format!(
            "{what} 실패: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(out.stdout)
}

/// "ACME-(CRM)보고서 생성-140726-002812" → ("보고서 생성", Some("CRM")).
pub fn title_of(stem: &str) -> (String, Option<String>) {
    let mut s = stem.trim();
    if let Some((prefix, rest)) = s.split_once('-')
        && prefix.chars().all(|c| c.is_ascii_uppercase())
        && !prefix.is_empty()
    {
        s = rest;
    }
    // Trailing export stamps: "-140726-002812".
    let mut parts: Vec<&str> = s.split('-').collect();
    while parts.len() > 1
        && parts
            .last()
            .is_some_and(|p| p.len() >= 4 && p.chars().all(|c| c.is_ascii_digit()))
    {
        parts.pop();
    }
    let s = parts.join("-");
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('(')
        && let Some((tag, title)) = rest.split_once(')')
    {
        return (title.trim().to_string(), Some(tag.trim().to_string()));
    }
    (s.to_string(), None)
}

/// (key, label, group) of each feature in the product map.
pub type FeatureRef<'a> = (&'a str, &'a str, Option<&'a str>);

/// The feature a document title names. A tag picks between same-named features by group
/// (a tag naming a group → that group; another tag → a group that is not a Latin tag itself).
pub fn match_feature(title: &str, tag: Option<&str>, features: &[FeatureRef]) -> Option<String> {
    let hits: Vec<&FeatureRef> = features.iter().filter(|f| f.1 == title).collect();
    match hits.as_slice() {
        [] => None,
        [one] => Some(one.0.to_string()),
        many => {
            // A tag naming a menu group picks that group. Another tag picks among the groups that
            // are not themselves tags (short Latin names like "BI" belong to their own tag).
            let tag = tag?;
            fn group<'a>(f: &FeatureRef<'a>) -> &'a str {
                f.2.unwrap_or("")
            }
            let named: Vec<&FeatureRef> = many
                .iter()
                .copied()
                .filter(|f| tag.eq_ignore_ascii_case(group(f)))
                .collect();
            let pick: Vec<&FeatureRef> = if named.is_empty() {
                many.iter()
                    .copied()
                    .filter(|f| {
                        let g = group(f);
                        g.is_empty() || !g.chars().all(|c| c.is_ascii_alphanumeric())
                    })
                    .collect()
            } else {
                named
            };
            (pick.len() == 1).then(|| pick[0].0.to_string())
        }
    }
}

#[derive(Debug)]
enum Item {
    Text {
        page: u32,
        top: i32,
        left: i32,
        width: i32,
        height: i32,
        size: f32,
        text: String,
    },
    Image {
        page: u32,
        top: i32,
        width: u32,
        height: u32,
        src: String,
    },
}

fn attrs(tag: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = tag;
    while let Some(eq) = rest.find("=\"") {
        let key = rest[..eq]
            .rsplit(|c: char| c.is_whitespace())
            .next()
            .unwrap_or("");
        let after = &rest[eq + 2..];
        let Some(end) = after.find('"') else { break };
        out.insert(key.to_string(), after[..end].to_string());
        rest = &after[end + 1..];
    }
    out
}

fn unescape(s: &str) -> String {
    let mut t = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => t.push(c),
            _ => {}
        }
    }
    t.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Parses `pdftohtml -xml` output into text fragments and images, in reading order.
fn parse_xml(xml: &str) -> Vec<Item> {
    let mut sizes: HashMap<String, f32> = HashMap::new();
    let mut items = Vec::new();
    let mut page = 0u32;
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        rest = &rest[start..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[1..end];
        if let Some(t) = tag.strip_prefix("page ") {
            page = attrs(t)
                .get("number")
                .and_then(|n| n.parse().ok())
                .unwrap_or(page);
        } else if let Some(t) = tag.strip_prefix("fontspec ") {
            let a = attrs(t);
            if let (Some(id), Some(size)) = (a.get("id"), a.get("size")) {
                sizes.insert(id.clone(), size.parse().unwrap_or(0.0));
            }
        } else if let Some(t) = tag.strip_prefix("image ") {
            let a = attrs(t);
            let num = |k: &str| a.get(k).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
            items.push(Item::Image {
                page,
                top: num("top") as i32,
                width: num("width") as u32,
                height: num("height") as u32,
                src: a.get("src").cloned().unwrap_or_default(),
            });
        } else if let Some(t) = tag.strip_prefix("text ") {
            let a = attrs(t);
            let close = rest.find("</text>").unwrap_or(end);
            let inner = &rest[end + 1..close.max(end + 1)];
            let num = |k: &str| a.get(k).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
            items.push(Item::Text {
                page,
                top: num("top"),
                left: num("left"),
                width: num("width"),
                height: num("height"),
                size: a
                    .get("font")
                    .and_then(|f| sizes.get(f))
                    .copied()
                    .unwrap_or(0.0),
                text: unescape(inner),
            });
            rest = &rest[close.max(end)..];
            continue;
        }
        rest = &rest[end + 1..];
    }
    items
}

#[derive(Debug, Clone)]
struct Line {
    page: u32,
    top: i32,
    size: f32,
    text: String,
}

/// Groups fragments into lines (same page, baselines within 3px), then joins each line
/// left to right. Spaces may be separate fragments a pixel off the baseline.
fn lines_of(items: &[Item]) -> Vec<Line> {
    struct Frag<'a> {
        page: u32,
        top: i32,
        left: i32,
        width: i32,
        height: i32,
        size: f32,
        text: &'a str,
    }
    let mut frags: Vec<Frag> = items
        .iter()
        .filter_map(|i| match i {
            Item::Text {
                page,
                top,
                left,
                width,
                height,
                size,
                text,
            } => Some(Frag {
                page: *page,
                top: *top,
                left: *left,
                width: *width,
                height: *height,
                size: *size,
                text: text.as_str(),
            }),
            _ => None,
        })
        .collect();
    frags.sort_by_key(|f| (f.page, f.top, f.left));
    let mut groups: Vec<Vec<Frag>> = Vec::new();
    for f in frags {
        match groups.last_mut() {
            Some(g) if g[0].page == f.page && (f.top - g[0].top).abs() <= 3 => g.push(f),
            _ => groups.push(vec![f]),
        }
    }
    groups
        .into_iter()
        .filter_map(|mut g| {
            g.sort_by_key(|f| f.left);
            let mut text = String::new();
            let mut right = i32::MIN;
            for f in &g {
                // A visible gap without a space fragment still separates words.
                if right != i32::MIN && f.left - right > f.height / 4 && !text.ends_with(' ') {
                    text.push(' ');
                }
                text.push_str(f.text);
                right = f.left + f.width;
            }
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                return None;
            }
            Some(Line {
                page: g[0].page,
                top: g[0].top,
                size: g.iter().map(|f| f.size).fold(0.0, f32::max),
                text,
            })
        })
        .collect()
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn id_for(title: &str, tag: Option<&str>) -> String {
    let base = match tag {
        Some(t) => format!("{t}-{title}"),
        None => title.to_string(),
    };
    base.chars()
        .map(|c| {
            if c.is_whitespace() || "/\\:*?\"<>|()".contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Imports one PDF into `reference/<id>/` (replacing an earlier import of the same title).
/// True for an old manual given as Markdown (no pages or cut-out pictures).
pub fn is_markdown(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".md") || p.ends_with(".markdown")
}

/// Imports an old manual: a PDF (pages, pictures) or a Markdown file (text only).
pub fn import(ws: &Path, pdf: &Path, features: &[FeatureRef]) -> Result<RefDoc, ManualError> {
    if is_markdown(&pdf.to_string_lossy()) {
        return import_markdown(ws, pdf, features);
    }
    let stem = pdf
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (title, tag) = title_of(&stem);
    let id = id_for(&title, tag.as_deref());
    let dir = ws.join(DIR).join(&id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(io(&dir))?;
    }
    let img = dir.join("img");
    std::fs::create_dir_all(&img).map_err(io(&img))?;

    let text = run(
        Command::new("pdftotext")
            .args(["-layout", "-enc", "UTF-8"])
            .arg(pdf)
            .arg("-"),
        "pdftotext",
    )?;
    std::fs::write(dir.join("text.txt"), &text).map_err(io(&dir))?;
    run(
        Command::new("pdftohtml")
            .args(["-xml", "-q", "-nodrm", "-fmt", "png"])
            .arg(pdf)
            .arg(img.join("p")),
        "pdftohtml",
    )?;
    let xml_path = img.join("p.xml");
    let xml = std::fs::read_to_string(&xml_path).map_err(io(&xml_path))?;
    let _ = std::fs::remove_file(&xml_path);

    let items = parse_xml(&xml);
    let pages = items
        .iter()
        .map(|i| match i {
            Item::Text { page, .. } | Item::Image { page, .. } => *page,
        })
        .max()
        .unwrap_or(0);
    let lines = lines_of(&items);
    // Body size = most common line size; headings are clearly larger.
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for l in &lines {
        *counts.entry((l.size * 10.0) as i32).or_default() += 1;
    }
    let body = counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(s, _)| s as f32 / 10.0)
        .unwrap_or(0.0);

    // Confluence exports list every heading on the first page; CJK glyphs come in a fallback
    // font without bold or larger sizes, so the outline is the reliable heading signal.
    let toc_page = lines.first().map_or(1, |l| l.page);
    let toc: std::collections::HashSet<&str> = lines
        .iter()
        .filter(|l| l.page == toc_page)
        .skip(1)
        .map(|l| l.text.as_str())
        .filter(|t| {
            t.chars().count() <= 30
                && t.chars().filter(|c| c.is_alphanumeric()).count() >= 2
                && !t.ends_with('.')
                && !t.ends_with('다')
        })
        .collect();

    let mut images = Vec::new();
    for i in &items {
        let Item::Image {
            page,
            top,
            width,
            height,
            src,
        } = i
        else {
            continue;
        };
        // Icons and rules are not screenshots.
        if *width < 150 || *height < 40 {
            let _ = std::fs::remove_file(img.join(Path::new(src).file_name().unwrap_or_default()));
            continue;
        }
        let before: Vec<&Line> = lines
            .iter()
            .filter(|l| (l.page, l.top) < (*page, *top))
            .collect();
        let is_heading =
            |l: &Line| l.size >= body + 1.5 || (l.page > toc_page && toc.contains(l.text.as_str()));
        let heading = before
            .iter()
            .rev()
            .find(|l| is_heading(l))
            .map(|l| l.text.clone())
            .unwrap_or_default();
        let context: Vec<&str> = before
            .iter()
            .rev()
            .take_while(|l| !is_heading(l))
            .take(3)
            .map(|l| l.text.as_str())
            .collect();
        let context = context.into_iter().rev().collect::<Vec<_>>().join(" ");
        let bottom = top + *height as i32;
        let caption = lines
            .iter()
            .find(|l| l.page == *page && l.top >= bottom && l.top <= bottom + 30)
            .filter(|l| l.text.chars().count() <= 40)
            .map(|l| l.text.clone())
            .unwrap_or_default();
        let file = Path::new(src)
            .file_name()
            .map(|n| format!("img/{}", n.to_string_lossy()))
            .unwrap_or_default();
        images.push(RefImage {
            file,
            page: *page,
            width: *width,
            height: *height,
            heading,
            context,
            caption,
            scene: None,
        });
    }

    let doc = RefDoc {
        ignored: false,
        feature: match_feature(&title, tag.as_deref(), features),
        id,
        title,
        tag,
        source: pdf.display().to_string(),
        pages,
        imported_at: now(),
        images,
    };
    save(ws, &doc)?;
    Ok(doc)
}

pub fn save(ws: &Path, doc: &RefDoc) -> Result<(), ManualError> {
    let path = ws.join(DIR).join(&doc.id).join("doc.json");
    let text = serde_json::to_string_pretty(doc).expect("doc serializes");
    std::fs::write(&path, text).map_err(io(&path))
}

pub fn list(ws: &Path) -> Vec<RefDoc> {
    let mut out: Vec<RefDoc> = std::fs::read_dir(ws.join(DIR))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| std::fs::read_to_string(e.path().join("doc.json")).ok())
                .filter_map(|t| serde_json::from_str(&t).ok())
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.title.cmp(&b.title));
    out
}

/// The reference document for a feature, if one was matched or chosen.
pub fn for_feature(ws: &Path, feature: &str) -> Option<RefDoc> {
    list(ws)
        .into_iter()
        .find(|d| d.feature.as_deref() == Some(feature))
}

/// An example or existing manual read for the models: text, plus page images for PDFs so
/// layout (tables, flow boxes, side-by-side dialogs) can be seen, not only read.
#[derive(Debug, Clone)]
pub struct Example {
    pub name: String,
    pub text: String,
    pub pages: Vec<PathBuf>,
}

/// Reads Markdown/text as is; a PDF becomes `pdftotext -layout` text plus pages 2–5 rendered
/// under `.mwa/example-pages/` (page 1 of a Confluence export is the outline).
pub fn load_example(ws: &Path, path: &Path) -> Result<Example, ManualError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let is_pdf = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    if !is_pdf {
        let text = std::fs::read_to_string(path).map_err(io(path))?;
        return Ok(Example {
            name,
            text,
            pages: Vec::new(),
        });
    }
    let text = run(
        Command::new("pdftotext")
            .args(["-layout", "-enc", "UTF-8"])
            .arg(path)
            .arg("-"),
        "pdftotext",
    )?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dir = ws.join(".mwa/example-pages").join(id_for(&stem, None));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(io(&dir))?;
    run(
        Command::new("pdftoppm")
            .args(["-r", "60", "-png", "-f", "2", "-l", "5"])
            .arg(path)
            .arg(dir.join("page")),
        "pdftoppm",
    )?;
    let mut pages: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(io(&dir))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect();
    pages.sort();
    Ok(Example {
        name,
        text: String::from_utf8_lossy(&text).into_owned(),
        pages,
    })
}

pub fn doc_dir(ws: &Path, id: &str) -> PathBuf {
    ws.join(DIR).join(id)
}

pub fn load(ws: &Path, id: &str) -> Option<RefDoc> {
    serde_json::from_str(&std::fs::read_to_string(doc_dir(ws, id).join("doc.json")).ok()?).ok()
}

/// A file inside a document folder; only plain relative paths are served.
pub fn file(ws: &Path, id: &str, rel: &str) -> Option<PathBuf> {
    let ok = |p: &Path| {
        p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
    };
    if !ok(Path::new(id)) || !ok(Path::new(rel)) {
        return None;
    }
    let p = doc_dir(ws, id).join(rel);
    p.is_file().then_some(p)
}

/// Page `n` of the original PDF as PNG (rendered once, then cached).
pub fn page(ws: &Path, id: &str, n: u32) -> Result<PathBuf, ManualError> {
    let doc = load(ws, id)
        .ok_or_else(|| ManualError::Invalid(format!("기존 매뉴얼이 없습니다: {id}")))?;
    if n == 0 || n > doc.pages {
        return Err(ManualError::Invalid(format!("{n}쪽이 없습니다")));
    }
    let dir = doc_dir(ws, id).join("pages");
    let out = dir.join(format!("p{n}.png"));
    if out.is_file() {
        return Ok(out);
    }
    std::fs::create_dir_all(&dir).map_err(io(&dir))?;
    let n = n.to_string();
    run(
        Command::new("pdftoppm")
            .args(["-r", "110", "-png", "-singlefile", "-f", &n, "-l", &n])
            .arg(&doc.source)
            .arg(out.with_extension("")),
        "pdftoppm",
    )?;
    Ok(out)
}

impl RefImage {
    /// A picture of the product's screens (whole or cut): it can be taken again. Drawn flows,
    /// diagrams and icons cannot.
    pub fn shootable(&self) -> bool {
        self.scene
            .as_ref()
            .is_some_and(|s| s.capture || !matches!(s.kind.as_str(), "flow" | "diagram" | "icon"))
    }
}

/// Result of trying to reproduce one scene of the old manual (④).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneShot {
    /// The old manual's picture (`img/...` in the reference document).
    pub image: String,
    pub page: u32,
    pub heading: String,
    pub what: String,
    /// `reproduced` (judge: same), `different` (judge: same screen, other state or UI) or
    /// `failed` (not taken, or another screen).
    pub status: String,
    /// For `failed`: `needs_write`, `realtime`, `account`, `not_found`, `no_data`,
    /// `wrong_screen` or `error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What differs from the old picture (judge), most important first.
    #[serde(default)]
    pub differences: Vec<String>,
    /// Of those, what changed in the product's screen itself: the manual's text follows these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
    /// New picture, relative to the workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// `"world"` when demo data filled the screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<mwa_capture::detail::Plan>,
    /// Every attempt (first plan, then re-plans), so a user can see where it got stuck.
    #[serde(default)]
    pub attempts: Vec<Attempt>,
    /// Write requests stopped because no demo response existed.
    #[serde(default)]
    pub blocked: Vec<String>,
    /// Requests answered from demo data.
    #[serde(default)]
    pub mocked: Vec<String>,
    /// Picture a person uploaded for this scene (file in the document's assets); wins over
    /// the capture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uploaded: Option<String>,
    /// Picture chosen for the new manual (see `set_scene_choice`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
}

/// One try at a scene: the steps as planned and how each went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attempt {
    pub steps: Vec<StepTrace>,
    /// Plain-language reason it failed, if it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Picture of the screen where it got stuck (workspace-relative).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepTrace {
    /// e.g. "클릭 · 버튼 '시스템 등록' (창 안)".
    pub text: String,
    /// `None` = not run (an earlier step failed).
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Attempt record from a shot result.
pub fn attempt_of(r: &mwa_capture::detail::DetailRecord) -> Attempt {
    let action = |a: &str| {
        match a {
            "click" => "클릭",
            "fill" => "입력",
            "select" => "선택",
            "hover" => "올리기",
            "wait" => "기다림",
            other => other,
        }
        .to_string()
    };
    let steps = r
        .plan
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut text = format!("{} · {} '{}'", action(&s.action), s.role, s.name);
            if !s.row.is_empty() {
                text.push_str(&format!(" (행 '{}')", s.row));
            }
            if !s.near.is_empty() {
                text.push_str(&format!(" ('{}' 근처)", s.near));
            }
            if s.within == "dialog" {
                text.push_str(" (창 안)");
            }
            if !s.value.is_empty() {
                text.push_str(&format!(" = '{}'", s.value));
            }
            let log = r.steps_log.get(i);
            StepTrace {
                text,
                ok: log.map(|l| l.ok),
                ms: log.map_or(0, |l| l.ms),
                error: log.and_then(|l| l.error.clone()),
            }
        })
        .collect();
    Attempt {
        steps,
        error: (!r.ok).then(|| r.error.clone().unwrap_or_default()),
        fail_file: r.fail_file.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneShots {
    pub feature: String,
    pub doc: String,
    pub taken_at: u64,
    pub scenes: Vec<SceneShot>,
}

pub fn scene_shots_path(ws: &Path, feature: &str) -> PathBuf {
    ws.join(format!("shots/reference/{feature}/manifest.json"))
}

/// The current-product picture a scene uses in the new manual, as the screen picks it
/// (`web/src/pictures.ts`): a person's choice, else an upload, else a usable capture. None when
/// the old picture stays.
pub fn current_picture(ws: &Path, feature: &str, s: &SceneShot) -> Option<PathBuf> {
    let upload = || {
        s.uploaded
            .as_deref()
            .and_then(|f| crate::document::asset(ws, feature, f))
    };
    let capture = || s.file.as_ref().map(|f| ws.join(f)).filter(|p| p.is_file());
    match s.choice.as_deref() {
        Some("old") => return None,
        Some("upload") if s.uploaded.is_some() => return upload(),
        Some("current") if s.file.is_some() => return capture(),
        _ => {}
    }
    upload().or_else(|| (s.status != "failed").then(capture).flatten())
}

pub fn load_scene_shots(ws: &Path, feature: &str) -> Option<SceneShots> {
    serde_json::from_str(&std::fs::read_to_string(scene_shots_path(ws, feature)).ok()?).ok()
}

pub fn save_scene_shots(ws: &Path, s: &SceneShots) -> Result<(), ManualError> {
    let path = scene_shots_path(ws, &s.feature);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io(dir))?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(s).expect("serializes")).map_err(io(&path))
}

/// Combines a new capture with the stored one: scenes not shot again stay, and pictures people
/// uploaded are kept. Scenes follow the old manual's picture order.
pub fn merge_scene_shots(ws: &Path, mut new: SceneShots) -> SceneShots {
    if let Some(old) = load_scene_shots(ws, &new.feature) {
        for s in &mut new.scenes {
            if let Some(o) = old.scenes.iter().find(|o| o.image == s.image) {
                s.uploaded.clone_from(&o.uploaded);
                s.choice.clone_from(&o.choice);
            }
        }
        for o in old.scenes {
            if !new.scenes.iter().any(|s| s.image == o.image) {
                new.scenes.push(o);
            }
        }
    }
    order_scenes(ws, &mut new);
    new
}

fn order_scenes(ws: &Path, shots: &mut SceneShots) {
    if let Some(doc) = load(ws, &shots.doc) {
        let pos = |img: &str| {
            doc.images
                .iter()
                .position(|i| i.file == img)
                .unwrap_or(usize::MAX)
        };
        shots.scenes.sort_by_key(|s| pos(&s.image));
    }
}

/// Sets (or clears) the picture a person uploaded for one old-manual picture.
pub fn set_scene_upload(
    ws: &Path,
    feature: &str,
    image: &str,
    file: Option<String>,
) -> Result<SceneShots, ManualError> {
    edit_scene(ws, feature, image, |s| {
        // An upload is a choice of that upload; removing it falls back to automatic.
        s.choice = file.as_ref().map(|_| "upload".to_string());
        s.uploaded = file;
    })
}

/// Which picture the new manual uses for a scene: `current` (the capture), `old` (the old
/// manual's), `upload`, or `None` (automatic: upload, else a usable capture, else old).
pub fn set_scene_choice(
    ws: &Path,
    feature: &str,
    image: &str,
    choice: Option<String>,
) -> Result<SceneShots, ManualError> {
    if let Some(c) = &choice
        && !["current", "old", "upload"].contains(&c.as_str())
    {
        return Err(ManualError::Invalid(format!(
            "알 수 없는 그림 선택입니다: {c}"
        )));
    }
    edit_scene(ws, feature, image, |s| s.choice = choice)
}

/// Changes one scene's record, creating a "not taken" one for pictures never shot.
fn edit_scene(
    ws: &Path,
    feature: &str,
    image: &str,
    change: impl FnOnce(&mut SceneShot),
) -> Result<SceneShots, ManualError> {
    let doc = for_feature(ws, feature).ok_or_else(|| {
        ManualError::Invalid(format!(
            "이 기능에 짝지은 기존 매뉴얼이 없습니다: {feature}"
        ))
    })?;
    let img =
        doc.images.iter().find(|i| i.file == image).ok_or_else(|| {
            ManualError::Invalid(format!("기존 매뉴얼에 없는 그림입니다: {image}"))
        })?;
    let mut shots = load_scene_shots(ws, feature).unwrap_or_else(|| SceneShots {
        feature: feature.to_string(),
        doc: doc.id.clone(),
        taken_at: 0,
        scenes: Vec::new(),
    });
    if !shots.scenes.iter().any(|s| s.image == image) {
        shots.scenes.push(SceneShot {
            image: image.to_string(),
            page: img.page,
            heading: img.heading.clone(),
            what: img.scene.as_ref().map_or_else(
                || img.caption.clone(),
                |s| format!("{} — {}", s.screen, s.state),
            ),
            status: "not_taken".into(),
            reason_kind: None,
            reason: None,
            differences: Vec::new(),
            changes: Vec::new(),
            file: None,
            data: None,
            plan: None,
            attempts: Vec::new(),
            blocked: Vec::new(),
            mocked: Vec::new(),
            uploaded: None,
            choice: None,
        });
    }
    change(
        shots
            .scenes
            .iter_mut()
            .find(|s| s.image == image)
            .expect("just ensured"),
    );
    order_scenes(ws, &mut shots);
    save_scene_shots(ws, &shots)?;
    Ok(shots)
}

/// Makes `id` the old manual of `feature` (one per feature: others are unlinked).
pub fn link(ws: &Path, feature: &str, id: &str) -> Result<RefDoc, ManualError> {
    let mut doc = load(ws, id)
        .ok_or_else(|| ManualError::Invalid(format!("기존 매뉴얼이 없습니다: {id}")))?;
    for mut other in list(ws) {
        if other.id != id && other.feature.as_deref() == Some(feature) {
            other.feature = None;
            save(ws, &other)?;
        }
    }
    doc.feature = Some(feature.to_string());
    doc.ignored = false;
    save(ws, &doc)?;
    Ok(doc)
}

/// Unpairs the feature from its old manual: its manual is then written from scratch (D-047).
/// The old manual stays imported and can be paired again.
pub fn unlink(ws: &Path, feature: &str) -> Result<(), ManualError> {
    for mut d in list(ws) {
        if d.feature.as_deref() == Some(feature) {
            d.feature = None;
            save(ws, &d)?;
        }
    }
    Ok(())
}

fn import_markdown(ws: &Path, md: &Path, features: &[FeatureRef]) -> Result<RefDoc, ManualError> {
    let stem = md
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (title, tag) = title_of(&stem);
    let id = id_for(&title, tag.as_deref());
    let dir = ws.join(DIR).join(&id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(io(&dir))?;
    }
    std::fs::create_dir_all(&dir).map_err(io(&dir))?;
    let text = std::fs::read_to_string(md).map_err(io(md))?;
    std::fs::write(dir.join("text.txt"), &text).map_err(io(&dir))?;
    let doc = RefDoc {
        ignored: false,
        feature: match_feature(&title, tag.as_deref(), features),
        id,
        title,
        tag,
        source: md.display().to_string(),
        pages: 0,
        imported_at: now(),
        images: Vec::new(),
    };
    save(ws, &doc)?;
    Ok(doc)
}

/// Stores an uploaded PDF or Markdown file in the project, imports it and links it to `feature`.
pub fn import_upload(
    ws: &Path,
    feature: &str,
    name: &str,
    data: &[u8],
    features: &[FeatureRef],
) -> Result<RefDoc, ManualError> {
    let file = Path::new(name)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .filter(|f| f.to_ascii_lowercase().ends_with(".pdf") || is_markdown(f))
        .ok_or_else(|| ManualError::Invalid("PDF나 MD(마크다운) 파일만 올릴 수 있습니다".into()))?;
    let dir = ws.join(DIR).join("_files");
    std::fs::create_dir_all(&dir).map_err(io(&dir))?;
    let path = dir.join(&file);
    std::fs::write(&path, data).map_err(io(&path))?;
    let doc = import(ws, &path, features)?;
    link(ws, feature, &doc.id)
}

/// "재현 7 · 다름 2 · 못 찍음 1 (실제 서버 쓰기 필요 1)".
pub fn summarize(s: &SceneShots) -> String {
    let count = |st: &str| s.scenes.iter().filter(|x| x.status == st).count();
    let mut reasons: std::collections::BTreeMap<&str, usize> = Default::default();
    for x in s.scenes.iter().filter(|x| x.status == "failed") {
        *reasons
            .entry(reason_label(x.reason_kind.as_deref().unwrap_or("error")))
            .or_default() += 1;
    }
    let why: Vec<String> = reasons.iter().map(|(k, n)| format!("{k} {n}")).collect();
    format!(
        "{}장 중 재현 {} · 다름 {} · 못 찍음 {}{}",
        s.scenes.len(),
        count("reproduced"),
        count("different"),
        count("failed"),
        if why.is_empty() {
            String::new()
        } else {
            format!(" ({})", why.join(", "))
        }
    )
}

pub fn reason_label(kind: &str) -> &'static str {
    match kind {
        "needs_write" => "실제 서버 쓰기 필요",
        "realtime" => "실시간·외부 의존",
        "account" => "다른 계정·권한 필요",
        "not_found" => "화면·버튼을 못 찾음",
        "no_data" => "그 상태의 데이터 없음",
        "wrong_screen" => "다른 화면이 찍힘",
        _ => "오류",
    }
}

const SCENE_SYSTEM: &str = "You look at pictures taken from an older Korean user manual of a web product. \
For each numbered picture say what it shows so a tool can reproduce the same state in a new screenshot of the \
current product. kind: screen (a page), dialog (modal), drawer (side panel), flow (process boxes drawn for the \
manual), diagram (other drawing), icon, other. screen: which screen or component, in Korean. state: what is \
visible beyond the default (inputs filled, success or error message, row selected, tab chosen, menu open), in \
Korean. capture: true only for real product UI (screen, dialog, drawer) worth reproducing; false for drawings, \
icons and decorations. how: how a user reaches that state, in Korean, with buttons in brackets like \
[시스템 등록] → 값 입력 → [연결 테스트]; empty when capture is false. Use the heading and text around each \
picture to name things the way the manual does.";

/// Asks the worker model to describe every picture (in batches) and saves the result.
pub async fn describe(
    d: &mwa_core::Dispatcher,
    ws: &Path,
    id: &str,
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<RefDoc, ManualError> {
    let mut doc = load(ws, id)
        .ok_or_else(|| ManualError::Invalid(format!("기존 매뉴얼이 없습니다: {id}")))?;
    let dir = doc_dir(ws, id);
    let schema = serde_json::json!({
        "type": "object",
        "properties": {"items": {"type": "array", "items": {
            "type": "object",
            "properties": {
                "index": {"type": "integer"},
                "kind": {"type": "string", "enum": ["screen", "dialog", "drawer", "flow", "diagram", "icon", "other"]},
                "screen": {"type": "string"},
                "state": {"type": "string"},
                "capture": {"type": "boolean"},
                "how": {"type": "string"}
            },
            "required": ["index", "kind", "screen", "state", "capture", "how"]
        }}},
        "required": ["items"]
    });
    let batches: Vec<Vec<usize>> = (0..doc.images.len())
        .collect::<Vec<_>>()
        .chunks(6)
        .map(|c| c.to_vec())
        .collect();
    let total = batches.len();
    let calls = batches.iter().enumerate().map(|(b, idx)| {
        let lines: Vec<String> = idx
            .iter()
            .enumerate()
            .map(|(n, &i)| {
                let im = &doc.images[i];
                format!(
                    "{}. (p.{}) 제목: {} / 앞 설명: {} / 캡션: {}",
                    n + 1,
                    im.page,
                    im.heading,
                    im.context,
                    im.caption
                )
            })
            .collect();
        let images: Vec<PathBuf> = idx.iter().map(|&i| dir.join(&doc.images[i].file)).collect();
        let prompt = format!(
            "기존 매뉴얼: {}\n그림 {}장을 순서대로 보냅니다. index는 1부터입니다.\n{}",
            doc.title,
            idx.len(),
            lines.join("\n")
        );
        let schema = &schema;
        async move {
            let out = d
                .call(
                    mwa_protocol::Role::Worker,
                    SCENE_SYSTEM,
                    &prompt,
                    &images,
                    Some(schema),
                )
                .await;
            progress(format!("장면 설명 {}/{total}", b + 1));
            out
        }
    });
    let results = futures::future::join_all(calls).await;
    #[derive(Deserialize)]
    struct Raw {
        items: Vec<RawScene>,
    }
    #[derive(Deserialize)]
    struct RawScene {
        index: usize,
        kind: String,
        screen: String,
        state: String,
        capture: bool,
        how: String,
    }
    for (idx, r) in batches.iter().zip(results) {
        let raw: Raw = serde_json::from_value(r?.response.json.unwrap_or_default())
            .map_err(|e| ManualError::Shape(format!("장면 설명: {e}")))?;
        for s in raw.items {
            let Some(&i) = s.index.checked_sub(1).and_then(|k| idx.get(k)) else {
                continue;
            };
            doc.images[i].scene = Some(Scene {
                kind: s.kind,
                screen: s.screen,
                state: s.state,
                capture: s.capture,
                how: s.how,
            });
        }
    }
    save(ws, &doc)?;
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploads_survive_and_captures_merge() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let img = |file: &str, page| RefImage {
            file: file.into(),
            page,
            width: 1,
            height: 1,
            heading: "h".into(),
            context: String::new(),
            caption: String::new(),
            scene: None,
        };
        std::fs::create_dir_all(ws.join(DIR).join("d")).unwrap();
        save(
            ws,
            &RefDoc {
                id: "d".into(),
                title: "D".into(),
                tag: None,
                source: "d.pdf".into(),
                feature: Some("f".into()),
                ignored: false,
                pages: 2,
                imported_at: 0,
                images: vec![img("img/a.png", 1), img("img/b.png", 2)],
            },
        )
        .unwrap();
        let up = set_scene_upload(ws, "f", "img/b.png", Some("x.png".into())).unwrap();
        assert_eq!(up.scenes[0].uploaded.as_deref(), Some("x.png"));
        assert_eq!(up.scenes[0].status, "not_taken");

        // A capture of the other scene keeps the upload and the order of the manual.
        let mut a = up.scenes[0].clone();
        a.image = "img/a.png".into();
        a.uploaded = None;
        a.status = "reproduced".into();
        let merged = merge_scene_shots(
            ws,
            SceneShots {
                feature: "f".into(),
                doc: "d".into(),
                taken_at: 1,
                scenes: vec![a],
            },
        );
        let order: Vec<_> = merged.scenes.iter().map(|s| s.image.as_str()).collect();
        assert_eq!(order, ["img/a.png", "img/b.png"]);
        assert_eq!(merged.scenes[1].uploaded.as_deref(), Some("x.png"));
        save_scene_shots(ws, &merged).unwrap();

        let cleared = set_scene_upload(ws, "f", "img/b.png", None).unwrap();
        assert!(cleared.scenes.iter().all(|s| s.uploaded.is_none()));
        let chosen = set_scene_choice(ws, "f", "img/a.png", Some("old".into())).unwrap();
        assert_eq!(chosen.scenes[0].choice.as_deref(), Some("old"));
        assert!(set_scene_choice(ws, "f", "img/a.png", Some("x".into())).is_err());
    }

    #[test]
    fn titles_from_export_names() {
        assert_eq!(
            title_of("ACME-(CRM)보고서 생성-140726-002812"),
            ("보고서 생성".into(), Some("CRM".into()))
        );
        assert_eq!(
            title_of("ACME-주문 관리-130726-072621"),
            ("주문 관리".into(), None)
        );
        assert_eq!(title_of("사용자 관리"), ("사용자 관리".into(), None));
    }

    #[test]
    fn tags_pick_between_same_named_features() {
        let fs: Vec<FeatureRef> = vec![
            ("crm-report", "보고서 생성", Some("영업")),
            ("bi-report", "보고서 생성", Some("BI")),
            ("orders", "주문 관리", Some("주문")),
        ];
        assert_eq!(
            match_feature("주문 관리", None, &fs).as_deref(),
            Some("orders")
        );
        assert_eq!(
            match_feature("보고서 생성", Some("BI"), &fs).as_deref(),
            Some("bi-report")
        );
        assert_eq!(
            match_feature("보고서 생성", Some("CRM"), &fs).as_deref(),
            Some("crm-report")
        );
        assert_eq!(match_feature("보고서 생성", None, &fs), None);
        assert_eq!(match_feature("모델 레지스트리", None, &fs), None);
    }

    #[test]
    fn parses_pdftohtml_xml() {
        let xml = r##"<pdf2xml><page number="1" position="absolute" top="0" left="0" height="842" width="596">
<fontspec id="0" size="16" family="A" color="#000"/><fontspec id="1" size="11" family="A" color="#000"/>
<text top="40" left="50" width="80" height="18" font="0"><b>등록 프로세스</b></text>
<text top="70" left="50" width="10" height="13" font="1">우</text><text top="70" left="60" width="10" height="13" font="1">측</text>
<text top="69" left="70" width="4" height="13" font="1"> </text>
<text top="70" left="74" width="60" height="13" font="1">버튼을 누릅니다</text>
<image top="100" left="50" width="400" height="200" src="x/p-1_1.png"/>
<text top="305" left="200" width="40" height="13" font="1">비활성화</text>
</page></pdf2xml>"##;
        let items = parse_xml(xml);
        let lines = lines_of(&items);
        assert_eq!(lines[0].text, "등록 프로세스");
        assert_eq!(lines[1].text, "우측 버튼을 누릅니다");
        assert!(
            items
                .iter()
                .any(|i| matches!(i, Item::Image { width: 400, .. }))
        );
    }
}
