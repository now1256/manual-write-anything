//! Manual generation for one feature (M3): code slice → labels and facts → template →
//! blocks → verification → comparison with an existing manual.

pub mod compare;
pub mod facts;
pub mod labels;
pub mod model;
pub mod pipeline;
pub mod render;
pub mod shotplan;
pub mod slice;
pub mod template;
pub mod verify;
pub mod world;
pub mod write;

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
