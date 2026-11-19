//! Plans for detail shots (M4a T4): the writer model turns "📷 캡처 필요 — <what>" blocks into
//! role/name steps, using the real accessibility snapshot of the starting pages. A plan that
//! fails is re-planned from the snapshot taken at the failing step.

use mwa_capture::detail::Plan;
use mwa_capture::shoot::Step;
use mwa_core::Dispatcher;
use mwa_protocol::Role;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ManualError;
use crate::model::{BlockKind, ManualRun};

pub const TODO_PREFIX: &str = "📷 캡처 필요 — ";

/// (block id, what to show) for every screenshot the manual still lacks.
pub fn todos(run: &ManualRun) -> Vec<(String, String)> {
    run.blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Todo)
        .filter_map(|b| {
            let what = b.markdown.trim().strip_prefix(TODO_PREFIX)?;
            Some((b.id.clone(), what.trim().to_string()))
        })
        .collect()
}

/// Turns the todo blocks named in `shots` ((block id, shot id)) into image blocks, saves
/// `manual/<key>.json` and re-renders `manual/<key>.md`. Returns how many blocks changed.
pub fn apply_shots(
    ws: &std::path::Path,
    key: &str,
    shots: &[(String, String)],
) -> Result<usize, ManualError> {
    let Some(mut run) = crate::pipeline::load_run(ws, key) else {
        return Ok(0);
    };
    let mut n = 0;
    for b in run.blocks.iter_mut().filter(|b| b.kind == BlockKind::Todo) {
        let Some((_, shot)) = shots.iter().find(|(id, _)| *id == b.id) else {
            continue;
        };
        let what = b
            .markdown
            .trim()
            .strip_prefix(TODO_PREFIX)
            .unwrap_or(b.markdown.trim())
            .to_string();
        b.kind = BlockKind::Image;
        b.shot = Some(shot.clone());
        b.markdown = what;
        n += 1;
    }
    if n == 0 {
        return Ok(0);
    }
    let title = run.title.clone();
    let captions = |id: &str| {
        if id.starts_with("menus/") {
            format!("{title} 목록 화면 (메뉴 진입 직후)")
        } else {
            String::new()
        }
    };
    let md = crate::render::markdown(&run.title, &run.blocks, "../shots/", &captions);
    let dir = ws.join("manual");
    let write = |name: String, text: String| {
        let path = dir.join(name);
        std::fs::write(&path, text).map_err(|e| ManualError::Io(path, e))
    };
    write(
        format!("{key}.json"),
        serde_json::to_string_pretty(&run).expect("run serializes"),
    )?;
    write(format!("{key}.md"), md)?;
    Ok(n)
}

const SYSTEM: &str = "You plan browser interactions that bring a web product to the screen a Korean user \
manual needs a screenshot of. Steps are executed with Playwright getByRole(role, {name}) and must match exactly \
one element, so copy roles and accessible names from the accessibility snapshot (YAML: `- role \"name\"`). \
When an element is not in the snapshot yet (inside a dialog that opens later), use the screen labels from the \
code. Use `within: \"dialog\"` for elements inside an open dialog, `row` (text of one cell of a table row, such as its name; not several cells joined) to \
pick one row's control, and `near` (a title or text inside the same card/section) when several cards have the same button. \
Bulk actions such as delete usually appear only after selecting rows: check the row's checkbox first, then press \
the toolbar button (often named with a count, e.g. \"삭제 (1)\"; use exact false). Prefer opening a detail page directly by path when the route takes an id from the demo data. \
Inside a dialog never press buttons that commit (저장, 생성, 등록, 삭제, 확인, 적용, 실행, 제출); to show a \
confirmation dialog, press the page button that opens it and stop there. For a wizard step N, fill required \
fields with realistic Korean values and press 다음 to advance. End every plan with a `wait` step for the element \
that proves the target screen is visible (e.g. role dialog, or a heading). capture: `dialog` for dialogs, \
`page` for a screen, `full` only when the target is below the fold of a long page.";

fn step_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "action": {"type": "string", "enum": ["click", "fill", "select", "hover", "wait"]},
            "role": {"type": "string"},
            "name": {"type": "string"},
            "exact": {"type": "boolean"},
            "within": {"type": "string", "enum": ["", "dialog"]},
            "row": {"type": "string"},
            "near": {"type": "string"},
            "value": {"type": "string"}
        },
        "required": ["action", "role", "name", "exact", "within", "row", "near", "value"]
    })
}

fn plan_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "todo": {"type": "string"},
            "path": {"type": "string"},
            "capture": {"type": "string", "enum": ["page", "full", "dialog"]},
            "steps": {"type": "array", "items": step_schema()}
        },
        "required": ["todo", "path", "capture", "steps"]
    })
}

