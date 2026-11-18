//! Import resolution and symbol-level reachability.
//!
//! File-level reachability over-attaches APIs: importing one hook from a module would pull
//! every fetcher in it. Instead we follow only the declarations a screen actually references.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::MapError;
use crate::fsutil::normalize;
use crate::ts::{Src, parse_file};

/// Top-level structure of one module.
#[derive(Debug, Default)]
struct ModIndex {
    /// Declared names → byte range of their declaration.
    decls: HashMap<String, Range<usize>>,
    /// `export { local as exported }` → exported → local.
    aliases: HashMap<String, String>,
    /// `export default ...`: range and, if it is a bare identifier, the name it refers to.
    default: Option<(Range<usize>, Option<String>)>,
    /// `import { a as b } from "x"` → local b → (spec, imported a). Namespace imports use "*".
    imports: HashMap<String, (String, String)>,
    /// `export { a as b } from "x"` → exported b → (spec, a).
    reexports: HashMap<String, (String, String)>,
    /// `export * from "x"`.
    stars: Vec<String>,
    /// Top-level statements that are not declarations (run on import).
    init: Vec<Range<usize>>,
}

/// What part of a file a screen reaches.
#[derive(Debug, Clone, PartialEq)]
pub enum Coverage {
    Whole,
    Ranges(Vec<Range<usize>>),
}

impl Coverage {
    pub fn contains(&self, byte: usize) -> bool {
        match self {
            Coverage::Whole => true,
            Coverage::Ranges(rs) => rs.iter().any(|r| r.contains(&byte)),
        }
    }
}

/// Parsed-file cache plus module resolution for one frontend repository.
pub struct Project {
    pub root: PathBuf,
    alias_prefix: String,
    files: HashMap<PathBuf, Src>,
    index: HashMap<PathBuf, ModIndex>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Target {
    Whole,
    Name(String),
}

impl Project {
    pub fn new(root: &Path, alias_prefix: &str) -> Self {
        Self {
            root: root.to_path_buf(),
            alias_prefix: alias_prefix.to_string(),
            files: HashMap::new(),
            index: HashMap::new(),
        }
    }

    pub fn src(&mut self, path: &Path) -> Result<&Src, MapError> {
        if !self.files.contains_key(path) {
            let src = parse_file(path)?;
            self.files.insert(path.to_path_buf(), src);
        }
        Ok(&self.files[path])
    }

    /// Resolves an import specifier to a source file; packages resolve to `None`.
    pub fn resolve(&self, from: &Path, spec: &str) -> Option<PathBuf> {
        let base = if let Some(rest) = spec.strip_prefix(&self.alias_prefix) {
            self.root.join(rest)
        } else if spec.starts_with('.') {
            from.parent()?.join(spec)
        } else {
            return None;
        };
        let base = normalize(&base);
        let has_ext = base
            .extension()
            .is_some_and(|e| matches!(e.to_str(), Some("ts" | "tsx")));
        let mut candidates = Vec::new();
        if has_ext {
            candidates.push(base.clone());
        }
        let s = base.to_string_lossy();
        candidates.extend([
            PathBuf::from(format!("{s}.ts")),
            PathBuf::from(format!("{s}.tsx")),
            base.join("index.ts"),
            base.join("index.tsx"),
        ]);
        candidates.into_iter().find(|c| c.is_file())
    }

    fn ensure_index(&mut self, path: &Path) -> Result<(), MapError> {
        if !self.index.contains_key(path) {
            let idx = build_index(self.src(path)?);
            self.index.insert(path.to_path_buf(), idx);
        }
        Ok(())
    }

