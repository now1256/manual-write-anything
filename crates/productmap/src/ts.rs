//! Thin helpers over tree-sitter-typescript: parsing, literals, imports, calls.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tree_sitter::{Node, Parser, Tree};

use crate::MapError;

pub struct Src {
    pub path: PathBuf,
    pub text: String,
    pub tree: Tree,
}

/// A piece of a string or template literal.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Lit(String),
    /// `${...}` substitution, holding the expression text.
    Sub(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub source: String,
    /// (imported name, local name); default imports use "default".
    pub names: Vec<(String, String)>,
}

pub fn parse_file(path: &Path) -> Result<Src, MapError> {
    let text = std::fs::read_to_string(path).map_err(|e| MapError::Io(path.to_path_buf(), e))?;
    parse_str(path, text)
}

pub fn parse_str(path: &Path, text: String) -> Result<Src, MapError> {
    let tsx = path.extension().is_some_and(|e| e == "tsx");
    let lang = if tsx {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };
    let mut parser = Parser::new();
    parser
        .set_language(&lang.into())
        .map_err(|e| MapError::Parse(path.to_path_buf(), e.to_string()))?;
    let tree = parser
        .parse(&text, None)
        .ok_or_else(|| MapError::Parse(path.to_path_buf(), "파싱 실패".into()))?;
    Ok(Src {
        path: path.to_path_buf(),
        text,
        tree,
    })
}

impl Src {
    pub fn root(&self) -> Node<'_> {
        self.tree.root_node()
    }

    pub fn text_of(&self, n: Node) -> &str {
        &self.text[n.byte_range()]
    }

    /// 1-based line number.
    pub fn line(&self, n: Node) -> usize {
        n.start_position().row + 1
    }

    /// All nodes in document order.
    pub fn nodes(&self) -> Vec<Node<'_>> {
        let mut out = Vec::new();
        let mut cursor = self.root().walk();
        'outer: loop {
            out.push(cursor.node());
            if cursor.goto_first_child() {
                continue;
            }
            loop {
                if cursor.goto_next_sibling() {
                    continue 'outer;
                }
                if !cursor.goto_parent() {
                    break 'outer;
                }
            }
        }
        out
    }

    pub fn nodes_of_kind<'a>(&'a self, kind: &str) -> Vec<Node<'a>> {
        self.nodes()
            .into_iter()
            .filter(|n| n.kind() == kind)
            .collect()
    }

    /// Parts of a `string` or `template_string` node; `None` for anything else.
    pub fn parts(&self, n: Node) -> Option<Vec<Part>> {
        match n.kind() {
            "string" => {
                let raw = self.text_of(n);
                Some(vec![Part::Lit(raw[1..raw.len() - 1].to_string())])
            }
            "template_string" => {
                let mut parts = Vec::new();
                let mut cursor = n.walk();
                // Literal text sits between child nodes; walk byte ranges to keep it exact.
                let inner_start = n.start_byte() + 1;
                let inner_end = n.end_byte() - 1;
                let mut pos = inner_start;
                for child in n.children(&mut cursor) {
                    if child.kind() != "template_substitution" {
                        continue;
                    }
                    if child.start_byte() > pos {
                        parts.push(Part::Lit(self.text[pos..child.start_byte()].to_string()));
                    }
                    let expr = self.text_of(child);
                    parts.push(Part::Sub(expr[2..expr.len() - 1].trim().to_string()));
                    pos = child.end_byte();
                }
                if inner_end > pos {
                    parts.push(Part::Lit(self.text[pos..inner_end].to_string()));
                }
                Some(parts)
            }
            _ => None,
        }
    }

    /// Plain string value of a string literal (or a template without substitutions).
    pub fn string_value(&self, n: Node) -> Option<String> {
        match self.parts(n)?.as_slice() {
            [] => Some(String::new()),
            [Part::Lit(s)] => Some(s.clone()),
            _ => None,
        }
    }

    /// Top-level `const NAME = "literal"` (exported or not).
    pub fn string_consts(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for decl in self.top_level_declarators() {
            if let (Some(name), Some(value)) = (
                decl.child_by_field_name("name"),
                decl.child_by_field_name("value"),
            ) && let Some(v) = self.string_value(value)
            {
                out.insert(self.text_of(name).to_string(), v);
            }
        }
        out
    }

    /// `variable_declarator` nodes of top-level (optionally exported) `const`/`let`.
    pub fn top_level_declarators(&self) -> Vec<Node<'_>> {
        let mut out = Vec::new();
        let mut cursor = self.root().walk();
        for stmt in self.root().children(&mut cursor) {
            let decl = match stmt.kind() {
                "lexical_declaration" | "variable_declaration" => Some(stmt),
                "export_statement" => stmt
                    .child_by_field_name("declaration")
                    .filter(|d| matches!(d.kind(), "lexical_declaration" | "variable_declaration")),
                _ => None,
            };
            if let Some(decl) = decl {
                let mut c = decl.walk();
                out.extend(
                    decl.children(&mut c)
                        .filter(|n| n.kind() == "variable_declarator"),
                );
            }
        }
        out
    }

    /// Value node of a top-level declarator by name.
    pub fn top_level_value(&self, name: &str) -> Option<Node<'_>> {
        self.top_level_declarators().into_iter().find_map(|d| {
            let n = d.child_by_field_name("name")?;
            (self.text_of(n) == name).then(|| d.child_by_field_name("value"))?
        })
    }

    pub fn imports(&self) -> Vec<Import> {
        let mut out = Vec::new();
        for stmt in self.nodes() {
            let source_node = match stmt.kind() {
                "import_statement" => stmt.child_by_field_name("source"),
                // `export { x } from "./y"` and `export * from "./y"` re-exports
                "export_statement" => stmt.child_by_field_name("source"),
                _ => None,
            };
            let Some(source) = source_node.and_then(|s| self.string_value(s)) else {
                continue;
            };
            let mut names = Vec::new();
            for n in stmt_children_deep(stmt) {
                match n.kind() {
                    "import_specifier" | "export_specifier" => {
                        let name = n
                            .child_by_field_name("name")
                            .map(|x| self.text_of(x).to_string());
                        let alias = n
                            .child_by_field_name("alias")
                            .map(|x| self.text_of(x).to_string());
                        if let Some(name) = name {
                            let local = alias.unwrap_or_else(|| name.clone());
                            names.push((name, local));
                        }
                    }
                    "import_clause" => {
                        let mut c = n.walk();
                        for ch in n.children(&mut c) {
                            if ch.kind() == "identifier" {
                                names.push(("default".into(), self.text_of(ch).to_string()));
                            }
                        }
                    }
                    "namespace_import" => {
                        let mut c = n.walk();
                        if let Some(id) = n.children(&mut c).find(|x| x.kind() == "identifier") {
                            names.push(("*".into(), self.text_of(id).to_string()));
                        }
                    }
                    _ => {}
                }
            }
            out.push(Import { source, names });
        }
        out
    }

    /// Name of the called function: `f(...)` → "f", `a.b(...)` → "b".
    pub fn callee_name(&self, call: Node) -> Option<String> {
        let f = call.child_by_field_name("function")?;
        match f.kind() {
            "identifier" => Some(self.text_of(f).to_string()),
            "member_expression" => f
                .child_by_field_name("property")
                .map(|p| self.text_of(p).to_string()),
            _ => None,
        }
    }

    /// Positional argument nodes of a call or `new` expression (comments skipped).
    pub fn args<'a>(&self, call: Node<'a>) -> Vec<Node<'a>> {
        let Some(args) = call.child_by_field_name("arguments") else {
            return Vec::new();
        };
        let mut c = args.walk();
        args.named_children(&mut c)
            .filter(|n| n.kind() != "comment")
            .collect()
    }

    /// Property value of an object literal by key (`key: value` or shorthand).
    pub fn prop<'a>(&self, obj: Node<'a>, key: &str) -> Option<Node<'a>> {
        if obj.kind() != "object" {
            return None;
        }
        let mut c = obj.walk();
        obj.named_children(&mut c).find_map(|p| match p.kind() {
            "pair" => {
                let k = p.child_by_field_name("key")?;
                let name = self
                    .string_value(k)
                    .unwrap_or_else(|| self.text_of(k).to_string());
                (name == key).then(|| p.child_by_field_name("value"))?
            }
            "shorthand_property_identifier" if self.text_of(p) == key => Some(p),
            _ => None,
        })
    }

    /// String values of an array literal; non-string elements are skipped.
    pub fn string_array(&self, arr: Node) -> Vec<String> {
        if arr.kind() != "array" {
            return Vec::new();
        }
        let mut c = arr.walk();
        arr.named_children(&mut c)
            .filter_map(|n| self.string_value(n))
            .collect()
    }
}