#[derive(Deserialize)]
struct RawPlan {
    todo: String,
    path: String,
    capture: String,
    steps: Vec<Step>,
    #[serde(default)]
    region: Vec<String>,
}

/// What the planner sees about the product.
pub struct Context<'a> {
    pub feature_label: &'a str,
    /// Routes of the feature (`{}` = id).
    pub routes: Vec<String>,
    /// (path, accessibility snapshot) of starting pages.
    pub snapshots: Vec<(String, String)>,
    /// Ids and names the demo data offers, one per line.
    pub data: String,
    /// Labels from the feature's code (buttons, dialog titles, fields).
    pub labels: Vec<String>,
    /// Write requests answered by demo data in the browser (`METHOD /path`).
    pub mocked_writes: Vec<String>,
}

fn context_text(c: &Context) -> String {
    let snaps: Vec<String> = c
        .snapshots
        .iter()
        .map(|(p, a)| format!("### {p}\n{a}"))
        .collect();
    format!(
        "기능: {}\n화면 경로: {}\n\n## 시작 화면의 접근성 트리\n{}\n\n## 화면에 나오는 데이터 (실제 서버 또는 데모, 경로의 id로 쓸 수 있음)\n{}\n\n## 데모 응답이 있는 쓰기 요청 (눌러도 서버에 가지 않고 성공 응답)\n{}\n\n## 코드의 화면 문구\n{}",
        c.feature_label,
        c.routes.join(", "),
        snaps.join("\n\n"),
        c.data,
        if c.mocked_writes.is_empty() {
            "(없음)".into()
        } else {
            c.mocked_writes.join("\n")
        },
        c.labels.join(" | "),
    )
}

fn to_plan(raw: RawPlan, what: &str) -> Plan {
    Plan {
        todo: raw.todo,
        what: what.to_string(),
        path: raw.path,
        steps: raw.steps,
        capture: raw.capture,
        region: raw.region,
        ratio: None,
    }
}

/// One plan per todo.
pub async fn plan(
    d: &Dispatcher,
    ctx: &Context<'_>,
    todos: &[(String, String)],
) -> Result<Vec<Plan>, ManualError> {
    let list: Vec<String> = todos
        .iter()
        .map(|(id, what)| format!("- {id}: {what}"))
        .collect();
    let prompt = format!(
        "{}\n\n## 찍어야 할 화면 (todo id: 내용)\n{}\n\n각 todo마다 계획 하나씩 만드세요.",
        context_text(ctx),
        list.join("\n")
    );
    let schema = json!({
        "type": "object",
        "properties": {"plans": {"type": "array", "items": plan_schema()}},
        "required": ["plans"]
    });
    let out = d
        .call(Role::Writer, SYSTEM, &prompt, &[], Some(&schema))
        .await?;
    #[derive(Deserialize)]
    struct Raw {
        plans: Vec<RawPlan>,
    }
    let raw: Raw = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("촬영 계획: {e}")))?;
    Ok(raw
        .plans
        .into_iter()
        .filter_map(|p| {
            let what = todos.iter().find(|(id, _)| *id == p.todo)?.1.clone();
            Some(to_plan(p, &what))
        })
        .collect())
}

/// A corrected plan after `error` at step `failed_step`, seen as `aria`.
pub async fn repair(
    d: &Dispatcher,
    ctx: &Context<'_>,
    failed: &Plan,
    error: &str,
    aria: Option<&str>,
) -> Result<Plan, ManualError> {
    let prompt = format!(
        "{}\n\n## 실패한 계획 ({}: {})\n{}\n\n## 오류\n{error}\n\n## 실패 시점의 접근성 트리\n{}\n\n\
         같은 todo로 고친 계획 하나를 보내세요. 성공한 앞 단계는 유지하고, 실패 시점 트리에 있는 이름을 쓰세요.",
        context_text(ctx),
        failed.todo,
        failed.what,
        serde_json::to_string_pretty(failed).unwrap_or_default(),
        aria.unwrap_or("(없음)"),
    );
    let out = d
        .call(Role::Writer, SYSTEM, &prompt, &[], Some(&plan_schema()))
        .await?;
    let raw: RawPlan = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("촬영 계획 수정: {e}")))?;
    let mut p = to_plan(raw, &failed.what);
    p.todo = failed.todo.clone();
    if p.region.is_empty() {
        p.region.clone_from(&failed.region);
    }
    p.ratio = failed.ratio;
    Ok(p)
}

/// A scene from the old manual to reproduce: its picture is shown to the planner.
pub struct SceneTodo {
    pub id: String,
    pub what: String,
    pub how: String,
    /// The old manual's picture of the scene; none for a picture slot of a new manual.
    pub image: Option<std::path::PathBuf>,
}

/// Why a scene cannot be reproduced here.
#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
pub struct Infeasible {
    /// `needs_write`, `realtime`, `account`, `not_found`, `no_data` or `other`.
    pub kind: String,
    pub reason: String,
}

