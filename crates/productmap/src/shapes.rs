//! Response shapes of client API calls (D-028): the `T` of `apiFetch<T>(...)`, or the
//! enclosing function's `Promise<T>` return type, resolved through aliases, interfaces,
//! generics and imports into a JSON-like shape. Anything we cannot follow becomes
//! `Unresolved` so callers can report coverage instead of guessing.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tree_sitter::Node;

use crate::MapError;
use crate::graph::Project;
use crate::ts::Src;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Shape {
    String,
    Number,
    Boolean,
    Null,
    /// `void`: the call expects no body.
    Void,
    /// `any`, `unknown`, `object`: any JSON value is accepted.
    Any,
    Lit {
        v: serde_json::Value,
    },
    Array {
        of: Box<Shape>,
    },
    Object {
        fields: BTreeMap<String, Field>,
    },
    /// `Record<string, T>` or an index signature.
    Record {
        of: Box<Shape>,
    },
    Union {
        of: Vec<Shape>,
    },
    Unresolved {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub shape: Shape,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

impl Shape {
    /// Texts of unresolved parts, for coverage reports.
    pub fn unresolved(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.walk(&mut |s| {
            if let Shape::Unresolved { text } = s {
                out.push(text.clone());
            }
        });
        out
    }

    /// Names of all object fields, at any depth.
    pub fn field_names(&self) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        self.walk(&mut |s| {
            if let Shape::Object { fields } = s {
                out.extend(fields.keys().cloned());
            }
        });
        out
    }

    /// Field paths whose value is unconstrained (`any`/`unknown`), e.g. `items[].config`.
    pub fn free_paths(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.free_at("", &mut out);
        out
    }

    fn free_at(&self, at: &str, out: &mut Vec<String>) {
        match self {
            Shape::Any => out.push(if at.is_empty() {
                "(전체)".into()
            } else {
                at.into()
            }),
            Shape::Array { of } => of.free_at(&format!("{at}[]"), out),
            Shape::Record { of } => of.free_at(&format!("{at}{{}}"), out),
            Shape::Object { fields } => {
                for (k, x) in fields {
                    let p = if at.is_empty() {
                        k.clone()
                    } else {
                        format!("{at}.{k}")
                    };
                    x.shape.free_at(&p, out);
                }
            }
            Shape::Union { of } => of.iter().for_each(|x| x.free_at(at, out)),
            _ => {}
        }
    }

    /// Mismatches between `v` and this shape, as `path: problem` lines. Extra object fields
    /// are allowed (the screen ignores them); missing required fields are not.
    pub fn check(&self, v: &serde_json::Value) -> Vec<String> {
        let mut out = Vec::new();
        self.check_at("$", v, &mut out);
        out
    }

    fn check_at(&self, at: &str, v: &serde_json::Value, out: &mut Vec<String>) {
        use serde_json::Value as V;
        let ok = match (self, v) {
            (Shape::Any | Shape::Unresolved { .. }, _) => true,
            (Shape::Void, _) => true,
            (Shape::String, V::String(_))
            | (Shape::Number, V::Number(_))
            | (Shape::Boolean, V::Bool(_))
            | (Shape::Null, V::Null) => true,
            (Shape::Lit { v: want }, got) => lit_eq(want, got),
            (Shape::Array { of }, V::Array(items)) => {
                for (i, x) in items.iter().enumerate() {
                    of.check_at(&format!("{at}[{i}]"), x, out);
                }
                true
            }
            (Shape::Record { of }, V::Object(m)) => {
                for (k, x) in m {
                    of.check_at(&format!("{at}.{k}"), x, out);
                }
                true
            }
            (Shape::Object { fields }, V::Object(m)) => {
                for (k, f) in fields {
                    match m.get(k) {
                        Some(x) => f.shape.check_at(&format!("{at}.{k}"), x, out),
                        None if f.optional => {}
                        None => out.push(format!("{at}.{k}: 필수 필드 없음")),
                    }
                }
                true
            }
            (Shape::Union { of }, _) => {
                // Accept if any member fits; report against the closest (fewest problems).
                let mut best: Option<Vec<String>> = None;
                for m in of {
                    let mut e = Vec::new();
                    m.check_at(at, v, &mut e);
                    if e.is_empty() {
                        return;
                    }
                    if best.as_ref().is_none_or(|b| e.len() < b.len()) {
                        best = Some(e);
                    }
                }
                let lits: Vec<String> = of
                    .iter()
                    .filter_map(|m| match m {
                        Shape::Lit { v } => Some(v.to_string()),
                        _ => None,
                    })
                    .collect();
                if lits.len() == of.len() {
                    out.push(format!("{at}: {v} 는 허용값({})이 아님", lits.join(" | ")));
                } else {
                    out.extend(best.unwrap_or_default());
                }
                true
            }
            _ => false,
        };
        if !ok {
            out.push(format!("{at}: {} 이어야 하는데 {}", self.ts(), kind_of(v)));
        }
    }

    /// Compact TypeScript-like rendering for prompts and reports.
    pub fn ts(&self) -> String {
        match self {
            Shape::String => "string".into(),
            Shape::Number => "number".into(),
            Shape::Boolean => "boolean".into(),
            Shape::Null => "null".into(),
            Shape::Void => "void".into(),
            Shape::Any => "unknown".into(),
            Shape::Lit { v } => v.to_string(),
            Shape::Array { of } => match **of {
                Shape::Union { .. } => format!("({})[]", of.ts()),
                _ => format!("{}[]", of.ts()),
            },
            Shape::Record { of } => format!("{{ [k: string]: {} }}", of.ts()),
            Shape::Union { of } => of.iter().map(|x| x.ts()).collect::<Vec<_>>().join(" | "),
            Shape::Object { fields } => {
                let parts: Vec<String> = fields
                    .iter()
                    .map(|(k, f)| {
                        format!("{k}{}: {}", if f.optional { "?" } else { "" }, f.shape.ts())
                    })
                    .collect();
                format!("{{ {} }}", parts.join("; "))
            }
            Shape::Unresolved { .. } => "unknown".into(),
        }
    }

    fn walk(&self, f: &mut impl FnMut(&Shape)) {
        f(self);
        match self {
            Shape::Array { of } | Shape::Record { of } => of.walk(f),
            Shape::Object { fields } => fields.values().for_each(|x| x.shape.walk(f)),
            Shape::Union { of } => of.iter().for_each(|x| x.walk(f)),
            _ => {}
        }
    }
}

fn lit_eq(want: &serde_json::Value, got: &serde_json::Value) -> bool {
    match (want.as_f64(), got.as_f64()) {
        (Some(a), Some(b)) => a == b,
        _ => want == got,
    }
}

fn kind_of(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "배열",
        serde_json::Value::Object(_) => "객체",
    }
}

