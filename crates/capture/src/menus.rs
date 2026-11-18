//! Batch capture of menu screens: N runs, stability check, manifest at `shots/menus/manifest.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::diff::{diff, dimensions};
use crate::shoot::{Blocked, Job, JobShot, ShootError, ShotEvent, run_sidecar};

/// Pixel-change ratio under which two runs count as the same image.
pub const STABLE_THRESHOLD: f64 = 0.005;

#[derive(Debug, Clone)]
pub struct Target {
    pub key: String,
    pub label: String,
    pub route: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShotRecord {
    pub key: String,
    pub label: String,
    pub route: String,
    /// Relative to the workspace, when the capture succeeded.
    pub file: Option<String>,
    pub ok: bool,
    pub error: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub sha256: Option<String>,
    /// Non-GET requests the screen tried and we blocked (union over runs).
    pub blocked: Vec<Blocked>,
    pub console_errors: Vec<String>,
    /// Pixel-change ratio between run 1 and run 2, if both succeeded.
    pub stable_diff: Option<f64>,
    /// Where run 1 and run 2 differ, `[x, y, w, h]` in image pixels.
    #[serde(default)]
    pub diff_box: Option<[u32; 4]>,
    /// Second-run image kept when the runs differ, for inspection.
    #[serde(default)]
    pub unstable_file: Option<String>,
    /// Errors from later runs (the first run's error is `error`).
    #[serde(default)]
    pub run_errors: Vec<String>,
    /// Toast notices visible in the saved (first-run) image.
    #[serde(default)]
    pub toasts: Vec<String>,
    /// Why this image should not go into a manual as is.
    #[serde(default)]
    pub needs_retake: Option<String>,
    /// `"world"` when demo data (D-028) was served; such images are retaken on a real stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    pub idle_timed_out: bool,
    pub ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub taken_at: u64,
    pub base_url: String,
    pub domain: Option<String>,
    pub frontend_sha: Option<String>,
    pub runs: u32,
    pub viewport: [u32; 2],
    pub scale: f64,
    pub shots: Vec<ShotRecord>,
}

impl Manifest {
    pub fn summary(&self) -> Vec<String> {
        let ok = self.shots.iter().filter(|s| s.ok).count();
        let compared: Vec<f64> = self.shots.iter().filter_map(|s| s.stable_diff).collect();
        let stable = compared.iter().filter(|d| **d <= STABLE_THRESHOLD).count();
        let blocked: usize = self.shots.iter().map(|s| s.blocked.len()).sum();
        let mut out = vec![format!("촬영 성공 {ok}/{}", self.shots.len())];
        if self.runs > 1 {
            out.push(format!(
                "2회 촬영 일치(픽셀 차이 {:.1}% 이하) {stable}/{} ({:.0}%)",
                STABLE_THRESHOLD * 100.0,
                compared.len(),
                if compared.is_empty() {
                    0.0
                } else {
                    100.0 * stable as f64 / compared.len() as f64
                }
            ));
        }
        out.push(format!(
            "차단한 쓰기 요청 {blocked}건 (공유 서버로 나가지 않음)"
        ));
        for s in self.shots.iter().filter(|s| !s.ok) {
            out.push(format!(
                "  실패 {} ({}): {}",
                s.label,
                s.route,
                s.error.as_deref().unwrap_or("-")
            ));
        }
        for s in self
            .shots
            .iter()
            .filter(|s| s.stable_diff.is_some_and(|d| d > STABLE_THRESHOLD))
        {
            let area = s
                .diff_box
                .map(|[x, y, w, h]| format!(", 바뀐 영역 x{x} y{y} {w}×{h}"))
                .unwrap_or_default();
            out.push(format!(
                "  흔들림 {} ({}): 픽셀 차이 {:.2}%{area}",
                s.label,
                s.route,
                s.stable_diff.unwrap_or(0.0) * 100.0
            ));
        }
        for s in self.shots.iter().filter(|s| !s.run_errors.is_empty()) {
            out.push(format!(
                "  회차 실패 {} ({}): {}",
                s.label,
                s.route,
                s.run_errors.join("; ")
            ));
        }
        for s in self.shots.iter().filter(|s| !s.toasts.is_empty()) {
            out.push(format!(
                "  알림 노출 {} ({}): {}",
                s.label,
                s.route,
                s.toasts.join(" | ")
            ));
        }
        let retake = self
            .shots
            .iter()
            .filter(|s| s.needs_retake.is_some())
            .count();
        if retake > 0 {
            out.push(format!(
                "매뉴얼에 그대로 쓰기 어려운 화면 {retake}개 (다시 찍기 필요)"
            ));
        }
        for s in self.shots.iter().filter(|s| !s.blocked.is_empty()) {
            let reqs: Vec<String> = s
                .blocked
                .iter()
                .map(|b| format!("{} {}", b.method, b.url))
                .collect();
            out.push(format!(
                "  쓰기 차단 {} ({}): {}",
                s.label,
                s.route,
                reqs.join(", ")
            ));
        }
        out
    }
}

pub struct Options<'a> {
    /// Refuse to start below this much available memory (MB).
    pub min_start_mb: u64,
    /// Stop between shots below this much available memory (MB).
    pub abort_below_mb: u64,
    pub workspace: &'a Path,
    pub node: &'a str,
    pub script: &'a Path,
    /// Job template; `shots` is filled per run.
    pub job: Job,
    pub targets: Vec<Target>,
    pub runs: u32,
    pub frontend_sha: Option<String>,
}

