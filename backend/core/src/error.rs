#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("database error: {0}")]
    Db(#[from] sea_orm::DbErr),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("invalid JSON data: {0}")]
    Json(#[from] serde_json::Error),

    #[error("endpoint resolution failed: {0}")]
    Endpoint(String),

    #[error("protocol transform failed: {0}")]
    Transform(String),

    #[error("channel {0} not found")]
    ChannelNotFound(i32),

    #[error("channel {0} has no usable credential")]
    NoUsableCredential(i32),

    #[error("invalid routing rule {id:?}: {reason}")]
    InvalidRoutingRule { id: Option<i32>, reason: String },

    #[error("channel {channel_id} does not support operation {operation:?}")]
    UnsupportedRoute {
        channel_id: i32,
        operation: gproxy_protocol::OperationKey,
    },

    #[error("routing implementation {implementation} is not executable yet")]
    UnsupportedRouteImplementation { implementation: &'static str },

    #[error("upstream returned status {status}")]
    Upstream { status: u16, body: String },
}
