//! Static evaluation of URL-building expressions: literals, constants (local and imported),
//! `+` concatenation, and inlining of small same-file helper functions.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use tree_sitter::Node;

use crate::MapError;
use crate::graph::Project;
use crate::ts::{Part, Src};

const MAX_INLINE_DEPTH: usize = 3;

/// Parameter bindings while inlining a helper.
pub type Env = HashMap<String, Vec<Part>>;

pub struct Expander<'a> {
    src: &'a Src,
    pub consts: HashMap<String, String>,
    pub imported: HashMap<String, String>,
    /// Byte ranges of helper functions inlined so far.
    pub inlined: RefCell<Vec<std::ops::Range<usize>>>,
}

impl<'a> Expander<'a> {
    pub fn new(src: &'a Src, imported: HashMap<String, String>) -> Self {
        Self {
            src,
            consts: src.string_consts(),
            imported,
            inlined: RefCell::new(Vec::new()),
        }
    }

    /// Literal parts an expression evaluates to, as far as statically known.
    pub fn expand(&self, n: Node<'a>, env: &Env, depth: usize) -> Option<Vec<Part>> {
        let src = self.src;
        match n.kind() {
            "string" => src.parts(n),
            "template_string" => {
                let mut out = Vec::new();
                for p in src.parts(n)? {
                    match p {
                        Part::Lit(s) => out.push(Part::Lit(s)),
                        Part::Sub(text) => out.extend(self.expand_sub(n, &text, env, depth)),
                    }
                }
                Some(out)
            }
            "identifier" | "shorthand_property_identifier" => {
                let name = src.text_of(n);
                env.get(name).cloned().or_else(|| {
                    self.consts
                        .get(name)
                        .or_else(|| self.imported.get(name))
                        .map(|v| vec![Part::Lit(v.clone())])
                })
            }
            "parenthesized_expression"
            | "as_expression"
            | "non_null_expression"
            | "satisfies_expression" => {
                let mut c = n.walk();
                let inner = n.named_children(&mut c).next()?;
                self.expand(inner, env, depth)
            }
            "binary_expression"
                if n.child_by_field_name("operator")
                    .is_some_and(|o| src.text_of(o) == "+") =>
            {
                let mut l = self.expand(n.child_by_field_name("left")?, env, depth)?;
                let r = self.expand_or_sub(n.child_by_field_name("right")?, env, depth);
                l.extend(r);
                Some(l)
            }
            "call_expression" => {
                let (body, env2) = self.inline(n, env, depth)?;
                self.expand(body, &env2, depth + 1)
            }
            _ => None,
        }
    }

    pub fn expand_or_sub(&self, n: Node<'a>, env: &Env, depth: usize) -> Vec<Part> {
        self.expand(n, env, depth)
            .unwrap_or_else(|| vec![Part::Sub(self.src.text_of(n).to_string())])
    }

    /// Expands a `${...}` of `template` by locating its expression node.
    fn expand_sub(&self, template: Node<'a>, text: &str, env: &Env, depth: usize) -> Vec<Part> {
        let mut c = template.walk();
        let expr = template
            .named_children(&mut c)
            .filter(|ch| ch.kind() == "template_substitution")
            .filter_map(|s| {
                let mut c2 = s.walk();
                s.named_children(&mut c2).next()
            })
            .find(|e| self.src.text_of(*e).trim() == text);
        match expr {
            Some(e) => self.expand_or_sub(e, env, depth),
            None => vec![Part::Sub(text.to_string())],
        }
    }

