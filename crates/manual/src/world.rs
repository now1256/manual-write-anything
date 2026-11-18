//! Demo data for detail screenshots (D-028): API responses the capture sidecar serves in
//! place of the shared server. The writer model invents the values; shapes come from the
//! frontend types and every response is checked against them before it is saved.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use mwa_core::Dispatcher;
use mwa_productmap::shapes::{ApiShape, Shape, ShapeSource};
use mwa_protocol::Role;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ManualError;

pub const WORLD_FILE: &str = "world/world.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct World {
    /// Feature key → shapes fingerprint its responses were made from.
    pub features: BTreeMap<String, String>,
    /// One paragraph describing the demo organisation, shared by all features.
    pub story: String,
    pub responses: Vec<MockResponse>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MockResponse {
    pub feature: String,
    pub method: String,
    /// `/api/...` template with `{}` placeholders, as in the product map.
    pub path: String,
    /// Values for the `{}` placeholders, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<String>,
    /// Query parameters that must match (others are ignored). Empty matches any query.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query: BTreeMap<String, String>,
    pub body: Value,
}

impl MockResponse {
    fn key(&self) -> (String, String, Vec<String>, BTreeMap<String, String>) {
        (
            self.method.clone(),
            self.path.clone(),
            self.params.clone(),
            self.query.clone(),
        )
    }

    fn label(&self) -> String {
        let mut s = format!("{} {}", self.method, self.path);
        if !self.params.is_empty() {
            s.push_str(&format!(" params={:?}", self.params));
        }
        if !self.query.is_empty() {
            s.push_str(&format!(" query={:?}", self.query));
        }
        s
    }
}

pub fn load(ws: &Path) -> Result<World, ManualError> {
    let path = ws.join(WORLD_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| ManualError::Invalid(format!("{} 형식 오류: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(World::default()),
        Err(e) => Err(ManualError::Io(path, e)),
    }
}

pub fn save(ws: &Path, w: &World) -> Result<(), ManualError> {
    let path = ws.join(WORLD_FILE);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| ManualError::Io(dir.to_path_buf(), e))?;
    }
    let text = serde_json::to_string_pretty(w).expect("world serializes");
    std::fs::write(&path, text).map_err(|e| ManualError::Io(path, e))
}

/// Everything the writer needs to invent one feature's data.
pub struct FeatureInput {
    pub key: String,
    pub label: String,
    pub shapes: Vec<ApiShape>,
    /// Frontend code that interprets the data (mappers, status tables, types), with headers.
    pub code: String,
    /// Property → string values the code compares against.
    pub hints: BTreeMap<String, BTreeSet<String>>,
}

impl FeatureInput {
    /// APIs whose responses the screen reads (and so need demo data).
    fn needed(&self) -> impl Iterator<Item = &ApiShape> {
        self.shapes.iter().filter(|s| needs_body(s))
    }
}

/// GETs always (the screen renders them); writes only when the screen reads their result.
fn needs_body(s: &ApiShape) -> bool {
    s.source != ShapeSource::UrlOnly
        && (s.method == "GET" || !matches!(s.shape, Shape::Any | Shape::Void))
}

#[derive(Debug, Default)]
pub struct Report {
    pub responses: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub repaired: bool,
}

const SYSTEM: &str = "You create demo data for screenshots in a Korean user manual of a B2B data/AI platform. \
You write HTTP JSON responses that the product's web screens will render. The data must look like a real, tidy \
customer deployment: Korean business names (no 'test', 'asdf', 'sample1', lorem ipsum), fictional Korean person \
names, plausible numbers that add up (totals equal item counts unless paged, progress matches counts), ISO 8601 \
timestamps between 2026-10-01 and 2026-11-17 (+09:00). Follow the given TypeScript response types exactly: every \
required field present, literal unions respected, no extra wrapping. Use the code excerpts to learn which status \
values and config structures the screens understand, and cover the visible states (e.g. one item in each \
status shown by badges or filters). IDs must be consistent: a detail response's id must appear in the matching \
list response, and references to other features must use ids that exist in the existing world.";

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "story": {"type": "string"},
            "responses": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "method": {"type": "string"},
                        "path": {"type": "string"},
                        "params": {"type": "array", "items": {"type": "string"}},
                        "query": {"type": "string", "description": "k=v&k2=v2 that must match, or empty"},
                        "body_json": {"type": "string", "description": "the response body as JSON text"}
                    },
                    "required": ["method", "path", "params", "query", "body_json"]
                }
            }
        },
        "required": ["story", "responses"]
    })
}