/// Fingerprint of a feature's shapes: changes when any response type changes.
pub fn fingerprint(shapes: &[ApiShape]) -> String {
    use sha2::{Digest, Sha256};
    let text = serde_json::to_string(shapes).expect("shapes serialize");
    let hash = Sha256::digest(text.as_bytes());
    hash.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// String values the code compares a property against (`x.status === "done"`,
/// `switch (x.kind) { case "a": }`, `["a", "b"].includes(x.status)`), keyed by property name.
/// Only a hint: values reached through lookup tables are not seen.
pub fn value_hints(
    project: &mut Project,
    files: &[PathBuf],
) -> Result<BTreeMap<String, std::collections::BTreeSet<String>>, MapError> {
    let mut out: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for file in files {
        let src = project.src(file)?;
        let prop = |n: Node| -> Option<String> {
            let n = crate::expand::unwrap_parens(n);
            match n.kind() {
                "member_expression" => n
                    .child_by_field_name("property")
                    .map(|p| src.text_of(p).to_string()),
                "identifier" => Some(src.text_of(n).to_string()),
                _ => None,
            }
        };
        let string = |n: Node| {
            (n.kind() == "string")
                .then(|| src.string_value(n))
                .flatten()
        };
        for n in src.nodes() {
            match n.kind() {
                "binary_expression" => {
                    let op = n
                        .child_by_field_name("operator")
                        .map(|o| src.text_of(o))
                        .unwrap_or_default();
                    if !matches!(op, "===" | "!==" | "==" | "!=") {
                        continue;
                    }
                    let (Some(l), Some(r)) = (
                        n.child_by_field_name("left"),
                        n.child_by_field_name("right"),
                    ) else {
                        continue;
                    };
                    let pair = match (prop(l), string(r)) {
                        (Some(p), Some(v)) => Some((p, v)),
                        _ => prop(r).zip(string(l)),
                    };
                    if let Some((p, v)) = pair {
                        out.entry(p).or_default().insert(v);
                    }
                }
                "switch_statement" => {
                    let Some(p) = n.child_by_field_name("value").and_then(prop) else {
                        continue;
                    };
                    let Some(body) = n.child_by_field_name("body") else {
                        continue;
                    };
                    let mut c = body.walk();
                    for case in body.named_children(&mut c) {
                        if let Some(v) = case.child_by_field_name("value").and_then(string) {
                            out.entry(p.clone()).or_default().insert(v);
                        }
                    }
                }
                // `statusMap[p.status]` where `statusMap` is a top-level object literal.
                "subscript_expression" => {
                    let (Some(obj), Some(p)) = (
                        n.child_by_field_name("object")
                            .filter(|o| o.kind() == "identifier"),
                        n.child_by_field_name("index").and_then(prop),
                    ) else {
                        continue;
                    };
                    let Some(table) = src
                        .top_level_value(src.text_of(obj))
                        .map(crate::expand::unwrap_parens)
                        .filter(|v| v.kind() == "object")
                    else {
                        continue;
                    };
                    let mut c = table.walk();
                    for pair in table.named_children(&mut c).filter(|x| x.kind() == "pair") {
                        let Some(k) = pair.child_by_field_name("key") else {
                            continue;
                        };
                        let key = match k.kind() {
                            "string" => src.string_value(k),
                            "property_identifier" => Some(src.text_of(k).to_string()),
                            _ => None,
                        };
                        out.entry(p.clone()).or_default().extend(key);
                    }
                }
                "call_expression" if src.callee_name(n).as_deref() == Some("includes") => {
                    let Some(arr) = n
                        .child_by_field_name("function")
                        .and_then(|f| f.child_by_field_name("object"))
                        .filter(|o| o.kind() == "array")
                    else {
                        continue;
                    };
                    let Some(p) = src.args(n).first().copied().and_then(prop) else {
                        continue;
                    };
                    let values = src.string_array(arr);
                    if !values.is_empty() {
                        out.entry(p).or_default().extend(values);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// Where the shape of one call came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeSource {
    /// `apiFetch<T>(...)`
    TypeArgument,
    /// `function f(): Promise<T>` around the call
    ReturnType,
    /// No type at the call site.
    None,
    /// The URL is only built (download link, `EventSource` URL helper); nothing is fetched here.
    UrlOnly,
}

/// Response shape of one API a feature calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiShape {
    pub method: String,
    pub path: String,
    /// `file:line` of the client call, relative to the frontend root.
    pub client: String,
    pub source: ShapeSource,
    pub shape: Shape,
}

/// Shapes of every distinct (method, path) the feature's screens call.
pub fn feature_shapes(
    project: &mut Project,
    feature: &crate::map::Feature,
    call_fns: &[&str],
) -> Result<Vec<ApiShape>, MapError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for api in feature.screens.iter().flat_map(|s| &s.apis) {
        if !seen.insert((api.method.clone(), api.path.clone())) {
            continue;
        }
        let (file, line) = api
            .client
            .rsplit_once(':')
            .and_then(|(f, l)| Some((f, l.parse::<usize>().ok()?)))
            .ok_or_else(|| MapError::Invalid(format!("호출 위치 형식 오류: {}", api.client)))?;
        let path = project.root.join(file);
        let (shape, source) = call_shape(project, &path, line, call_fns)?;
        out.push(ApiShape {
            method: api.method.clone(),
            path: api.path.clone(),
            client: api.client.clone(),
            source,
            shape,
        });
    }
    Ok(out)
}

const MAX_DEPTH: usize = 16;

/// Shape of the response of the client call at `file:line` (`call_fns` as in the adapter).
pub fn call_shape(
    project: &mut Project,
    file: &Path,
    line: usize,
    call_fns: &[&str],
) -> Result<(Shape, ShapeSource), MapError> {
    let (node_range, src_kind) = {
        let src = project.src(file)?;
        // The recorded line may be inside a multi-line call (e.g. the URL argument line);
        // take the innermost matching call that spans it.
        let call = src
            .nodes_of_kind("call_expression")
            .into_iter()
            .filter(|n| {
                n.start_position().row < line && line <= n.end_position().row + 1
                    || src.line(*n) == line
            })
            .filter(|n| callee(src, *n).is_some_and(|c| call_fns.contains(&c)))
            .min_by_key(|n| n.byte_range().len());
        let Some(call) = call else {
            return Ok((Shape::Any, ShapeSource::UrlOnly));
        };
        if let Some(args) = call.child_by_field_name("type_arguments") {
            let first = args.named_child(0).map(|t| t.byte_range());
            (first, ShapeSource::TypeArgument)
        } else {
            (
                enclosing_return_type(call).map(|t| t.byte_range()),
                ShapeSource::ReturnType,
            )
        }
    };
    let Some(range) = node_range else {
        return Ok((Shape::Any, ShapeSource::None));
    };
    let mut r = Resolver {
        project,
        stack: Vec::new(),
    };
    // `Promise<T>` return types unwrap through the `Promise` builtin in `named`.
    Ok((r.at(file, range, &HashMap::new())?, src_kind))
}

/// Callee name; tree-sitter parses `await f<T>(x)` as a call whose function is `await f`.
fn callee<'s>(src: &'s Src, call: Node) -> Option<&'s str> {
    let mut f = call.child_by_field_name("function")?;
    if f.kind() == "await_expression" {
        f = f.named_child(0)?;
    }
    match f.kind() {
        "identifier" => Some(src.text_of(f)),
        "member_expression" => f.child_by_field_name("property").map(|p| src.text_of(p)),
        _ => None,
    }
}

fn enclosing_return_type(n: Node) -> Option<Node> {
    let mut cur = n.parent();
    while let Some(p) = cur {
        if matches!(
            p.kind(),
            "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
                | "generator_function_declaration"
        ) {
            let ann = p.child_by_field_name("return_type")?;
            return ann.named_child(0);
        }
        cur = p.parent();
    }
    None
}

type Env = HashMap<String, Shape>;

struct Resolver<'p> {
    project: &'p mut Project,
    /// (file, name) currently being resolved, to cut recursive types.
    stack: Vec<(PathBuf, String)>,
}

impl Resolver<'_> {
    /// Resolves the type node at `range` in `file`.
    fn at(
        &mut self,
        file: &Path,
        range: std::ops::Range<usize>,
        env: &Env,
    ) -> Result<Shape, MapError> {
        let src = self.project.src(file)?;
        let node = src
            .root()
            .descendant_for_byte_range(range.start, range.end)
            .ok_or_else(|| MapError::Invalid(format!("{} 타입 위치 없음", file.display())))?;
        let plan = plan(src, node);
        self.run(file, plan, env)
    }

    fn run(&mut self, file: &Path, plan: Plan, env: &Env) -> Result<Shape, MapError> {
        if self.stack.len() > MAX_DEPTH {
            return Ok(Shape::Unresolved {
                text: "깊이 초과".into(),
            });
        }
        Ok(match plan {
            Plan::Done(s) => s,
            Plan::Array(inner) => Shape::Array {
                of: Box::new(self.run(file, *inner, env)?),
            },
            Plan::Union(parts) => {
                let mut of = Vec::new();
                for p in parts {
                    match self.run(file, p, env)? {
                        Shape::Union { of: inner } => of.extend(inner),
                        s => of.push(s),
                    }
                }
                of.dedup();
                if of.len() == 1 {
                    of.pop().unwrap()
                } else {
                    Shape::Union { of }
                }
            }
            Plan::Intersection(parts) => {
                let mut fields = BTreeMap::new();
                for p in parts {
                    match self.run(file, p, env)? {
                        Shape::Object { fields: f } => fields.extend(f),
                        Shape::Any => {}
                        other => {
                            return Ok(Shape::Unresolved {
                                text: format!("교차 타입의 비객체 부분: {other:?}"),
                            });
                        }
                    }
                }
                Shape::Object { fields }
            }
            Plan::Object { fields, index } => {
                if fields.is_empty()
                    && let Some(ix) = index
                {
                    return Ok(Shape::Record {
                        of: Box::new(self.run(file, *ix, env)?),
                    });
                }
                let mut out = BTreeMap::new();
                for (name, optional, p) in fields {
                    let shape = self.run(file, p, env)?;
                    out.insert(name, Field { shape, optional });
                }
                Shape::Object { fields: out }
            }
            Plan::Named { name, args } => {
                let mut arg_shapes = Vec::new();
                for a in args {
                    arg_shapes.push(self.run(file, a, env)?);
                }
                self.named(file, &name, arg_shapes, env)?
            }
            Plan::Imported { spec, name } => match self.project.resolve(file, &spec) {
                Some(target) => self.named(&target, &name, Vec::new(), &Env::new())?,
                None => Shape::Unresolved {
                    text: format!("import(\"{spec}\").{name}"),
                },
            },
            Plan::Utility { name, target, keys } => {
                let base = self.run(file, *target, env)?;
                utility(&name, base, &keys)
            }
        })
    }

    /// A type referenced by name: generic parameter, builtin, local or imported declaration.
    fn named(
        &mut self,
        file: &Path,
        name: &str,
        args: Vec<Shape>,
        env: &Env,
    ) -> Result<Shape, MapError> {
        if let Some(s) = env.get(name) {
            return Ok(s.clone());
        }
        let first = || args.first().cloned().unwrap_or(Shape::Any);
        match name {
            "Array" | "ReadonlyArray" => {
                return Ok(Shape::Array {
                    of: Box::new(first()),
                });
            }
            "Record" => {
                return Ok(Shape::Record {
                    of: Box::new(args.get(1).cloned().unwrap_or(Shape::Any)),
                });
            }
            "Promise" | "Readonly" | "NonNullable" | "Required" => return Ok(first()),
            "Partial" => return Ok(utility("Partial", first(), &[])),
            "Date" => return Ok(Shape::String),
            _ => {}
        }
        let Some((decl_file, decl_range)) = self.find_decl(file, name, 0)? else {
            return Ok(Shape::Unresolved {
                text: format!("{name} 정의 없음"),
            });
        };
        let key = (decl_file.clone(), name.to_string());
        if self.stack.contains(&key) {
            return Ok(Shape::Any);
        }
        self.stack.push(key);
        let src = self.project.src(&decl_file)?;
        let decl = src
            .root()
            .descendant_for_byte_range(decl_range.start, decl_range.end)
            .expect("declaration node");
        let (params, body) = decl_plan(src, decl);
        let mut inner_env = Env::new();
        for (i, p) in params.into_iter().enumerate() {
            inner_env.insert(p, args.get(i).cloned().unwrap_or(Shape::Any));
        }
        let out = self.run(&decl_file, body, &inner_env);
        self.stack.pop();
        out
    }

    /// Finds the declaration of `name` visible in `file`, following imports and re-exports.
    fn find_decl(
        &mut self,
        file: &Path,
        name: &str,
        hops: usize,
    ) -> Result<Option<(PathBuf, std::ops::Range<usize>)>, MapError> {
        if hops > 8 {
            return Ok(None);
        }
        let (local, imports) = {
            let src = self.project.src(file)?;
            (local_decl(src, name), src.imports())
        };
        if let Some(r) = local {
            return Ok(Some((file.to_path_buf(), r)));
        }
        // Named imports and `export { a as name } from`.
        for imp in &imports {
            if let Some((imported, _)) = imp.names.iter().find(|(_, l)| l == name)
                && imported != "*"
                && let Some(target) = self.project.resolve(file, &imp.source)
            {
                return self.find_decl(&target, imported, hops + 1);
            }
        }
        // `export * from "./x"`
        for imp in imports.iter().filter(|i| i.names.is_empty()) {
            if let Some(target) = self.project.resolve(file, &imp.source)
                && let Some(found) = self.find_decl(&target, name, hops + 1)?
            {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
}

/// Top-level `type`/`interface`/`enum` named `name` (exported or not).
fn local_decl(src: &Src, name: &str) -> Option<std::ops::Range<usize>> {
    let root = src.root();
    let mut c = root.walk();
    for stmt in root.named_children(&mut c) {
        let decl = if stmt.kind() == "export_statement" {
            match stmt.child_by_field_name("declaration") {
                Some(d) => d,
                None => continue,
            }
        } else {
            stmt
        };
        if matches!(
            decl.kind(),
            "type_alias_declaration" | "interface_declaration" | "enum_declaration"
        ) && decl
            .child_by_field_name("name")
            .is_some_and(|n| src.text_of(n) == name)
        {
            return Some(decl.byte_range());
        }
    }
    None
}

/// A syntax-level description of a type, built while the source is borrowed and resolved
/// afterwards (resolution needs `&mut Project` to open other files).
enum Plan {
    Done(Shape),
    Array(Box<Plan>),
    Union(Vec<Plan>),
    Intersection(Vec<Plan>),
    Object {
        fields: Vec<(String, bool, Plan)>,
        index: Option<Box<Plan>>,
    },
    Named {
        name: String,
        args: Vec<Plan>,
    },
    /// `import("spec").Name`
    Imported {
        spec: String,
        name: String,
    },
    /// `Omit<T, K>` / `Pick<T, K>` with literal keys.
    Utility {
        name: String,
        target: Box<Plan>,
        keys: Vec<String>,
    },
}

fn unresolved(src: &Src, n: Node) -> Plan {
    Plan::Done(Shape::Unresolved {
        text: crate::short(src.text_of(n)),
    })
}

fn plan(src: &Src, n: Node) -> Plan {
    match n.kind() {
        "type_annotation" | "parenthesized_type" | "readonly_type" => {
            match named(n).first().copied() {
                Some(c) => plan(src, c),
                None => unresolved(src, n),
            }
        }
        "predefined_type" => Plan::Done(match src.text_of(n) {
            "string" => Shape::String,
            "number" | "bigint" => Shape::Number,
            "boolean" => Shape::Boolean,
            "null" | "undefined" => Shape::Null,
            "void" | "never" => Shape::Void,
            _ => Shape::Any,
        }),
        "literal_type" => {
            let Some(c) = n.named_child(0) else {
                return unresolved(src, n);
            };
            Plan::Done(match c.kind() {
                "string" => Shape::Lit {
                    v: src.string_value(c).unwrap_or_default().into(),
                },
                "number" => match src.text_of(c).parse::<f64>() {
                    Ok(x) => Shape::Lit { v: x.into() },
                    Err(_) => Shape::Number,
                },
                "true" => Shape::Lit { v: true.into() },
                "false" => Shape::Lit { v: false.into() },
                "null" | "undefined" => Shape::Null,
                "unary_expression" => Shape::Number,
                _ => return unresolved(src, n),
            })
        }
        "template_literal_type" => Plan::Done(Shape::String),
        "array_type" => match n.named_child(0) {
            Some(c) => Plan::Array(Box::new(plan(src, c))),
            None => unresolved(src, n),
        },
        "tuple_type" => {
            let parts: Vec<Plan> = named(n).into_iter().map(|x| plan(src, x)).collect();
            Plan::Array(Box::new(Plan::Union(parts)))
        }
        "union_type" => Plan::Union(named(n).into_iter().map(|x| plan(src, x)).collect()),
        "intersection_type" => {
            Plan::Intersection(named(n).into_iter().map(|x| plan(src, x)).collect())
        }
        "object_type" | "interface_body" => object_plan(src, n),
        "type_identifier" => Plan::Named {
            name: src.text_of(n).to_string(),
            args: Vec::new(),
        },
        "generic_type" => {
            let Some(name) = n.child_by_field_name("name") else {
                return unresolved(src, n);
            };
            if name.kind() != "type_identifier" {
                return unresolved(src, n);
            }
            let name = src.text_of(name).to_string();
            let args: Vec<Node> = n
                .child_by_field_name("type_arguments")
                .map(|a| {
                    let mut c = a.walk();
                    a.named_children(&mut c).collect()
                })
                .unwrap_or_default();
            if matches!(name.as_str(), "Omit" | "Pick") && args.len() == 2 {
                return Plan::Utility {
                    name,
                    target: Box::new(plan(src, args[0])),
                    keys: literal_keys(src, args[1]),
                };
            }
            Plan::Named {
                name,
                args: args.into_iter().map(|a| plan(src, a)).collect(),
            }
        }
        "function_type" | "constructor_type" => Plan::Done(Shape::Any),
        // `void`/`undefined`/`null` in type position are their own node kinds.
        _ => match src.text_of(n) {
            "void" | "never" => Plan::Done(Shape::Void),
            "undefined" | "null" => Plan::Done(Shape::Null),
            text => match import_type(text) {
                Some((spec, name)) => Plan::Imported { spec, name },
                None => unresolved(src, n),
            },
        },
    }
}

/// Named children without comments (comments may sit between union members).
fn named(n: Node) -> Vec<Node> {
    let mut c = n.walk();
    n.named_children(&mut c)
        .filter(|x| x.kind() != "comment")
        .collect()
}

/// `import("spec").Name` → (spec, Name).
fn import_type(text: &str) -> Option<(String, String)> {
    let rest = text.strip_prefix("import(")?;
    let (spec, rest) = rest.split_once(')')?;
    let spec = spec.trim().trim_matches(|c| c == '"' || c == '\'');
    let name = rest.strip_prefix('.')?;
    name.chars()
        .all(|c| c.is_alphanumeric() || c == '_')
        .then(|| (spec.to_string(), name.to_string()))
}

fn object_plan(src: &Src, n: Node) -> Plan {
    let mut fields = Vec::new();
    let mut index = None;
    let mut c = n.walk();
    for m in n.named_children(&mut c) {
        match m.kind() {
            "property_signature" => {
                let Some(name) = m.child_by_field_name("name") else {
                    continue;
                };
                let key = match name.kind() {
                    "string" => src.string_value(name).unwrap_or_default(),
                    _ => src.text_of(name).to_string(),
                };
                let mut cc = m.walk();
                let optional = m.children(&mut cc).any(|x| x.kind() == "?");
                let ty = m
                    .child_by_field_name("type")
                    .map(|t| plan(src, t))
                    .unwrap_or(Plan::Done(Shape::Any));
                fields.push((key, optional, ty));
            }
            "index_signature" => {
                index = m
                    .child_by_field_name("type")
                    .map(|t| Box::new(plan(src, t)));
            }
            _ => {}
        }
    }
    Plan::Object { fields, index }
}

fn literal_keys(src: &Src, n: Node) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![n];
    while let Some(x) = stack.pop() {
        if x.kind() == "string" {
            out.extend(src.string_value(x));
            continue;
        }
        let mut c = x.walk();
        stack.extend(x.named_children(&mut c));
    }
    out
}

/// Generic parameter names and body of a `type`/`interface`/`enum` declaration.
fn decl_plan(src: &Src, decl: Node) -> (Vec<String>, Plan) {
    let params: Vec<String> = decl
        .child_by_field_name("type_parameters")
        .map(|tp| {
            let mut c = tp.walk();
            tp.named_children(&mut c)
                .filter_map(|p| p.child_by_field_name("name"))
                .map(|n| src.text_of(n).to_string())
                .collect()
        })
        .unwrap_or_default();
    let body = match decl.kind() {
        "type_alias_declaration" => match decl.child_by_field_name("value") {
            Some(v) => plan(src, v),
            None => unresolved(src, decl),
        },
        "interface_declaration" => {
            let own = match decl.child_by_field_name("body") {
                Some(b) => object_plan(src, b),
                None => unresolved(src, decl),
            };
            let mut parts = Vec::new();
            let mut c = decl.walk();
            for ch in decl.named_children(&mut c) {
                if ch.kind() == "extends_type_clause" {
                    let mut cc = ch.walk();
                    parts.extend(ch.named_children(&mut cc).map(|t| plan(src, t)));
                }
            }
            if parts.is_empty() {
                own
            } else {
                parts.push(own);
                Plan::Intersection(parts)
            }
        }
        "enum_declaration" => {
            let mut lits = Vec::new();
            let mut next = 0f64;
            if let Some(body) = decl.child_by_field_name("body") {
                let mut c = body.walk();
                for m in body.named_children(&mut c) {
                    let value = m.child_by_field_name("value");
                    let shape = match value {
                        Some(v) if v.kind() == "string" => Shape::Lit {
                            v: src.string_value(v).unwrap_or_default().into(),
                        },
                        Some(v) => match src.text_of(v).parse::<f64>() {
                            Ok(x) => {
                                next = x + 1.0;
                                Shape::Lit { v: x.into() }
                            }
                            Err(_) => Shape::Number,
                        },
                        None => {
                            let s = Shape::Lit { v: next.into() };
                            next += 1.0;
                            s
                        }
                    };
                    lits.push(Plan::Done(shape));
                }
            }
            Plan::Union(lits)
        }
        _ => unresolved(src, decl),
    };
    (params, body)
}

fn utility(name: &str, base: Shape, keys: &[String]) -> Shape {
    let Shape::Object { mut fields } = base else {
        return base;
    };
    match name {
        "Partial" => fields.values_mut().for_each(|f| f.optional = true),
        "Omit" => fields.retain(|k, _| !keys.contains(k)),
        "Pick" => fields.retain(|k, _| keys.contains(k)),
        _ => {}
    }
    Shape::Object { fields }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, text: &str) -> PathBuf {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
        p
    }

    fn obj(fields: &[(&str, Shape, bool)]) -> Shape {
        Shape::Object {
            fields: fields
                .iter()
                .map(|(k, s, o)| {
                    (
                        k.to_string(),
                        Field {
                            shape: s.clone(),
                            optional: *o,
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn resolves_generics_imports_and_barrels() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "src/api/types.ts",
            r#"
export type Status = "draft" | "done";
export type Page<T> = { items: T[]; total: number };
export interface Base { id: number }
export interface Item extends Base { name: string; status: Status; meta?: Record<string, unknown> | null }
"#,
        );
        write(root, "src/api/index.ts", "export * from \"./types\";\n");
        let f = write(
            root,
            "src/api/fetcher.ts",
            r#"import type { Page, Item } from "@/src/api";
export async function list() {
  return apiFetch<Page<Item>>(`/api/items`);
}
export async function del(): Promise<{ ok: true }> {
  return apiFetch(`/api/items`, { method: "DELETE" });
}
export async function nothing() {
  return apiFetch<void>(`/api/x`);
}
"#,
        );
        let mut p = Project::new(root, "@/");
        let (s, src) = call_shape(&mut p, &f, 3, &["apiFetch"]).unwrap();
        assert_eq!(src, ShapeSource::TypeArgument);
        let item = obj(&[
            ("id", Shape::Number, false),
            ("name", Shape::String, false),
            (
                "status",
                Shape::Union {
                    of: vec![
                        Shape::Lit { v: "draft".into() },
                        Shape::Lit { v: "done".into() },
                    ],
                },
                false,
            ),
            (
                "meta",
                Shape::Union {
                    of: vec![
                        Shape::Record {
                            of: Box::new(Shape::Any),
                        },
                        Shape::Null,
                    ],
                },
                true,
            ),
        ]);
        let want = obj(&[
            ("items", Shape::Array { of: Box::new(item) }, false),
            ("total", Shape::Number, false),
        ]);
        assert_eq!(s, want);
        assert!(s.unresolved().is_empty());

        let (s, src) = call_shape(&mut p, &f, 6, &["apiFetch"]).unwrap();
        assert_eq!(src, ShapeSource::ReturnType);
        assert_eq!(s, obj(&[("ok", Shape::Lit { v: true.into() }, false)]));

        let (s, _) = call_shape(&mut p, &f, 9, &["apiFetch"]).unwrap();
        assert_eq!(s, Shape::Void);
    }

    #[test]
    fn multiline_calls_comments_and_import_types() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "src/t.ts",
            "export type K =\n  | \"a\"\n  // note\n  | \"b\";\nexport type R = { k: K };\n",
        );
        let f = write(
            root,
            "src/f.ts",
            r#"export async function a() {
  const res = await apiFetch<{ data: string[] }>(
    `/api/a`,
  );
  return res;
}
export const b = async (): Promise<import("@/src/t").R> => {
  return apiFetch(`/api/b`);
};
"#,
        );
        let mut p = Project::new(root, "@/");
        let (s, _) = call_shape(&mut p, &f, 3, &["apiFetch"]).unwrap();
        assert_eq!(
            s,
            obj(&[(
                "data",
                Shape::Array {
                    of: Box::new(Shape::String)
                },
                false
            )])
        );
        let (s, src) = call_shape(&mut p, &f, 8, &["apiFetch"]).unwrap();
        assert_eq!(src, ShapeSource::ReturnType);
        assert!(s.unresolved().is_empty(), "{s:?}");
    }

    #[test]
    fn check_reports_paths() {
        let shape = obj(&[
            ("id", Shape::Number, false),
            (
                "status",
                Shape::Union {
                    of: vec![Shape::Lit { v: "a".into() }, Shape::Lit { v: "b".into() }],
                },
                false,
            ),
            ("note", Shape::String, true),
        ]);
        let list = Shape::Array {
            of: Box::new(shape),
        };
        let v = serde_json::json!([{"id": 1, "status": "a"}, {"id": "x", "status": "c", "extra": 1}, {}]);
        assert_eq!(
            list.check(&v),
            vec![
                "$[1].id: number 이어야 하는데 string",
                "$[1].status: \"c\" 는 허용값(\"a\" | \"b\")이 아님",
                "$[2].id: 필수 필드 없음",
                "$[2].status: 필수 필드 없음",
            ]
        );
    }

    #[test]
    fn value_hints_from_comparisons() {
        let dir = tempfile::tempdir().unwrap();
        let f = write(
            dir.path(),
            "a.tsx",
            r#"const statusMap: Record<string, string> = { active: "진행 중", "on-hold": "보류" };
const label = statusMap[item.status];
const x = p.status === "done" || "review" === item.status;
switch (row.kind) { case "qa": break; case "sum": break; }
const y = ["draft", "reject"].includes(e.status);
if (code !== 404) {}
"#,
        );
        let mut p = Project::new(dir.path(), "@/");
        let h = value_hints(&mut p, &[f]).unwrap();
        let get = |k: &str| h[k].iter().cloned().collect::<Vec<_>>();
        assert_eq!(
            get("status"),
            vec!["active", "done", "draft", "on-hold", "reject", "review"]
        );
        assert_eq!(get("kind"), vec!["qa", "sum"]);
        assert!(!h.contains_key("code"));
    }

    #[test]
    fn omit_and_unknown_names() {
        let dir = tempfile::tempdir().unwrap();
        let f = write(
            dir.path(),
            "a.ts",
            r#"type A = { a: string; b: number };
const x = () => apiFetch<Omit<A, "b"> & { c: Missing }>(`/api/a`);
"#,
        );
        let mut p = Project::new(dir.path(), "@/");
        let (s, _) = call_shape(&mut p, &f, 2, &["apiFetch"]).unwrap();
        assert_eq!(
            s,
            obj(&[
                ("a", Shape::String, false),
                (
                    "c",
                    Shape::Unresolved {
                        text: "Missing 정의 없음".into()
                    },
                    false
                ),
            ])
        );
        assert_eq!(s.unresolved(), vec!["Missing 정의 없음".to_string()]);
    }
}
