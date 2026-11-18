//! Detail shots (M4a T4): screens reached by clicking from a page, such as dialogs and
//! detail pages. A plan is a list of role/name steps; once it works it is replayed as is.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diff::diff;
use crate::menus::STABLE_THRESHOLD;
use crate::shoot::{Blocked, Job, JobShot, ShootError, ShotEvent, Step, run_sidecar};

/// How to reach and capture one screen the manual asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// Manual block id this picture fills.
    pub todo: String,
    /// What the picture should show (from the manual).
    pub what: String,
    /// Starting page.
    pub path: String,
    pub steps: Vec<Step>,
    /// `page`, `full` or `dialog`.
    pub capture: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetailRecord {
    pub plan: Plan,
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub failed_step: Option<usize>,
    /// Relative to the workspace.
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub stable_diff: Option<f64>,
    #[serde(default)]
    pub blocked: Vec<Blocked>,
    #[serde(default)]
    pub toasts: Vec<String>,
    #[serde(default)]
    pub needs_retake: Option<String>,
    /// `"world"` when demo data was served (D-028).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// Accessibility snapshot at the failing step, for re-planning. Not persisted.
    #[serde(skip)]
    pub aria: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DetailManifest {
    pub feature: String,
    pub taken_at: u64,
    pub records: Vec<DetailRecord>,
}

pub fn manifest_path(ws: &Path, feature: &str) -> PathBuf {
    ws.join(format!("shots/detail/{feature}/manifest.json"))
}

pub fn load_manifest(ws: &Path, feature: &str) -> Option<DetailManifest> {
    let text = std::fs::read_to_string(manifest_path(ws, feature)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Saves `records`, replacing older records for the same blocks.
pub fn save_manifest(
    ws: &Path,
    feature: &str,
    taken_at: u64,
    records: &[DetailRecord],
) -> Result<DetailManifest, ShootError> {
    let mut all = load_manifest(ws, feature)
        .map(|m| m.records)
        .unwrap_or_default();
    for r in records {
        all.retain(|o| o.plan.todo != r.plan.todo);
        all.push(r.clone());
    }
    let m = DetailManifest {
        feature: feature.to_string(),
        taken_at,
        records: all,
    };
    let path = manifest_path(ws, feature);
    std::fs::create_dir_all(path.parent().expect("has parent"))?;
    std::fs::write(&path, serde_json::to_string_pretty(&m).expect("serializes"))?;
    Ok(m)
}

/// How to start the capture sidecar.
#[derive(Debug, Clone, Copy)]
pub struct Sidecar<'a> {
    pub node: &'a str,
    pub script: &'a Path,
}

/// Accessibility snapshots of pages as they first appear (for planning).
pub async fn probe(
    sc: Sidecar<'_>,
    job: &Job,
    tmp: &Path,
    paths: &[String],
) -> Result<Vec<ShotEvent>, ShootError> {
    std::fs::create_dir_all(tmp)?;
    let mut job = job.clone();
    job.shots = paths
        .iter()
        .enumerate()
        .map(|(i, p)| JobShot {
            id: p.clone(),
            path: p.clone(),
            out: tmp.join(format!("probe{i}.png")),
            steps: Vec::new(),
            capture: "page".into(),
            aria: true,
        })
        .collect();
    run_sidecar(sc.node, sc.script, &job, None, |_| {}).await
}

/// Runs every plan `runs` times, compares runs and keeps the first run's picture under
/// `shots/detail/<feature>/<todo>.png`.
pub async fn shoot(
    sc: Sidecar<'_>,
    job: &Job,
    ws: &Path,
    feature: &str,
    plans: &[Plan],
    runs: u32,
    mut progress: impl FnMut(String),
) -> Result<Vec<DetailRecord>, ShootError> {
    let stamp = job.fixed_time;
    let tmp = ws.join(format!(".mwa/shots/detail-{stamp}"));
    let final_dir = ws.join(format!("shots/detail/{feature}"));
    std::fs::create_dir_all(&final_dir)?;
    let path_of = |run: u32, todo: &str| tmp.join(format!("run{run}/{todo}.png"));

    let mut events: Vec<Vec<ShotEvent>> = Vec::new();
    for run in 1..=runs.max(1) {
        std::fs::create_dir_all(tmp.join(format!("run{run}")))?;
        let mut j = job.clone();
        j.shots = plans
            .iter()
            .map(|p| JobShot {
                id: p.todo.clone(),
                path: p.path.clone(),
                out: path_of(run, &p.todo),
                steps: p.steps.clone(),
                capture: p.capture.clone(),
                aria: false,
            })
            .collect();
        let total = j.shots.len();
        let mut n = 0;
        events.push(
            run_sidecar(sc.node, sc.script, &j, None, |ev| {
                n += 1;
                let mark = if ev.ok { "✓" } else { "✗" };
                let err = ev
                    .error
                    .as_deref()
                    .map(|e| format!(" — {e}"))
                    .unwrap_or_default();
                progress(format!("[{run}회차 {n}/{total}] {mark} {}{err}", ev.id));
            })
            .await?,
        );
    }

    let mut out = Vec::new();
    for p in plans {
        let evs: Vec<&ShotEvent> = events
            .iter()
            .filter_map(|r| r.iter().find(|e| e.id == p.todo))
            .collect();
        let first = evs.first().copied();
        let mut rec = DetailRecord {
            plan: p.clone(),
            ok: first.is_some_and(|e| e.ok),
            error: first
                .and_then(|e| e.error.clone())
                .or_else(|| first.is_none().then(|| "결과 없음".into())),
            failed_step: first.and_then(|e| e.failed_step),
            file: None,
            stable_diff: None,
            blocked: Vec::new(),
            toasts: first.map(|e| e.toasts.clone()).unwrap_or_default(),
            needs_retake: None,
            data: evs
                .iter()
                .any(|e| !e.mocked.is_empty())
                .then(|| "world".to_string()),
            aria: first.and_then(|e| e.aria.clone()),
        };
        for e in &evs {
            for b in &e.blocked {
                if !rec.blocked.contains(b) {
                    rec.blocked.push(b.clone());
                }
            }
        }
        if rec.ok {
            if evs.len() > 1 && evs[1].ok {
                if let Ok(d) = diff(&path_of(1, &p.todo), &path_of(2, &p.todo)) {
                    rec.stable_diff = Some(d.ratio);
                }
            } else if evs.len() > 1 {
                rec.error = Some(format!(
                    "2회차 실패: {}",
                    evs[1].error.as_deref().unwrap_or("결과 없음")
                ));
            }
            let dest = final_dir.join(format!("{}.png", p.todo));
            std::fs::copy(path_of(1, &p.todo), &dest)?;
            rec.file = Some(format!("shots/detail/{feature}/{}.png", p.todo));
        }
        rec.needs_retake = if !rec.blocked.is_empty() {
            Some("화면이 데모 데이터에 없는 쓰기 요청을 보내 차단했습니다".into())
        } else if rec.toasts.iter().any(|t| t.starts_with("error")) {
            Some("오류 알림이 떠 있는 상태로 찍혔습니다".into())
        } else if rec.stable_diff.is_some_and(|d| d > STABLE_THRESHOLD) {
            Some("두 번 찍은 화면이 다릅니다".into())
        } else if rec.ok && rec.error.is_some() {
            Some("2회차 재생에 실패했습니다".into())
        } else {
            None
        };
        out.push(rec);
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(out)
}