pub const MANIFEST: &str = "shots/menus/manifest.json";

pub async fn capture(
    opts: Options<'_>,
    mut progress: impl FnMut(String),
) -> Result<Manifest, ShootError> {
    if let Some(available) = crate::mem::available_mb()
        && available < opts.min_start_mb
    {
        return Err(ShootError::LowMemory {
            available,
            limit: opts.min_start_mb,
        });
    }
    let guard = crate::mem::MemGuard::new(opts.abort_below_mb);
    let stamp = opts.job.fixed_time;
    let tmp = opts.workspace.join(format!(".mwa/shots/{stamp}"));
    let final_dir = opts.workspace.join("shots/menus");
    std::fs::create_dir_all(&final_dir)?;

    let mut runs: Vec<Vec<ShotEvent>> = Vec::new();
    for run in 1..=opts.runs.max(1) {
        let dir = tmp.join(format!("run{run}"));
        std::fs::create_dir_all(&dir)?;
        let mut job = opts.job.clone();
        job.shots = opts
            .targets
            .iter()
            .map(|t| JobShot {
                id: t.key.clone(),
                path: t.route.clone(),
                out: dir.join(format!("{}.png", t.key)),
                steps: Vec::new(),
                capture: "page".into(),
                aria: false,
            })
            .collect();
        let total = job.shots.len();
        let mut n = 0;
        let events = run_sidecar(opts.node, opts.script, &job, Some(guard), |ev| {
            n += 1;
            let mark = if ev.ok { "✓" } else { "✗" };
            progress(format!(
                "[{run}회차 {n}/{total}] {mark} {} ({} ms)",
                ev.id, ev.ms
            ));
        })
        .await?;
        runs.push(events);
    }

    let path_of = |run: usize, key: &str| tmp.join(format!("run{run}/{key}.png"));
    let mut shots = Vec::new();
    for t in &opts.targets {
        let evs: Vec<&ShotEvent> = runs
            .iter()
            .filter_map(|r| r.iter().find(|e| e.id == t.key))
            .collect();
        let first = evs.first().copied();
        let ok = first.is_some_and(|e| e.ok);
        let mut blocked: Vec<Blocked> = Vec::new();
        for e in &evs {
            for b in &e.blocked {
                if !blocked.contains(b) {
                    blocked.push(b.clone());
                }
            }
        }
        let mut rec = ShotRecord {
            key: t.key.clone(),
            label: t.label.clone(),
            route: t.route.clone(),
            file: None,
            ok,
            error: first
                .and_then(|e| e.error.clone())
                .or_else(|| first.is_none().then(|| "결과 없음".into())),
            width: None,
            height: None,
            sha256: None,
            blocked,
            console_errors: first.map(|e| e.console_errors.clone()).unwrap_or_default(),
            stable_diff: None,
            diff_box: None,
            unstable_file: None,
            run_errors: evs
                .iter()
                .enumerate()
                .skip(1)
                .filter(|(_, e)| !e.ok)
                .map(|(i, e)| format!("{}회차: {}", i + 1, e.error.as_deref().unwrap_or("실패")))
                .chain((evs.len() < runs.len()).then(|| "일부 회차 결과 없음".to_string()))
                .collect(),
            toasts: first.map(|e| e.toasts.clone()).unwrap_or_default(),
            needs_retake: None,
            data: evs
                .iter()
                .any(|e| !e.mocked.is_empty())
                .then(|| "world".to_string()),
            idle_timed_out: evs.iter().any(|e| e.idle_timed_out),
            ms: first.map_or(0, |e| e.ms),
        };
        rec.needs_retake = if !rec.blocked.is_empty() {
            Some("화면이 쓰기 요청을 보내 차단했습니다. 공유 서버에서는 결과가 달라질 수 있어 전용 스택에서 다시 찍어야 합니다".into())
        } else if rec.toasts.iter().any(|t| t.starts_with("error")) {
            Some("오류 알림이 떠 있는 상태로 찍혔습니다".into())
        } else {
            None
        };
        if ok {
            let src = path_of(1, &t.key);
            if evs.len() > 1 && evs[1].ok {
                let second = path_of(2, &t.key);
                if let Ok(d) = diff(&src, &second) {
                    rec.stable_diff = Some(d.ratio);
                    rec.diff_box = d.bbox;
                    if d.ratio > STABLE_THRESHOLD {
                        let debug = final_dir.join("_unstable");
                        std::fs::create_dir_all(&debug)?;
                        std::fs::copy(&second, debug.join(format!("{}.run2.png", t.key)))?;
                        rec.unstable_file =
                            Some(format!("shots/menus/_unstable/{}.run2.png", t.key));
                    }
                }
            }
            let dest = final_dir.join(format!("{}.png", t.key));
            std::fs::copy(&src, &dest)?;
            let bytes = std::fs::read(&dest)?;
            rec.sha256 = Some(
                Sha256::digest(&bytes)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
            );
            if let Some((w, h)) = dimensions(&dest) {
                rec.width = Some(w);
                rec.height = Some(h);
            }
            rec.file = Some(format!("shots/menus/{}.png", t.key));
        }
        shots.push(rec);
    }
    let _ = std::fs::remove_dir_all(&tmp);

    // A partial run (`--only`) replaces just its own records in the previous manifest.
    if let Some(old) = load_manifest(opts.workspace) {
        let mut merged: Vec<ShotRecord> = old
            .shots
            .into_iter()
            .map(|r| shots.iter().find(|s| s.key == r.key).cloned().unwrap_or(r))
            .collect();
        let added: Vec<ShotRecord> = shots
            .iter()
            .filter(|s| !merged.iter().any(|m| m.key == s.key))
            .cloned()
            .collect();
        merged.extend(added);
        shots = merged;
    }

    let manifest = Manifest {
        taken_at: stamp / 1000,
        base_url: opts.job.base_url.clone(),
        domain: opts.job.domain.clone(),
        frontend_sha: opts.frontend_sha.clone(),
        runs: opts.runs.max(1),
        viewport: opts.job.viewport,
        scale: opts.job.scale,
        shots,
    };
    std::fs::write(
        opts.workspace.join(MANIFEST),
        serde_json::to_string_pretty(&manifest).expect("manifest serializes") + "\n",
    )?;
    Ok(manifest)
}

pub fn load_manifest(workspace: &Path) -> Option<Manifest> {
    serde_json::from_str(&std::fs::read_to_string(workspace.join(MANIFEST)).ok()?).ok()
}

/// Resolves a file under `shots/` for serving; rejects anything that is not a plain relative path.
pub fn shot_file(workspace: &Path, rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    if !p
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        return None;
    }
    let full = workspace.join("shots").join(p);
    full.is_file().then_some(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shot_file_stays_inside_shots() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("shots/menus")).unwrap();
        std::fs::write(dir.path().join("shots/menus/a.png"), "x").unwrap();
        std::fs::write(dir.path().join("mwa.toml"), "secret").unwrap();
        assert!(shot_file(dir.path(), "menus/a.png").is_some());
        assert!(shot_file(dir.path(), "../mwa.toml").is_none());
        assert!(shot_file(dir.path(), "/etc/passwd").is_none());
        assert!(shot_file(dir.path(), "menus/missing.png").is_none());
    }
}
