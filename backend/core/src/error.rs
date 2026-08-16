use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("database error: {0}")]
    Db(#[from] sea_orm::DbErr),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("websocket error: {0}")]
    WebSocket(String),

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

    #[error("target {target:?} does not support semantic capability {capability:?}")]
    UnsupportedCapability {
        capability: crate::llm::ir::Capability,
        target: gproxy_protocol::OperationKey,
    },

    #[error("target {target:?} cannot preserve request fields {fields:?}")]
    IncompatibleRoute {
        target: gproxy_protocol::OperationKey,
        fields: Vec<String>,
    },

    #[error("unmodeled provider event for {target:?}: {event}")]
    UnmodeledProviderEvent {
        target: gproxy_protocol::OperationKey,
        event: String,
    },

    #[error("invalid provider payload for {target:?}: {reason}")]
    InvalidProviderPayload {
        target: gproxy_protocol::OperationKey,
        reason: String,
    },

    #[error("upstream returned status {status}")]
    Upstream { status: u16, body: String },

    #[error("generation failed: {0:?}")]
    OperationFailed(crate::llm::ir::OperationFailure),

    #[error("asset {0} not found")]
    AssetNotFound(i32),

    #[error("asset {id} revision {revision} not found")]
    AssetRevisionNotFound { id: i32, revision: i32 },

    #[error("asset {id} expected kind {expected:?}, found {found:?}")]
    AssetKindMismatch {
        id: i32,
        expected: crate::entities::asset::AssetKind,
        found: crate::entities::asset::AssetKind,
    },

    #[error("asset {id} head moved past expected revision {expected}")]
    AssetRevisionConflict { id: i32, expected: i32 },

    #[error("chunk {0} referenced by manifest is missing")]
    ChunkMissing(String),

    #[error("invalid pipeline definition: {reason}")]
    InvalidPipeline { reason: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Database,
    UpstreamUnavailable,
    InvalidData,
    InvalidRoute,
    ChannelNotFound,
    NoUsableCredential,
    UnsupportedRoute,
    UnsupportedCapability,
    UpstreamRejected,
    AssetNotFound,
    Conflict,
}

#[derive(Clone, Debug, Deserialize, Serialize, ts_rs::TS)]
pub struct ApiError {
    pub code: ErrorCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl CoreError {
    pub fn api_error(&self) -> ApiError {
        use serde_json::json;

        let (code, details) = match self {
            Self::Db(_) => (ErrorCode::Database, None),
            Self::Http(_) | Self::WebSocket(_) => (ErrorCode::UpstreamUnavailable, None),
            Self::Json(_) => (ErrorCode::InvalidData, None),
            Self::Endpoint(_) | Self::Transform(_) | Self::InvalidRoutingRule { .. } => {
                (ErrorCode::InvalidRoute, None)
            }
            Self::ChannelNotFound(id) => (ErrorCode::ChannelNotFound, Some(json!({ "id": id }))),
            Self::NoUsableCredential(channel_id) => (
                ErrorCode::NoUsableCredential,
                Some(json!({ "channel_id": channel_id })),
            ),
            Self::UnsupportedRoute {
                channel_id,
                operation,
            } => (
                ErrorCode::UnsupportedRoute,
                Some(json!({ "channel_id": channel_id, "operation": operation })),
            ),
            Self::UnsupportedRouteImplementation { .. } => (ErrorCode::UnsupportedRoute, None),
            Self::UnsupportedCapability { capability, .. } => (
                ErrorCode::UnsupportedCapability,
                Some(json!({ "capability": capability })),
            ),
            Self::IncompatibleRoute { fields, .. } => (
                ErrorCode::UnsupportedCapability,
                Some(json!({ "fields": fields })),
            ),
            Self::UnmodeledProviderEvent { .. } | Self::InvalidProviderPayload { .. } => {
                (ErrorCode::InvalidData, None)
            }
            Self::Upstream { status, .. } => (
                ErrorCode::UpstreamRejected,
                Some(json!({ "status": status })),
            ),
            Self::OperationFailed(failure) => (
                ErrorCode::UpstreamRejected,
                Some(json!({
                    "operation_code": failure.code,
                    "retryable": failure.retryable,
                })),
            ),
            Self::AssetNotFound(id) => (ErrorCode::AssetNotFound, Some(json!({ "id": id }))),
            Self::AssetRevisionNotFound { id, revision } => (
                ErrorCode::AssetNotFound,
                Some(json!({ "id": id, "revision": revision })),
            ),
            Self::AssetKindMismatch {
                id,
                expected,
                found,
            } => (
                ErrorCode::InvalidData,
                Some(json!({ "id": id, "expected": expected, "found": found })),
            ),
            Self::AssetRevisionConflict { id, expected } => (
                ErrorCode::Conflict,
                Some(json!({ "id": id, "expected": expected })),
            ),
            Self::ChunkMissing(hash) => (ErrorCode::Database, Some(json!({ "chunk": hash }))),
            Self::InvalidPipeline { reason } => {
                (ErrorCode::InvalidData, Some(json!({ "reason": reason })))
            }
        };
        ApiError { code, details }
    }
}