    /// Declarations reachable from `entry` (whole file) through referenced names.
    /// Files under `stop_dirs` (relative to root) are never entered.
    pub fn reach(
        &mut self,
        entry: &Path,
        stop_dirs: &[&str],
    ) -> Result<BTreeMap<PathBuf, Coverage>, MapError> {
        let stops: Vec<PathBuf> = stop_dirs.iter().map(|d| self.root.join(d)).collect();
        let mut ranges: BTreeMap<PathBuf, Vec<Range<usize>>> = BTreeMap::new();
        let mut whole: BTreeSet<PathBuf> = BTreeSet::new();
        let mut seen: BTreeSet<(PathBuf, Target)> = BTreeSet::new();
        let mut queue = VecDeque::from([(entry.to_path_buf(), Target::Whole)]);

        while let Some((file, target)) = queue.pop_front() {
            if !seen.insert((file.clone(), target.clone())) {
                continue;
            }
            self.ensure_index(&file)?;
            let first_visit = !ranges.contains_key(&file) && !whole.contains(&file);
            let idx = &self.index[&file];
            let mut scan: Vec<Range<usize>> = Vec::new();
            let mut follow: Vec<(Option<PathBuf>, Target)> = Vec::new();
            if first_visit {
                scan.extend(idx.init.iter().cloned());
            }
            match &target {
                Target::Whole => {
                    whole.insert(file.clone());
                    scan.push(0..self.files[&file].text.len());
                }
                Target::Name(name) => {
                    let local = idx.aliases.get(name).unwrap_or(name);
                    if let Some(r) = idx.decls.get(local) {
                        scan.push(r.clone());
                    } else if name == "default" {
                        if let Some((r, ident)) = &idx.default {
                            scan.push(r.clone());
                            if let Some(id) = ident {
                                follow.push((Some(file.clone()), Target::Name(id.clone())));
                            }
                        }
                    } else if let Some((spec, imported)) = idx.reexports.get(name) {
                        follow.push((self.resolve(&file, spec), Target::Name(imported.clone())));
                    } else if let Some((spec, imported)) = idx.imports.get(local) {
                        // `import { x } from "y"; export { x };`
                        follow.push((self.resolve(&file, spec), Target::Name(imported.clone())));
                    } else {
                        for spec in &idx.stars {
                            follow.push((self.resolve(&file, spec), Target::Name(name.clone())));
                        }
                    }
                }
            }

            let src = &self.files[&file];
            for r in &scan {
                for (ident, member) in identifiers_in(src, r.clone()) {
                    if idx.decls.contains_key(&ident) {
                        follow.push((Some(file.clone()), Target::Name(ident)));
                    } else if let Some((spec, imported)) = idx.imports.get(&ident) {
                        let t = match (imported.as_str(), member) {
                            ("*", Some(m)) => Target::Name(m),
                            ("*", None) => Target::Whole,
                            (name, _) => Target::Name(name.to_string()),
                        };
                        follow.push((self.resolve(&file, spec), t));
                    }
                }
            }
            if !whole.contains(&file) {
                ranges.entry(file.clone()).or_default().extend(scan);
            }
            for (target_file, t) in follow {
                if let Some(f) = target_file
                    && !stops.iter().any(|s| f.starts_with(s))
                {
                    queue.push_back((f, t));
                }
            }
        }

        let mut out: BTreeMap<PathBuf, Coverage> = ranges
            .into_iter()
            .filter(|(_, rs)| !rs.is_empty())
            .map(|(f, rs)| (f, Coverage::Ranges(rs)))
            .collect();
        for f in whole {
            out.insert(f, Coverage::Whole);
        }
        Ok(out)
    }
}

/// Identifiers referenced in a byte range. For `ns.member` the member name is returned too.
fn identifiers_in(src: &Src, range: Range<usize>) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for n in src.nodes() {
        if n.start_byte() < range.start || n.end_byte() > range.end {
            continue;
        }
        if !matches!(n.kind(), "identifier" | "shorthand_property_identifier") {
            continue;
        }
        if in_module_syntax(n) {
            continue;
        }
        let member = n
            .parent()
            .filter(|p| {
                p.kind() == "member_expression" && p.child_by_field_name("object") == Some(n)
            })
            .and_then(|p| p.child_by_field_name("property"))
            .map(|p| src.text_of(p).to_string());
        out.push((src.text_of(n).to_string(), member));
    }
    out
}

