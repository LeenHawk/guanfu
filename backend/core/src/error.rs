use crate::llm::capability::Capability;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("database error: {0}")]
    Db(#[from] sea_orm::DbErr),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("endpoint resolution failed: {0}")]
    Endpoint(String),

    #[error("protocol transform failed: {0}")]
    Transform(String),

    #[error("channel {0} not found")]
    ChannelNotFound(i32),

    #[error("channel {0} has no usable credential")]
    NoUsableCredential(i32),

    #[error("unknown provider kind: {0}")]
    UnknownProvider(String),

    #[error("capability {0:?} is not supported by this channel")]
    UnsupportedCapability(Capability),

    #[error("upstream returned status {status}")]
    Upstream { status: u16, body: String },
}