fn stmt_children_deep(n: Node) -> Vec<Node> {
    let mut out = Vec::new();
    let mut stack = vec![n];
    while let Some(x) = stack.pop() {
        let mut c = x.walk();
        for ch in x.named_children(&mut c) {
            out.push(ch);
            stack.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(name: &str, text: &str) -> Src {
        parse_str(Path::new(name), text.to_string()).unwrap()
    }

    #[test]
    fn string_and_template_parts() {
        let s = src(
            "a.ts",
            r#"const A = "/api/x"; const B = `/api/y/${id}?db=${db}`; const C = `plain`;"#,
        );
        assert_eq!(
            s.string_consts().get("A").map(String::as_str),
            Some("/api/x")
        );
        assert_eq!(
            s.string_consts().get("C").map(String::as_str),
            Some("plain")
        );
        let b = s.top_level_value("B").unwrap();
        assert_eq!(
            s.parts(b).unwrap(),
            vec![
                Part::Lit("/api/y/".into()),
                Part::Sub("id".into()),
                Part::Lit("?db=".into()),
                Part::Sub("db".into()),
            ]
        );
    }

    #[test]
    fn imports_named_default_namespace_and_reexport() {
        let s = src(
            "a.tsx",
            r#"import X, { a, b as c } from "@/src/x";
import * as ns from "./y";
import type { T } from "./t";
export { d } from "./z";
export * from "./w";
const el = <div />;"#,
        );
        let imps = s.imports();
        assert_eq!(imps.len(), 5);
        assert_eq!(imps[0].source, "@/src/x");
        assert!(imps[0].names.contains(&("default".into(), "X".into())));
        assert!(imps[0].names.contains(&("b".into(), "c".into())));
        assert_eq!(imps[1].names, vec![("*".into(), "ns".into())]);
        assert_eq!(imps[3].names, vec![("d".into(), "d".into())]);
        assert_eq!(imps[4].source, "./w");
    }

    #[test]
    fn calls_args_and_object_props() {
        let s = src(
            "a.ts",
            r#"apiFetch(`/api/s/${id}`, { method: "DELETE", body }); api.get("/x");"#,
        );
        let calls = s.nodes_of_kind("call_expression");
        assert_eq!(s.callee_name(calls[0]).as_deref(), Some("apiFetch"));
        let args = s.args(calls[0]);
        assert_eq!(args.len(), 2);
        let m = s.prop(args[1], "method").unwrap();
        assert_eq!(s.string_value(m).as_deref(), Some("DELETE"));
        assert!(s.prop(args[1], "body").is_some());
        assert_eq!(s.callee_name(calls[1]).as_deref(), Some("get"));
        assert_eq!(s.line(calls[1]), 1);
    }
}