#[derive(Deserialize)]
struct Raw {
    story: String,
    responses: Vec<RawResponse>,
}

#[derive(Deserialize)]
struct RawResponse {
    method: String,
    path: String,
    params: Vec<String>,
    query: String,
    body_json: String,
}

fn parse_query(q: &str) -> BTreeMap<String, String> {
    q.trim_start_matches('?')
        .split('&')
        .filter(|kv| !kv.is_empty())
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

/// Converts model output; bodies that are not JSON become errors.
fn convert(feature: &str, raw: Vec<RawResponse>, errors: &mut Vec<String>) -> Vec<MockResponse> {
    raw.into_iter()
        .filter_map(|r| {
            let method = r.method.to_uppercase();
            match serde_json::from_str::<Value>(&r.body_json) {
                Ok(body) => Some(MockResponse {
                    feature: feature.to_string(),
                    method,
                    path: r.path,
                    params: r.params,
                    query: parse_query(&r.query),
                    body,
                }),
                Err(e) => {
                    errors.push(format!(
                        "{method} {}: body_json이 JSON이 아님 ({e})",
                        r.path
                    ));
                    None
                }
            }
        })
        .collect()
}

fn world_summary(w: &World, max_chars: usize) -> String {
    if w.responses.is_empty() {
        return "(없음)".into();
    }
    let mut out = format!("이야기: {}\n", w.story);
    for r in &w.responses {
        let body = serde_json::to_string(&r.body).unwrap_or_default();
        let body: String = body.chars().take(max_chars).collect();
        out.push_str(&format!("- [{}] {}: {body}\n", r.feature, r.label()));
    }
    out
}

fn prompt(input: &FeatureInput, world: &World) -> String {
    let mut types = String::new();
    for s in input.needed() {
        types.push_str(&format!("{} {}\n  → {}\n", s.method, s.path, s.shape.ts()));
    }
    let hints: Vec<String> = input
        .hints
        .iter()
        .map(|(k, v)| format!("{k}: {}", v.iter().cloned().collect::<Vec<_>>().join(", ")))
        .collect();
    format!(
        "기능: {label} ({key})\n\n\
         ## 이미 있는 가상 세계 (다른 기능, ID를 이어서 쓰세요)\n{existing}\n\
         ## 만들 응답 (메서드 경로 → 응답 타입). `{{}}`는 경로 인자입니다\n{types}\n\
         ## 코드에서 비교하는 문자열 값 (속성: 값들)\n{hints}\n\n\
         ## 데이터 해석 코드\n{code}\n\n\
         요구사항:\n\
         - 위 GET은 모두 응답을 1개 이상 만드세요. 쓰기(POST/PUT/PATCH/DELETE)는 성공 응답 1개씩.\n\
         - 목록은 3~5건, 화면의 배지·필터·상태가 모두 보이도록 상태를 섞으세요.\n\
         - `{{}}`가 있는 상세 경로는 목록에 나온 ID마다(최소 2개) params로 응답을 만드세요.\n\
         - 같은 경로라도 쿼리 값(예: project_id)에 따라 응답이 달라야 하면 query에 그 조건만 적으세요.\n\
         - story는 이 가상 조직과 데이터를 2~3문장으로 설명하세요(기존 이야기가 있으면 이어서).\n\
         - body_json은 응답 본문 JSON 텍스트입니다.",
        label = input.label,
        key = input.key,
        existing = world_summary(world, 600),
        hints = if hints.is_empty() {
            "(없음)".into()
        } else {
            hints.join("\n")
        },
        code = input.code,
    )
}

/// Deterministic checks of one feature's responses against its shapes and the world.
pub fn validate(input: &FeatureInput, world: &World) -> Report {
    let mut rep = Report::default();
    let mine: Vec<&MockResponse> = world
        .responses
        .iter()
        .filter(|r| r.feature == input.key)
        .collect();
    rep.responses = mine.len();
    for r in &mine {
        let Some(api) = input
            .shapes
            .iter()
            .find(|s| s.method == r.method && s.path == r.path)
        else {
            rep.errors
                .push(format!("{}: 이 기능이 부르지 않는 API", r.label()));
            continue;
        };
        let holes = r.path.matches("{}").count();
        if r.params.len() != holes {
            rep.errors.push(format!(
                "{}: 경로 인자 {holes}개인데 params {}개",
                r.label(),
                r.params.len()
            ));
        }
        for e in api.shape.check(&r.body) {
            rep.errors.push(format!("{}: {e}", r.label()));
        }
        // A detail id must appear in another response (the screen navigates from it).
        for p in &r.params {
            let elsewhere = world
                .responses
                .iter()
                .filter(|o| o.key() != r.key())
                .any(|o| contains_scalar(&o.body, p));
            if !elsewhere {
                rep.errors
                    .push(format!("{}: ID {p} 가 어느 목록 응답에도 없음", r.label()));
            }
        }
        hint_warnings(&r.body, &input.hints, &r.label(), &mut rep.warnings);
    }
    for s in input.needed().filter(|s| s.method == "GET") {
        if !mine
            .iter()
            .any(|r| r.method == s.method && r.path == s.path)
        {
            rep.errors
                .push(format!("{} {}: 응답 없음", s.method, s.path));
        }
    }
    rep
}

fn contains_scalar(v: &Value, needle: &str) -> bool {
    match v {
        Value::String(s) => s == needle,
        Value::Number(n) => n.to_string() == needle,
        Value::Array(a) => a.iter().any(|x| contains_scalar(x, needle)),
        Value::Object(m) => m.values().any(|x| contains_scalar(x, needle)),
        _ => false,
    }
}

fn hint_warnings(
    v: &Value,
    hints: &BTreeMap<String, BTreeSet<String>>,
    at: &str,
    out: &mut Vec<String>,
) {
    match v {
        Value::Array(a) => a.iter().for_each(|x| hint_warnings(x, hints, at, out)),
        Value::Object(m) => {
            for (k, x) in m {
                // Only code-like values (`completed`, `qa_embedding`); names and prose are free.
                let code_like = |s: &str| {
                    s.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                };
                if let (Value::String(s), Some(seen)) = (x, hints.get(k))
                    && code_like(s)
                    && !seen.contains(s)
                {
                    out.push(format!("{at}: {k}={s} 는 코드 비교문에 없는 값"));
                }
                hint_warnings(x, hints, at, out);
            }
        }
        _ => {}
    }
}

/// Makes (or remakes) one feature's responses, validates them and repairs once.
pub async fn generate(
    d: &Dispatcher,
    world: &mut World,
    input: &FeatureInput,
    fingerprint: &str,
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<Report, ManualError> {
    world.responses.retain(|r| r.feature != input.key);
    world.features.remove(&input.key);
    let schema = schema();
    progress(format!("{}: 데모 데이터 작성 중", input.key));
    let out = d
        .call(
            Role::Writer,
            SYSTEM,
            &prompt(input, world),
            &[],
            Some(&schema),
        )
        .await?;
    let raw: Raw = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("가상 세계: {e}")))?;
    let mut parse_errors = Vec::new();
    world.story = raw.story;
    world
        .responses
        .extend(convert(&input.key, raw.responses, &mut parse_errors));

    let mut rep = validate(input, world);
    rep.errors.extend(parse_errors);
    if rep.errors.is_empty() {
        world
            .features
            .insert(input.key.clone(), fingerprint.to_string());
        return Ok(rep);
    }

    progress(format!(
        "{}: 검사 위반 {}건, 1회 수정 요청",
        input.key,
        rep.errors.len()
    ));
    let mine: Vec<String> = world
        .responses
        .iter()
        .filter(|r| r.feature == input.key)
        .map(|r| format!("- {}: {}", r.label(), r.body))
        .collect();
    let fix = format!(
        "{}\n\n## 지금 만든 응답\n{}\n\n## 검사 위반\n{}\n\n\
         위반이 있는 응답과 빠진 응답만 고쳐서 다시 보내세요. 같은 method·path·params·query로 보내면 \
         기존 것을 대체합니다. 위반 없는 응답은 보내지 마세요. story는 그대로 두세요.",
        prompt(input, world),
        mine.join("\n"),
        rep.errors.join("\n")
    );
    let out = d
        .call(Role::Writer, SYSTEM, &fix, &[], Some(&schema))
        .await?;
    let raw: Raw = serde_json::from_value(out.response.json.unwrap_or_default())
        .map_err(|e| ManualError::Shape(format!("가상 세계 수정: {e}")))?;
    let mut parse_errors = Vec::new();
    for r in convert(&input.key, raw.responses, &mut parse_errors) {
        world.responses.retain(|o| o.key() != r.key());
        world.responses.push(r);
    }
    let mut rep = validate(input, world);
    rep.errors.extend(parse_errors);
    rep.repaired = true;
    if rep.errors.is_empty() {
        world
            .features
            .insert(input.key.clone(), fingerprint.to_string());
    }
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mwa_productmap::shapes::Field;

    fn shapes() -> Vec<ApiShape> {
        let item = Shape::Object {
            fields: [
                (
                    "id".to_string(),
                    Field {
                        shape: Shape::Number,
                        optional: false,
                    },
                ),
                (
                    "status".to_string(),
                    Field {
                        shape: Shape::String,
                        optional: false,
                    },
                ),
            ]
            .into(),
        };
        vec![
            ApiShape {
                method: "GET".into(),
                path: "/api/p".into(),
                client: "a.ts:1".into(),
                source: ShapeSource::TypeArgument,
                shape: Shape::Array {
                    of: Box::new(item.clone()),
                },
            },
            ApiShape {
                method: "GET".into(),
                path: "/api/p/{}".into(),
                client: "a.ts:2".into(),
                source: ShapeSource::TypeArgument,
                shape: item,
            },
            ApiShape {
                method: "DELETE".into(),
                path: "/api/p".into(),
                client: "a.ts:3".into(),
                source: ShapeSource::None,
                shape: Shape::Any,
            },
        ]
    }

    fn resp(path: &str, params: &[&str], body: Value) -> MockResponse {
        MockResponse {
            feature: "f".into(),
            method: "GET".into(),
            path: path.into(),
            params: params.iter().map(|s| s.to_string()).collect(),
            query: BTreeMap::new(),
            body,
        }
    }

    #[test]
    fn validate_checks_shapes_ids_and_coverage() {
        let input = FeatureInput {
            key: "f".into(),
            label: "F".into(),
            shapes: shapes(),
            code: String::new(),
            hints: [(
                "status".to_string(),
                ["done".to_string(), "draft".to_string()].into(),
            )]
            .into(),
        };
        let mut w = World::default();
        w.responses.push(resp(
            "/api/p",
            &[],
            json!([{"id": 1, "status": "done"}, {"id": 2, "status": "running"}]),
        ));
        w.responses.push(resp(
            "/api/p/{}",
            &["1"],
            json!({"id": 1, "status": "done"}),
        ));
        w.responses.push(resp(
            "/api/p/{}",
            &["9"],
            json!({"id": "9", "status": "done"}),
        ));
        let rep = validate(&input, &w);
        assert_eq!(
            rep.errors,
            vec![
                "GET /api/p/{} params=[\"9\"]: $.id: number 이어야 하는데 string",
                "GET /api/p/{} params=[\"9\"]: ID 9 가 어느 목록 응답에도 없음",
            ]
        );
        assert_eq!(
            rep.warnings,
            vec!["GET /api/p: status=running 는 코드 비교문에 없는 값"]
        );

        w.responses.retain(|r| r.path != "/api/p");
        let rep = validate(&input, &w);
        assert!(rep.errors.iter().any(|e| e == "GET /api/p: 응답 없음"));
    }

    #[test]
    fn parses_query_and_bad_json() {
        let mut errors = Vec::new();
        let out = convert(
            "f",
            vec![
                RawResponse {
                    method: "get".into(),
                    path: "/api/e".into(),
                    params: vec![],
                    query: "?project_id=3&x=1".into(),
                    body_json: "[]".into(),
                },
                RawResponse {
                    method: "GET".into(),
                    path: "/api/bad".into(),
                    params: vec![],
                    query: String::new(),
                    body_json: "{oops".into(),
                },
            ],
            &mut errors,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].method, "GET");
        assert_eq!(out[0].query["project_id"], "3");
        assert_eq!(errors.len(), 1);
    }
}