/// Names inside `import`/`export ... from` statements are bindings, not references.
fn in_module_syntax(n: tree_sitter::Node) -> bool {
    let mut p = n.parent();
    while let Some(x) = p {
        match x.kind() {
            "import_statement" | "export_clause" => return true,
            "program" => return false,
            _ => p = x.parent(),
        }
    }
    false
}

fn build_index(src: &Src) -> ModIndex {
    let mut idx = ModIndex::default();
    let root = src.root();
    let mut cursor = root.walk();
    for stmt in root.children(&mut cursor) {
        match stmt.kind() {
            "import_statement" => {
                if let Some(spec) = stmt
                    .child_by_field_name("source")
                    .and_then(|s| src.string_value(s))
                {
                    for imp in src.imports().into_iter().filter(|i| i.source == spec) {
                        for (imported, local) in imp.names {
                            idx.imports.insert(local, (spec.clone(), imported));
                        }
                    }
                }
            }
            "export_statement" => index_export(src, stmt, &mut idx),
            _ => {
                if !declare(src, stmt, &mut idx) && stmt.is_named() && stmt.kind() != "comment" {
                    idx.init.push(stmt.byte_range());
                }
            }
        }
    }
    idx
}

fn index_export(src: &Src, stmt: tree_sitter::Node, idx: &mut ModIndex) {
    let source = stmt
        .child_by_field_name("source")
        .and_then(|s| src.string_value(s));
    let mut cursor = stmt.walk();
    let children: Vec<_> = stmt.named_children(&mut cursor).collect();
    let clause = children.iter().find(|c| c.kind() == "export_clause");
    match (source, clause) {
        (Some(spec), Some(clause)) => {
            for (name, alias) in specifiers(src, *clause) {
                idx.reexports
                    .insert(alias.unwrap_or_else(|| name.clone()), (spec.clone(), name));
            }
        }
        (Some(spec), None) => idx.stars.push(spec),
        (None, Some(clause)) => {
            for (name, alias) in specifiers(src, *clause) {
                if let Some(alias) = alias {
                    idx.aliases.insert(alias, name);
                }
            }
        }
        (None, None) => {
            let is_default = {
                let mut c = stmt.walk();
                stmt.children(&mut c).any(|ch| ch.kind() == "default")
            };
            if let Some(decl) = stmt.child_by_field_name("declaration") {
                if is_default {
                    idx.default = Some((stmt.byte_range(), None));
                }
                declare(src, decl, idx);
            } else if let Some(value) = stmt.child_by_field_name("value") {
                let ident = (value.kind() == "identifier").then(|| src.text_of(value).to_string());
                idx.default = Some((stmt.byte_range(), ident));
            }
        }
    }
}

fn specifiers(src: &Src, clause: tree_sitter::Node) -> Vec<(String, Option<String>)> {
    let mut c = clause.walk();
    clause
        .named_children(&mut c)
        .filter(|n| n.kind() == "export_specifier")
        .filter_map(|n| {
            let name = src.text_of(n.child_by_field_name("name")?).to_string();
            let alias = n
                .child_by_field_name("alias")
                .map(|a| src.text_of(a).to_string());
            Some((name, alias))
        })
        .collect()
}

