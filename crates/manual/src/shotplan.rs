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
}

fn context_text(c: &Context) -> String {
    let snaps: Vec<String> = c
        .snapshots
        .iter()
        .map(|(p, a)| format!("### {p}\n{a}"))
        .collect();
    format!(
        "기능: {}\n화면 경로: {}\n\n## 시작 화면의 접근성 트리\n{}\n\n## 데모 데이터 (경로의 id로 쓸 수 있음)\n{}\n\n## 코드의 화면 문구\n{}",
        c.feature_label,
        c.routes.join(", "),
        snaps.join("\n\n"),
        c.data,
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
    Ok(p)
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