const SCENE_RULES: &str = " You are also given, for each todo, a picture of that screen from an older \
manual. Reproduce the same screen and state in the current product. If the state cannot be reached with \
this tool, set feasible false instead of guessing: needs_write (it needs a change the shared server must \
really perform; requests listed as answered by demo data are fine to send), realtime (live streams or \
progress), account (another user, role or permission), not_found (the screen or control no longer exists \
in the snapshot or the code's labels), no_data (the demo data cannot express it), other. Give the reason \
in Korean. Data values, names and counts may differ from the old picture; that never makes a scene \
infeasible. Use no_data only when the state itself (an error from the server, a record in a status the \
data lacks) cannot appear. Names of records in the old picture need not exist: use any record of \
the same kind and state that the data shows. A disabled control is usually caused by the record's \
state (e.g. only inactive items can be deleted): say which state is needed and use no_data; use \
account only when the screen shows a permission message. The old picture is usually cut from part of \
the screen (a card row, a table, a drawer, one dialog). Set region to 2-5 short texts that appear inside \
the old picture and will be on the current screen (a title, card or column labels, button names), \
spread across the area, so the new picture is cut to the same area; avoid data values. A picture without the \
product's top bar is a cut too. Use an empty region only when the old picture shows the whole \
screen including the top bar.";

const SLOT_RULES: &str = " Each todo is a picture a new manual needs, written as '화면 — 상태 — 자를 \
부분'. Reach that state. If it cannot be reached with this tool, set feasible false with the same \
reason kinds: needs_write (it needs a change the shared server must really perform; requests \
listed as answered by demo data are fine to send), realtime, account, not_found, no_data, other; \
the reason in Korean. Set region to 2-5 short texts that will appear inside the part to cut (a \
dialog's title and buttons, a card row's labels, a table's column labels), spread across it; \
avoid data values. Use an empty region when the part is the whole screen.";

/// Plans for scenes of the old manual; infeasible ones come back with a reason instead.
pub async fn plan_scenes(
    d: &Dispatcher,
    ctx: &Context<'_>,
    scenes: &[SceneTodo],
) -> Result<Vec<(Plan, Option<Infeasible>)>, ManualError> {
    let list: Vec<String> = scenes
        .iter()
        .enumerate()
        .map(|(i, s)| {
            format!(
                "- 그림 {} [todo id: {}] 장면: {} (가는 방법 참고: {})",
                i + 1,
                s.id,
                s.what,
                s.how
            )
        })
        .collect();
    let prompt = format!(
        "{}\n\n## 다시 찍을 장면 (그림 순서 = 아래 순서)\n{}\n\n각 장면마다 계획 하나씩, 위 순서대로 만드세요. todo에는 대괄호 안의 id(예: p2-1)만 그대로 적으세요.",
        context_text(ctx),
        list.join("\n")
    );
    let mut item = plan_schema();
    item["properties"]["feasible"] = json!({"type": "boolean"});
    item["properties"]["reason_kind"] = json!({"type": "string", "enum": ["", "needs_write", "realtime", "account", "not_found", "no_data", "other"]});
    item["properties"]["reason"] = json!({"type": "string"});
    item["properties"]["region"] = json!({"type": "array", "items": {"type": "string"}});
    item["required"] = json!([
        "todo",
        "path",
        "capture",
        "steps",
        "region",
        "feasible",
        "reason_kind",
        "reason"
    ]);
    let schema = json!({
        "type": "object",
        "properties": {"plans": {"type": "array", "items": item}},
        "required": ["plans"]
    });
    let images: Vec<std::path::PathBuf> = scenes.iter().filter_map(|s| s.image.clone()).collect();
    let rules = if images.len() == scenes.len() {
        SCENE_RULES
    } else {
        SLOT_RULES
    };
    let system = format!("{SYSTEM}{rules}");
    let out = d
        .call(Role::Writer, &system, &prompt, &images, Some(&schema))
        .await?;
    #[derive(Deserialize)]
    struct RawScenePlan {
        #[serde(flatten)]
        plan: RawPlan,
        feasible: bool,
        reason_kind: String,
        reason: String,
    }
    #[derive(Deserialize)]
    struct Raw {
        plans: Vec<RawScenePlan>,
    }
    let raw: Raw = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("장면 촬영 계획: {e}")))?;
    // Match by id; if the model wrote something else in `todo`, fall back to the order asked.
    let by_id = raw
        .plans
        .iter()
        .all(|p| scenes.iter().any(|s| s.id == p.plan.todo));
    Ok(raw
        .plans
        .into_iter()
        .enumerate()
        .filter_map(|(i, mut p)| {
            let scene = if by_id {
                scenes.iter().find(|s| s.id == p.plan.todo)?
            } else {
                scenes.get(i)?
            };
            p.plan.todo = scene.id.clone();
            let infeasible = (!p.feasible).then(|| Infeasible {
                kind: if p.reason_kind.is_empty() {
                    "other".into()
                } else {
                    p.reason_kind
                },
                reason: p.reason,
            });
            Some((to_plan(p.plan, &scene.what), infeasible))
        })
        .collect())
}

