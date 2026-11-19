//! Manual versions (D-031 ③): every generation is kept with the source commits it was written
//! from, and compared with the previous version. The old manual (PDF) is version 0, compared by
//! its outline.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ManualError;
use crate::model::{BlockKind, ManualRun};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceAt {
    pub name: String,
    pub sha: String,
    #[serde(default)]
    pub branch: Option<String>,
    /// The checkout had uncommitted changes.
    #[serde(default)]
    pub preview: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionMeta {
    pub n: u32,
    pub created_at: u64,
    pub sources: Vec<SourceAt>,
    /// Where the format was learned from.
    pub template: String,
    pub blocks: usize,
    pub images: usize,
    pub supported: usize,
    pub partial: usize,
    pub unsupported: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stored {
    meta: VersionMeta,
    run: ManualRun,
}

fn dir(ws: &Path, feature: &str) -> PathBuf {
    ws.join("manual/versions").join(feature)
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> ManualError + '_ {
    move |e| ManualError::Io(path.to_path_buf(), e)
}

fn stored(ws: &Path, feature: &str) -> Vec<Stored> {
    let mut out: Vec<Stored> = std::fs::read_dir(dir(ws, feature))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .filter_map(|t| serde_json::from_str(&t).ok())
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|s| s.meta.n);
    out
}

/// Saves `run` as the next version of its feature and returns its metadata.
pub fn save(
    ws: &Path,
    run: &ManualRun,
    sources: Vec<SourceAt>,
    template: &str,
) -> Result<VersionMeta, ManualError> {
    let n = stored(ws, &run.feature).last().map_or(1, |s| s.meta.n + 1);
    let verdict = |v: &str| run.checks.iter().filter(|c| c.verdict == v).count();
    let meta = VersionMeta {
        n,
        created_at: run.generated_at,
        sources,
        template: template.to_string(),
        blocks: run.blocks.len(),
        images: run
            .blocks
            .iter()
            .filter(|b| b.kind == BlockKind::Image)
            .count(),
        supported: verdict("supported"),
        partial: verdict("partial"),
        unsupported: verdict("unsupported"),
    };
    let d = dir(ws, &run.feature);
    std::fs::create_dir_all(&d).map_err(io(&d))?;
    let path = d.join(format!("v{n}.json"));
    let s = Stored {
        meta: meta.clone(),
        run: run.clone(),
    };
    std::fs::write(&path, serde_json::to_string_pretty(&s).expect("serializes"))
        .map_err(io(&path))?;
    Ok(meta)
}

pub fn list(ws: &Path, feature: &str) -> Vec<VersionMeta> {
    stored(ws, feature).into_iter().map(|s| s.meta).collect()
}

pub fn load(ws: &Path, feature: &str, n: u32) -> Option<ManualRun> {
    stored(ws, feature)
        .into_iter()
        .find(|s| s.meta.n == n)
        .map(|s| s.run)
}

/// `---\n\n# 제목` → "제목".
fn heading_text(markdown: &str) -> Option<String> {
    markdown
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('#'))
        .map(|l| l.trim_start().trim_start_matches('#').trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Headings of a generated manual, in order (separators dropped).
pub fn outline(run: &ManualRun) -> Vec<String> {
    run.blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Heading)
        .filter_map(|b| heading_text(&b.markdown))
        .collect()
}

/// Headings of an imported old manual: the outline on its first page.
pub fn reference_outline(ws: &Path, doc_id: &str) -> Vec<String> {
    let text = std::fs::read_to_string(crate::reference::doc_dir(ws, doc_id).join("text.txt"))
        .unwrap_or_default();
    let first = text.split('\u{c}').next().unwrap_or("");
    let mut out: Vec<String> = Vec::new();
    for t in first.lines().skip(1).map(str::trim) {
        let heading_like = !t.is_empty()
            && t.chars().count() <= 30
            && t.chars().filter(|c| c.is_alphanumeric()).count() >= 2
            && !t.ends_with('.')
            && !t.ends_with('다')
            // Table rows in layout text have wide gaps between cells.
            && !t.contains("   ");
        if !heading_like {
            continue;
        }
        // The body starts where an outline entry appears again.
        if out.iter().any(|o| o == t) {
            break;
        }
        out.push(t.to_string());
    }
    out
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// What changed between two versions of one feature's manual.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Diff {
    /// What this version is compared with ("v2", "기존 매뉴얼(PDF)").
    pub against: String,
    /// Headings only in the newer version.
    pub added: Vec<String>,
    /// Headings only in the older version.
    pub removed: Vec<String>,
    /// Headings in both whose section text changed.
    pub changed: Vec<String>,
    pub images_added: usize,
    pub images_removed: usize,
}

/// Section text under each heading (for "changed").
fn sections(run: &ManualRun) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for b in &run.blocks {
        if b.kind == BlockKind::Heading {
            out.push((heading_text(&b.markdown).unwrap_or_default(), String::new()));
        } else if let Some(last) = out.last_mut() {
            last.1.push_str(&b.markdown);
            last.1.push('\n');
        }
    }
    out
}