    /// For a call to a same-file helper, returns its result expression and parameter bindings.
    pub fn inline(&self, call: Node<'a>, env: &Env, depth: usize) -> Option<(Node<'a>, Env)> {
        if depth >= MAX_INLINE_DEPTH {
            return None;
        }
        let src = self.src;
        let f = call.child_by_field_name("function")?;
        if f.kind() != "identifier" {
            return None;
        }
        let def = function_def(src, src.text_of(f))?;
        self.inlined.borrow_mut().push(def.byte_range());
        let params = def.child_by_field_name("parameters")?;
        let body = def.child_by_field_name("body")?;
        let result = if body.kind() == "statement_block" {
            let mut c = body.walk();
            let ret = body
                .named_children(&mut c)
                .filter(|s| s.kind() == "return_statement")
                .last()?;
            let mut c2 = ret.walk();
            ret.named_children(&mut c2).next()?
        } else {
            body
        };
        let args = src.args(call);
        let mut env2 = Env::new();
        let mut pc = params.walk();
        for (i, p) in params
            .named_children(&mut pc)
            .filter(|p| p.kind().ends_with("parameter"))
            .enumerate()
        {
            let Some(pat) = p
                .child_by_field_name("pattern")
                .filter(|x| x.kind() == "identifier")
            else {
                continue;
            };
            let name = src.text_of(pat).to_string();
            let bound = match args.get(i) {
                Some(a) => self.expand_or_sub(*a, env, depth + 1),
                None => p
                    .child_by_field_name("value")
                    .and_then(|v| self.expand(v, env, depth + 1))
                    .unwrap_or_default(),
            };
            env2.insert(name, bound);
        }
        Some((result, env2))
    }

    /// HTTP method from a request-init argument: an object literal or a same-file helper returning one.
    pub fn method(&self, init: Option<Node<'a>>) -> String {
        let Some(init) = init else {
            return "GET".into();
        };
        let (obj, env) = match init.kind() {
            "object" => (Some(init), Env::new()),
            "call_expression" => match self.inline(init, &Env::new(), 0) {
                Some((body, env)) => (Some(unwrap_parens(body)), env),
                None => (None, Env::new()),
            },
            _ => (None, Env::new()),
        };
        let Some(obj) = obj else { return "?".into() };
        match self.src.prop(obj, "method") {
            None => "GET".into(),
            Some(v) => match self.expand(v, &env, 0).as_deref() {
                Some([Part::Lit(m)]) => m.to_uppercase(),
                _ => "?".into(),
            },
        }
    }
}

pub fn unwrap_parens(n: Node) -> Node {
    let mut n = n;
    while n.kind() == "parenthesized_expression" {
        let mut c = n.walk();
        match n.named_children(&mut c).next() {
            Some(inner) => n = inner,
            None => break,
        }
    }
    n
}

/// `const f = (...) => ...` or `function f(...) {...}` at top level.
pub fn function_def<'a>(src: &'a Src, name: &str) -> Option<Node<'a>> {
    if let Some(v) = src.top_level_value(name).filter(|v| {
        matches!(
            v.kind(),
            "arrow_function" | "function_expression" | "function"
        )
    }) {
        return Some(v);
    }
    src.nodes_of_kind("function_declaration")
        .into_iter()
        .find(|f| {
            f.child_by_field_name("name")
                .is_some_and(|n| src.text_of(n) == name)
        })
}

/// `const url = \`/api/..\`; apiFetch(url)`: the initializer of a same-file variable declared before the call.
pub fn local_var_literal<'a>(src: &'a Src, arg: Node<'a>, call: Node<'a>) -> Option<Node<'a>> {
    if arg.kind() != "identifier" {
        return None;
    }
    let name = src.text_of(arg);
    src.nodes_of_kind("variable_declarator")
        .into_iter()
        .filter(|d| d.start_byte() < call.start_byte())
        .filter(|d| {
            d.child_by_field_name("name")
                .is_some_and(|n| src.text_of(n) == name)
        })
        .filter_map(|d| d.child_by_field_name("value"))
        .rfind(|v| v.kind() != "arrow_function")
}

/// String constants imported by name from other project modules.
pub fn imported_consts(
    project: &mut Project,
    file: &Path,
) -> Result<HashMap<String, String>, MapError> {
    let mut out = HashMap::new();
    let imports = project.src(file)?.imports();
    for imp in imports {
        let Some(target) = project.resolve(file, &imp.source) else {
            continue;
        };
        let consts = project.src(&target)?.string_consts();
        for (imported, local) in imp.names {
            if let Some(v) = consts.get(&imported) {
                out.insert(local, v.clone());
            }
        }
    }
    Ok(out)
}
