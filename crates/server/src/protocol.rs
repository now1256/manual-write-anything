//! WebSocket messages (JSON, tagged by `type`). Mirrored in web/src/api.ts.

use mwa_host::{Status, Who};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptItem {
    pub who: Who,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// One chat turn; `transcript` ends with the new user message.
    Chat {
        id: u64,
        transcript: Vec<TranscriptItem>,
        /// Key of the feature open in the UI, if any.
        selected: Option<String>,
    },
    /// `map`, `doctor`, `auth`, `shot`, or `gen` (with `arg` = feature key).
    Command {
        id: u64,
        name: String,
        #[serde(default)]
        arg: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Delta {
        id: u64,
        text: String,
    },
    ChatDone {
        id: u64,
    },
    Lines {
        id: u64,
        title: String,
        lines: Vec<String>,
    },
    /// The product map changed; clients refetch `/api/map`.
    MapBuilt {
        id: u64,
        lines: Vec<String>,
    },
    Error {
        id: u64,
        message: String,
    },
    /// One progress line of a long command (e.g. capture).
    Progress {
        id: u64,
        text: String,
    },
    /// Manual generated; clients refetch `/api/manual/{key}`.
    ManualDone {
        id: u64,
        key: String,
        lines: Vec<String>,
    },
    /// Capture finished; clients refetch `/api/shots`.
    ShotsDone {
        id: u64,
        lines: Vec<String>,
    },
    Status {
        status: Status,
    },
}
