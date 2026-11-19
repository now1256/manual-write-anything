//! Reading one feature's code: its facts (map facts and checked code facts) in
//! `manual/<key>.json`, what manuals are written from.

use std::path::{Path, PathBuf};

use mwa_core::Dispatcher;
use mwa_productmap::graph::Project;
use mwa_productmap::map::{Feature, ProductMap};

use crate::ManualError;
use crate::facts;
use crate::model::{Fact, FactKind, ManualRun};
use crate::slice::{self, Slice};

pub struct Options<'a> {
    pub workspace: &'a Path,
    pub feature: &'a str,
    pub frontend_root: &'a Path,
    pub backend_root: &'a Path,
    pub backend_code_dir: &'a str,
    pub alias_prefix: &'a str,
    pub stop_dirs: &'a [&'a str],
    /// Example manual to learn the template from; otherwise the saved one is used.
    pub template_example: Option<crate::reference::Example>,
    /// (name, markdown) of an existing page for this feature to compare against.
    pub compare_with: Option<(String, String)>,
    /// Captured screenshots available: (shot id, what it shows).
    pub shots: Vec<(String, String)>,
}

pub const TEMPLATE_FILE: &str = "template/schema.json";

pub fn menu_path(f: &Feature) -> String {
    let mut parts = vec![f.license_menu.clone()];
    if let Some(g) = &f.group
        && g != &f.license_menu
    {
        parts.push(g.clone());
    }
    parts.push(f.label.clone());
    parts.join(" > ")
}

/// Deterministic facts from the product map: menu path and permissions (with enforcement).
pub fn map_facts(map: &ProductMap, f: &Feature) -> Vec<Fact> {
    let mut out = vec![Fact {
        id: "M01".into(),
        statement: format!(
            "메뉴 경로는 {}이고 화면 주소는 {}입니다.",
            menu_path(f),
            f.href
        ),
        kind: FactKind::Label,
        source: None,
        basis: "deterministic".into(),
    }];
    let ui: Vec<&String> = f
        .screens
        .iter()
        .flat_map(|s| s.ui_permissions.iter())
        .collect();
    let mut codes: Vec<&String> = f.permissions.iter().chain(ui).collect();
    codes.sort();
    codes.dedup();
    for (i, code) in codes.iter().enumerate() {
        let p = map.permissions.iter().find(|p| &p.code == *code);
        let label = p.map_or("라벨 없음".to_string(), |p| p.label.clone());
        let menu = f.permissions.contains(code);
        let enforced = p.is_some_and(|p| !p.server.is_empty());
        let statement = format!(
            "권한 {code}({label}){}. 서버는 이 권한을 {}.",
            if menu {
                "이 없으면 이 메뉴가 화면에 보이지 않습니다"
            } else {
                "은 화면 안의 일부 버튼·동작 표시에 쓰입니다"
            },
            if enforced {
                "직접 검사합니다"
            } else {
                "검사하지 않습니다(화면에서만 가림)"
            }
        );
        out.push(Fact {
            id: format!("P{:02}", i + 1),
            statement,
            kind: FactKind::Permission,
            source: None,
            basis: "deterministic".into(),
        });
    }
    out
}

pub fn build_slice(
    map: &ProductMap,
    f: &Feature,
    opts: &Options<'_>,
) -> Result<Slice, ManualError> {
    let mut project = Project::new(opts.frontend_root, opts.alias_prefix);
    let pages: Vec<PathBuf> = f
        .screens
        .iter()
        .map(|s| opts.frontend_root.join(&s.file))
        .collect();
    let mut files = slice::frontend(&mut project, &pages, opts.stop_dirs)?;
    let ops: Vec<String> = f
        .screens
        .iter()
        .flat_map(|s| s.apis.iter())
        .filter_map(|a| a.operation.clone())
        .collect();
    files.extend(slice::backend(
        opts.backend_root,
        opts.backend_code_dir,
        &ops,
    )?);
    let _ = map;
    Ok(Slice { files })
}

fn write_json(path: &Path, v: &impl serde::Serialize) -> Result<(), ManualError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| ManualError::Io(dir.to_path_buf(), e))?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(v).expect("serializes") + "\n",
    )
    .map_err(|e| ManualError::Io(path.to_path_buf(), e))
}

pub fn load_run(workspace: &Path, key: &str) -> Option<ManualRun> {
    serde_json::from_str(
        &std::fs::read_to_string(workspace.join(format!("manual/{key}.json"))).ok()?,
    )
    .ok()
}

