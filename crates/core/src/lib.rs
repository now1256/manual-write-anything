//! Orchestration primitives: run log, budget, per-backend concurrency, retry and fallback.

pub mod dispatch;
pub mod runlog;

pub use dispatch::{CallOutcome, CoreError, Dispatcher};
pub use runlog::{CallRecord, LogError, RunLog};