/// Old-manual picture vs new screenshot.
#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
pub struct Verdict {
    /// `same` (same screen and state), `different` (same screen, other state or changed UI)
    /// or `wrong` (another screen).
    pub verdict: String,
    /// Differences in Korean, most important first.
    pub differences: Vec<String>,
    /// What changed in the product's screen itself (not data, not the cut).
    #[serde(default)]
    pub changes: Vec<String>,
}

const JUDGE_SYSTEM: &str = "You compare two screenshots of a web product for a Korean user manual. Picture 1 \
is from an older manual (the target); picture 2 was just taken from the current product with demo data. \
Data values (names, numbers, dates) will differ and do not matter. Decide: same (same screen and the same \
state: same dialog or panel open, same inputs filled, same messages or selections visible), different (same \
screen but the state differs or the UI has changed: missing message, other tab, added or removed columns or \
buttons), wrong (a different screen or nothing useful). List the differences that matter in Korean, most \
important first; leave the list empty for same. Separately, list in changes only what changed in the \
product's screen itself, which the manual's text must follow: a column, card, button, field, option, tab, \
message or label added, removed or renamed, a control that is now disabled or read-only, a different \
layout of the same parts. Not data values, not how the picture is cut, not a state that was not reached. \
One short Korean sentence each, naming the part (e.g. '목록에 \'전송 방식\' 컬럼이 추가됨'); empty when \
nothing changed.";

pub async fn judge(
    d: &Dispatcher,
    reference: &std::path::Path,
    shot: &std::path::Path,
    what: &str,
) -> Result<Verdict, ManualError> {
    let schema = json!({
        "type": "object",
        "properties": {
            "verdict": {"type": "string", "enum": ["same", "different", "wrong"]},
            "differences": {"type": "array", "items": {"type": "string"}},
            "changes": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["verdict", "differences", "changes"]
    });
    let out = d
        .call(
            Role::Writer,
            JUDGE_SYSTEM,
            &format!("목표 장면: {what}"),
            &[reference.to_path_buf(), shot.to_path_buf()],
            Some(&schema),
        )
        .await?;
    serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("장면 대조: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Block;

    #[test]
    fn apply_shots_turns_todos_into_images() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("manual")).unwrap();
        let run = serde_json::json!({
            "feature": "f", "title": "기능", "generated_at": 0, "models": [], "cost_usd": 0.0,
            "facts": [], "dropped_facts": 0, "checks": [], "compare": null, "compare_source": null,
            "blocks": [
                {"id": "a", "kind": "todo", "markdown": "📷 캡처 필요 — 삭제 대화상자", "facts": [], "shot": null},
                {"id": "b", "kind": "todo", "markdown": "📷 캡처 필요 — 다른 화면", "facts": [], "shot": null}
            ]
        });
        std::fs::write(dir.path().join("manual/f.json"), run.to_string()).unwrap();
        let n = apply_shots(dir.path(), "f", &[("a".into(), "detail/f/a".into())]).unwrap();
        assert_eq!(n, 1);
        let run = crate::pipeline::load_run(dir.path(), "f").unwrap();
        assert_eq!(run.blocks[0].kind, BlockKind::Image);
        assert_eq!(run.blocks[0].markdown, "삭제 대화상자");
        assert_eq!(run.blocks[1].kind, BlockKind::Todo);
        let md = std::fs::read_to_string(dir.path().join("manual/f.md")).unwrap();
        assert!(md.contains("![사진 1](../shots/detail/f/a.png)"), "{md}");
    }

    #[test]
    fn todos_only_capture_blocks() {
        let block = |id: &str, kind: BlockKind, md: &str| Block {
            id: id.into(),
            kind,
            markdown: md.into(),
            facts: vec![],
            shot: None,
        };
        let run = ManualRun {
            blocks: vec![
                block("a", BlockKind::Todo, "📷 캡처 필요 — 삭제 대화상자"),
                block("b", BlockKind::Todo, "확인 필요: 권한"),
                block("c", BlockKind::Paragraph, "📷 캡처 필요 — x"),
            ],
            feature: "f".into(),
            title: "F".into(),
            generated_at: 0,
            models: vec![],
            cost_usd: 0.0,
            facts: vec![],
            dropped_facts: 0,
            checks: vec![],
            compare: None,
            compare_source: None,
        };
        assert_eq!(
            todos(&run),
            vec![("a".to_string(), "삭제 대화상자".to_string())]
        );
    }
}