/// Reads a feature's code (D-045): facts from the product map and from the code, each quoted
/// line checked. What the editable manual is written from; no draft is written any more.
pub async fn read_code(
    d: &Dispatcher,
    map: &ProductMap,
    opts: Options<'_>,
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<ManualRun, ManualError> {
    let f = map
        .features
        .iter()
        .find(|f| f.key == opts.feature)
        .ok_or_else(|| {
            ManualError::Invalid(format!("지도에 없는 기능 키입니다: {}", opts.feature))
        })?;
    let spent_before = d.spent_usd();
    let slice = build_slice(map, f, &opts)?;
    progress(format!(
        "코드 조각: 파일 {}개, {}줄",
        slice.files.len(),
        slice.total_lines()
    ));
    let mut all_facts = map_facts(map, f);
    let extracted = facts::extract(d, &slice, &f.label, 60_000, progress).await?;
    progress(format!(
        "사실 {}개 (근거 줄이 맞지 않아 버림 {})",
        extracted.facts.len(),
        extracted.dropped.len()
    ));
    all_facts.extend(extracted.facts);
    let run = ManualRun {
        feature: f.key.clone(),
        title: f.label.clone(),
        generated_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|x| x.as_secs())
            .unwrap_or(0),
        models: [mwa_protocol::Role::Worker]
            .iter()
            .filter_map(|r| d.role_model(*r).map(|(b, m)| format!("{r}={b}·{m}")))
            .collect(),
        cost_usd: d.spent_usd() - spent_before,
        dropped_facts: extracted.dropped.len(),
        facts: all_facts,
        blocks: Vec::new(),
        checks: Vec::new(),
        compare: None,
        compare_source: None,
    };
    write_json(&opts.workspace.join(format!("manual/{}.json", f.key)), &run)?;
    Ok(run)
}

/// Summary lines for CLI/UI.
pub fn summary(run: &ManualRun) -> Vec<String> {
    if run.blocks.is_empty() {
        return vec![format!(
            "{}: 코드 사실 {}개 (근거 줄이 맞지 않아 버림 {}) · 사용량 정가 환산 ${:.3}",
            run.title,
            run.facts.len(),
            run.dropped_facts,
            run.cost_usd
        )];
    }
    let count = |v: &str| run.checks.iter().filter(|c| c.verdict == v).count();
    let issues: usize = run.checks.iter().map(|c| c.issues.len()).sum();
    let todos = run
        .blocks
        .iter()
        .filter(|b| b.kind == crate::model::BlockKind::Todo)
        .count();
    let mut out = vec![
        format!(
            "{} 매뉴얼: 블록 {}개, 사실 {}개 (근거 불일치로 버림 {})",
            run.title,
            run.blocks.len(),
            run.facts.len(),
            run.dropped_facts
        ),
        format!(
            "검증: 지지 {} · 일부 {} · 근거 없음 {} · 결정적 문제 {}건 · 확인 필요 블록 {}",
            count("supported"),
            count("partial"),
            count("unsupported"),
            issues,
            todos
        ),
    ];
    if let Some(items) = &run.compare {
        let c = |s: &str| items.iter().filter(|i| i.status == s).count();
        out.push(format!(
            "기존 매뉴얼 비교: 같음 {} · 충돌 {} · 기존에만 {} · 새로 추가 {}",
            c("same"),
            c("conflict"),
            c("only_existing"),
            c("only_generated")
        ));
        let conflicts: Vec<&crate::model::CompareItem> =
            items.iter().filter(|i| i.status == "conflict").collect();
        let ours_wrong = conflicts
            .iter()
            .filter(|i| i.code_supports.as_deref() == Some("existing"))
            .count();
        let theirs_outdated = conflicts
            .iter()
            .filter(|i| i.code_supports.as_deref() == Some("generated"))
            .count();
        out.push(format!(
            "충돌 판정: 코드가 생성본을 지지 {theirs_outdated}건(기존 매뉴얼 갱신 필요) · 코드가 기존을 지지 {ours_wrong}건(생성본 오류) · 판단 불가 {}건",
            conflicts.len() - ours_wrong - theirs_outdated
        ));
    }
    out.push(format!(
        "구독 사용량(정가 환산, 청구 없음) ${:.3} · {}",
        run.cost_usd,
        run.models.join(", ")
    ));
    out
}
