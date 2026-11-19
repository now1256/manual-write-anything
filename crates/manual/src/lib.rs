//! Manuals for one product: old manuals (reference), code facts (pipeline, facts), documents
//! written from them (rewrite, document) with their history and basis (provenance, grounding).

/// What a user manual may say, shared by every writing prompt (D-046).
macro_rules! user_rule {
    () => {
        "Write only what a user sees on the screen and can do, in the words the screen uses. \
Never write: how the server checks (or does not check) permissions, internal APIs, addresses \
(URLs, routes, file paths), request or \
response contents, what is returned, sessions, tokens, audit or history records the user cannot \
see, logs or the console, caches, when validation runs, or any security weakness. Keep a message \
only when the user's own input or action causes it and the user can act on it (a required field, \
a wrong format, a duplicate name); leave out network failures and generic error fallbacks. \
Quote a message whole, as the user reads it: code often builds it from parts (a field name, a \
number), so fill those in from the facts ('시스템명을 입력해주세요', '100자 이하로 입력해주세요') \
or describe it, never quote a fragment. Leave out what any user takes for granted: lists and \
counts refreshing after saving, loading indicators, the previous page staying until the next \
one loads. Say a thing once: a warning panel must not repeat the sentence above it, and one \
rule is not written in two places. "
    };
}

pub mod document;
pub mod facts;
pub mod grounding;
pub mod labels;
pub mod model;
pub mod pipeline;
pub mod provenance;
pub mod reference;
pub mod render;
pub mod rewrite;
pub mod shotplan;
pub mod slice;
pub mod template;
pub mod versions;
pub mod world;

#[derive(Debug, thiserror::Error)]
pub enum ManualError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Map(#[from] mwa_productmap::MapError),
    #[error(transparent)]
    Core(#[from] mwa_core::CoreError),
    #[error("{0}: {1}")]
    Io(std::path::PathBuf, #[source] std::io::Error),
    #[error("모델 응답 형식 오류: {0}")]
    Shape(String),
}
