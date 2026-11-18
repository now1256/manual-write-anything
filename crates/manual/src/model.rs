//! Data written to the workspace (`manual/<feature>.*.json`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repo {
    Frontend,
    Backend,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub repo: Repo,
    pub file: String,
    pub line_start: usize,
    pub line_end: usize,
    /// Exact code excerpt the fact rests on (checked against the file).
    pub quote: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    /// Visible text: buttons, columns, cards, placeholders.
    Label,
    /// Input rules and constraints (required, length, format, editability).
    Rule,
    /// What happens when the user acts (code-derived, not executed).
    Behavior,
    /// Notices and error messages shown to the user.
    Message,
    /// Who may see or do what (deterministic, from the product map).
    Permission,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub id: String,
    /// User-facing Korean statement.
    pub statement: String,
    pub kind: FactKind,
    pub source: Option<Source>,
    /// `code` = read from code only (behavior not executed); `deterministic` = parser/map output.
    pub basis: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Heading,
    Paragraph,
    Table,
    Steps,
    Callout,
    Image,
    /// Something the manual needs but no fact supports; for a person to fill in.
    Todo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub id: String,
    pub kind: BlockKind,
    pub markdown: String,
    #[serde(default)]
    pub facts: Vec<String>,
    /// Screenshot id (e.g. `menus/user-account`) for image blocks.
    #[serde(default)]
    pub shot: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionSpec {
    pub title: String,
    pub purpose: String,
    #[serde(default)]
    pub typical_blocks: Vec<String>,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    /// Where it was learned from.
    pub source: String,
    pub sections: Vec<SectionSpec>,
    pub style_rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockCheck {
    pub id: String,
    /// Deterministic problems (missing evidence, unknown label, ...).
    pub issues: Vec<String>,
    /// Judge verdict: `supported`, `partial`, `unsupported` or `skipped`.
    pub verdict: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompareItem {
    pub topic: String,
    pub generated: Option<String>,
    pub existing: Option<String>,
    /// `same`, `only_existing`, `only_generated`, `conflict`
    pub status: String,
    /// For conflicts: which side the code supports and why.
    #[serde(default)]
    pub code_says: Option<String>,
    /// `generated`, `existing` or `unknown`: the side the code facts back.
    #[serde(default)]
    pub code_supports: Option<String>,
    #[serde(default)]
    pub facts: Vec<String>,
}

/// Everything produced for one feature, saved as `manual/<key>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManualRun {
    pub feature: String,
    pub title: String,
    pub generated_at: u64,
    pub models: Vec<String>,
    pub cost_usd: f64,
    pub facts: Vec<Fact>,
    pub dropped_facts: usize,
    pub blocks: Vec<Block>,
    pub checks: Vec<BlockCheck>,
    pub compare: Option<Vec<CompareItem>>,
    pub compare_source: Option<String>,
}