pub fn diff_runs(older: &ManualRun, newer: &ManualRun, against: &str) -> Diff {
    let (a, b) = (sections(older), sections(newer));
    let find = |list: &[(String, String)], h: &str| {
        list.iter()
            .find(|(t, _)| norm(t) == norm(h))
            .map(|(_, body)| body.clone())
    };
    let images = |r: &ManualRun| -> std::collections::BTreeSet<String> {
        r.blocks
            .iter()
            .filter(|x| x.kind == BlockKind::Image)
            .filter_map(|x| x.shot.clone())
            .collect()
    };
    let (ia, ib) = (images(older), images(newer));
    Diff {
        against: against.to_string(),
        added: b
            .iter()
            .filter(|(t, _)| find(&a, t).is_none())
            .map(|(t, _)| t.clone())
            .collect(),
        removed: a
            .iter()
            .filter(|(t, _)| find(&b, t).is_none())
            .map(|(t, _)| t.clone())
            .collect(),
        changed: b
            .iter()
            .filter(|(t, body)| find(&a, t).is_some_and(|old| norm(&old) != norm(body)))
            .map(|(t, _)| t.clone())
            .collect(),
        images_added: ib.difference(&ia).count(),
        images_removed: ia.difference(&ib).count(),
    }
}

/// Compares a version with the old manual's outline (headings only).
pub fn diff_reference(ref_outline: &[String], newer: &ManualRun) -> Diff {
    let ours = outline(newer);
    let has = |list: &[String], h: &str| list.iter().any(|x| norm(x) == norm(h));
    Diff {
        against: "기존 매뉴얼(PDF)".into(),
        added: ours
            .iter()
            .filter(|h| !has(ref_outline, h))
            .cloned()
            .collect(),
        removed: ref_outline
            .iter()
            .filter(|h| !has(&ours, h))
            .cloned()
            .collect(),
        changed: Vec::new(),
        images_added: 0,
        images_removed: 0,
    }
}

/// Every version with its diff against the one before (v1 against the old manual, if any).
pub fn history(ws: &Path, feature: &str) -> Vec<(VersionMeta, Diff)> {
    let all = stored(ws, feature);
    let reference = crate::reference::for_feature(ws, feature)
        .map(|d| reference_outline(ws, &d.id))
        .filter(|o| !o.is_empty());
    let mut out = Vec::new();
    for (i, s) in all.iter().enumerate() {
        let diff = match i.checked_sub(1).and_then(|p| all.get(p)) {
            Some(prev) => diff_runs(&prev.run, &s.run, &format!("v{}", prev.meta.n)),
            None => match &reference {
                Some(o) => diff_reference(o, &s.run),
                None => Diff {
                    against: "(처음 버전)".into(),
                    ..Default::default()
                },
            },
        };
        out.push((s.meta.clone(), diff));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Block;

    fn run(blocks: &[(BlockKind, &str)]) -> ManualRun {
        ManualRun {
            feature: "f".into(),
            title: "F".into(),
            generated_at: 1,
            models: vec![],
            cost_usd: 0.0,
            facts: vec![],
            dropped_facts: 0,
            blocks: blocks
                .iter()
                .enumerate()
                .map(|(i, (k, md))| Block {
                    id: format!("b{i}"),
                    kind: k.clone(),
                    markdown: md.to_string(),
                    facts: vec![],
                    shot: (*k == BlockKind::Image).then(|| format!("detail/f/{i}")),
                })
                .collect(),
            checks: vec![],
            compare: None,
            compare_source: None,
        }
    }

    #[test]
    fn versions_number_and_diff() {
        let dir = tempfile::tempdir().unwrap();
        let v1 = run(&[
            (BlockKind::Heading, "# 권한 안내"),
            (BlockKind::Paragraph, "관리자만 씁니다."),
            (BlockKind::Heading, "---\n\n# 용어 설명"),
            (BlockKind::Paragraph, "주문 시스템"),
        ]);
        let v2 = run(&[
            (BlockKind::Heading, "# 권한 안내"),
            (BlockKind::Paragraph, "관리자와 담당자가 씁니다."),
            (BlockKind::Heading, "# 화면 안내"),
            (BlockKind::Image, "목록"),
        ]);
        assert_eq!(save(dir.path(), &v1, vec![], "t").unwrap().n, 1);
        assert_eq!(save(dir.path(), &v2, vec![], "t").unwrap().n, 2);
        let h = history(dir.path(), "f");
        assert_eq!(h.len(), 2);
        let d = &h[1].1;
        assert_eq!(d.against, "v1");
        assert_eq!(d.added, ["화면 안내"]);
        assert_eq!(d.removed, ["용어 설명"]);
        assert_eq!(d.changed, ["권한 안내"]);
        assert_eq!(d.images_added, 1);

        let pdf = dir.path().join("reference/doc");
        std::fs::create_dir_all(&pdf).unwrap();
        std::fs::write(
            pdf.join("text.txt"),
            "주문 관리\n\n  소개 문단은 길어서 목차가 아닙니다. 이렇게 끝납니다.\n 권한 안내\n 용어 설명\n\n권한 안내\n  역할      책임\n\u{c}둘째 쪽",
        )
        .unwrap();
        assert_eq!(
            reference_outline(dir.path(), "doc"),
            ["권한 안내", "용어 설명"]
        );
        let r = diff_reference(&["권한 안내".into(), "용어 설명".into()], &v2);
        assert_eq!(r.removed, ["용어 설명"]);
        assert_eq!(r.added, ["화면 안내"]);
    }
}