/// Records declarations; returns false if `node` declares nothing.
fn declare(src: &Src, node: tree_sitter::Node, idx: &mut ModIndex) -> bool {
    match node.kind() {
        "function_declaration"
        | "generator_function_declaration"
        | "class_declaration"
        | "abstract_class_declaration"
        | "enum_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                idx.decls
                    .insert(src.text_of(name).to_string(), node.byte_range());
            }
            true
        }
        "lexical_declaration" | "variable_declaration" => {
            let mut c = node.walk();
            for d in node
                .named_children(&mut c)
                .filter(|n| n.kind() == "variable_declarator")
            {
                if let Some(name) = d
                    .child_by_field_name("name")
                    .filter(|n| n.kind() == "identifier")
                {
                    idx.decls
                        .insert(src.text_of(name).to_string(), d.byte_range());
                }
            }
            true
        }
        // Types never reach runtime calls.
        "interface_declaration" | "type_alias_declaration" | "ambient_declaration" => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil::rel;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn covered_text(p: &mut Project, cov: &BTreeMap<PathBuf, Coverage>, file: &Path) -> String {
        let src = p.src(file).unwrap();
        match &cov[file] {
            Coverage::Whole => src.text.clone(),
            Coverage::Ranges(rs) => rs
                .iter()
                .map(|r| &src.text[r.clone()])
                .collect::<Vec<_>>()
                .join("|"),
        }
    }

    #[test]
    fn follows_only_referenced_symbols_through_barrels() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(
            r,
            "src/app/x/page.tsx",
            r#"import { useA } from "@/src/lib/hooks"; import Def from "./_c/d"; import * as api from "@/src/lib/api/ns"; import ctx from "@/src/lib/contexts/u";
export default function Page() { useA(); api.three(); return <Def />; }"#,
        );
        write(
            r,
            "src/app/x/_c/d.tsx",
            r#"const helper = () => "h"; export default function D() { return helper(); }"#,
        );
        write(
            r,
            "src/lib/hooks/index.ts",
            r#"export * from "./use-a"; export { useB as useB2 } from "./use-b";"#,
        );
        write(
            r,
            "src/lib/hooks/use-a.ts",
            r#"import { fetchA, fetchZ } from "../api/f"; export const useA = () => fetchA(); export const useZ = () => fetchZ();"#,
        );
        write(
            r,
            "src/lib/hooks/use-b.ts",
            r#"export const useB = () => 1;"#,
        );
        write(
            r,
            "src/lib/api/f.ts",
            r#"export const fetchA = () => apiFetch("/api/a"); export const fetchZ = () => apiFetch("/api/z");"#,
        );
        write(
            r,
            "src/lib/api/ns.ts",
            r#"export const three = () => 3; export const four = () => 4;"#,
        );
        write(r, "src/lib/contexts/u.tsx", r#"export default 1;"#);

        let mut p = Project::new(r, "@/");
        let cov = p
            .reach(&r.join("src/app/x/page.tsx"), &["src/lib/contexts"])
            .unwrap();
        let files: Vec<String> = cov.keys().map(|f| rel(r, f)).collect();
        assert_eq!(
            files,
            vec![
                "src/app/x/_c/d.tsx",
                "src/app/x/page.tsx",
                "src/lib/api/f.ts",
                "src/lib/api/ns.ts",
                "src/lib/hooks/use-a.ts",
            ],
            "barrel index has no declarations; use-b and contexts not reached"
        );
        let f = covered_text(&mut p, &cov, &r.join("src/lib/api/f.ts"));
        assert!(f.contains("/api/a") && !f.contains("/api/z"), "{f}");
        let ns = covered_text(&mut p, &cov, &r.join("src/lib/api/ns.ts"));
        assert!(ns.contains("three") && !ns.contains("four"), "{ns}");
        let d = covered_text(&mut p, &cov, &r.join("src/app/x/_c/d.tsx"));
        assert!(d.contains("helper = "), "local helper followed: {d}");
    }

    #[test]
    fn resolves_relative_alias_and_index() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "src/a/b.ts", "");
        write(r, "src/c/index.tsx", "");
        let p = Project::new(r, "@/");
        let from = r.join("src/a/x.ts");
        assert_eq!(p.resolve(&from, "./b"), Some(r.join("src/a/b.ts")));
        assert_eq!(p.resolve(&from, "@/src/c"), Some(r.join("src/c/index.tsx")));
        assert_eq!(p.resolve(&from, "../c"), Some(r.join("src/c/index.tsx")));
        assert_eq!(p.resolve(&from, "react"), None);
    }
}
