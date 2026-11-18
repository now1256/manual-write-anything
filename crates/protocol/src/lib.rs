//! Shared types: configuration (`mwa.toml`) and LLM request/response shapes.

pub mod config;
pub mod llm;

pub use config::{
    BackendCfg, CaptureCfg, Config, ConfigError, ProductCfg, ProfileCfg, ProfilePurpose, Role,
    RoleCfg, SourceCfg,
};
pub use llm::{Quota, Request, Response, Usage};
