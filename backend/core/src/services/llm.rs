use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gproxy_protocol::{ContentGenerationKind, Operation, OperationKey, Provider};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::entities::{channel, credential, routing_rule};
use crate::error::ApiError;
use crate::llm::client::{CallTarget, LlmClient};
use crate::llm::codec::{DecodedResponse, OperationEvent, ProviderCodec, SemanticEventStream};
use crate::llm::ir::{OperationRequest, OperationResponse};
use crate::llm::pool::{self, FailureKind};
use crate::llm::routing::{self, RouteDecision};
use crate::llm::wire::WireResult;
use crate::CoreError;

#[derive(Clone, Debug, Deserialize, Serialize, ts_rs::TS)]
pub struct SemanticLlmRequest {
    pub channel_id: i32,
    pub request: OperationRequest,
}

#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SemanticStreamMessage {
    Event { event: OperationEvent },
    Error { error: ApiError },
}

pub enum SemanticLlmOutput {
    Complete(OperationResponse),
    Stream(SemanticEventStream),
    /// Realtime WebSocket 双工连接;由壳层以专用通道暴露,不走通用请求/响应。
    Realtime(crate::llm::realtime::RealtimeConnection),
}

/// Executes provider-neutral model operations through a channel routing table.
pub struct LlmService {
    client: LlmClient,
    rotation: AtomicUsize,
}

impl Default for LlmService {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmService {
    pub fn new() -> Self {
        Self::with_timeouts(Duration::from_secs(10), Duration::from_secs(120))
    }

    pub fn with_timeouts(connect_timeout: Duration, request_timeout: Duration) -> Self {
        Self {
            client: LlmClient::with_timeouts(connect_timeout, request_timeout),
            rotation: AtomicUsize::new(0),
        }
    }

    #[tracing::instrument(skip(self, db, request), fields(operation = ?request.operation()))]
    pub async fn execute(
        &self,
        db: &impl ConnectionTrait,
        channel_id: i32,
        request: OperationRequest,
    ) -> Result<SemanticLlmOutput, CoreError> {
        let channel = channel::Entity::find_by_id(channel_id)
            .one(db)
            .await?
            .filter(|channel| channel.enabled)
            .ok_or(CoreError::ChannelNotFound(channel_id))?;
        let rules = routing_rule::Entity::find()
            .filter(routing_rule::Column::ChannelId.eq(channel.id))
            .all(db)
            .await?;
        let routes = routing::targets_for_operation(&rules, request.operation())?;
        if routes.is_empty() {
            return Err(unsupported_route(channel_id, request.operation()));
        }

        let credentials = credential::Entity::find()
            .filter(credential::Column::ChannelId.eq(channel.id))
            .all(db)
            .await?;
        let rotation = self.rotation.fetch_add(1, Ordering::Relaxed);
        let ordered = pool::order_credentials(&credentials, rotation, OffsetDateTime::now_utc());
        let mut last_error = None;

        // Realtime 是 WebSocket 双工,不进 HTTP codec 通路;
        // 路由矩阵仍是能力开关(要求 OpenAI 直通路由存在)。
        if let OperationRequest::Platform(
            crate::llm::ir::platform::PlatformRequest::ConnectRealtime(connect),
        ) = &request
        {
            if !routes.iter().any(|route| {
                matches!(
                    route,
                    routing::RouteDecision::TransformTo(target)
                        if target.provider_family() == Provider::OpenAi
                )
            }) {
                return Err(unsupported_route(channel_id, request.operation()));
            }
            if ordered.is_empty() {
                return Err(CoreError::NoUsableCredential(channel.id));
            }
            for credential in &ordered {
                let target = CallTarget {
                    base_url: &channel.base_url,
                    secret: &credential.secret,
                };
                match self.client.connect_realtime(&target, connect).await {
                    Ok(connection) => {
                        mark_success(db, credential).await?;
                        return Ok(SemanticLlmOutput::Realtime(connection));
                    }
                    Err(error) => last_error = Some(error),
                }
            }
            return Err(last_error.unwrap_or(CoreError::NoUsableCredential(channel.id)));
        }

        for route in routes {
            let target_key = match route {
                RouteDecision::Local => {
                    return execute_local(&request).map(SemanticLlmOutput::Complete)
                }
                RouteDecision::TransformTo(target) => target,
                RouteDecision::Unsupported => continue,
                RouteDecision::Passthrough => unreachable!("semantic routes resolve passthrough"),
            };
            let wire_request = match ProviderCodec::encode(&request, target_key) {
                Ok(request) => request,
                Err(
                    error @ (CoreError::UnsupportedCapability { .. }
                    | CoreError::IncompatibleRoute { .. }),
                ) => {
                    last_error = Some(error);
                    continue;
                }
                Err(error) => return Err(error),
            };
            if ordered.is_empty() {
                return Err(CoreError::NoUsableCredential(channel.id));
            }

            for credential in &ordered {
                let target = CallTarget {
                    base_url: &channel.base_url,
                    secret: &credential.secret,
                };
                let reply = match self
                    .client
                    .execute_wire(&target, target_key.provider_family(), wire_request.clone())
                    .await
                {
                    Ok(WireResult::Success(reply)) => reply,
                    Ok(WireResult::Rejected(error)) => {
                        let status = error.metadata.status.as_u16();
                        let upstream = CoreError::Upstream {
                            status,
                            body: String::from_utf8_lossy(&error.body).into_owned(),
                        };
                        match pool::classify_status(status) {
                            Some(FailureKind::Fatal) | None => return Err(upstream),
                            Some(kind) => {
                                mark_failure(db, credential, kind).await?;
                                last_error = Some(upstream);
                                continue;
                            }
                        }
                    }
                    Err(error) => {
                        last_error = Some(error);
                        continue;
                    }
                };
                mark_success(db, credential).await?;
                return Ok(match ProviderCodec::decode(&request, target_key, reply)? {
                    DecodedResponse::Complete(response) => SemanticLlmOutput::Complete(response),
                    DecodedResponse::Stream(stream) => SemanticLlmOutput::Stream(stream),
                });
            }
        }

        Err(last_error.unwrap_or_else(|| unsupported_route(channel_id, request.operation())))
    }
}

fn execute_local(request: &OperationRequest) -> Result<OperationResponse, CoreError> {
    let OperationRequest::CountTokens(request) = request else {
        return Err(CoreError::UnsupportedRouteImplementation {
            implementation: "local semantic operation",
        });
    };
    let body = match &request.input {
        crate::llm::ir::tokens::TokenCountInput::Text { values } => {
            serde_json::json!({ "input": values.join("\n") })
        }
        crate::llm::ir::tokens::TokenCountInput::Generation(input) => serde_json::json!({
            "input": input.input,
            "instructions": input.instructions,
            "tools": input.tools,
        }),
    };
    let bytes = serde_json::to_vec(&body)?;
    Ok(OperationResponse::CountTokens(
        crate::llm::ir::tokens::CountTokensResponse {
            input_tokens: crate::llm::count_tokens_local(&request.model.0, &bytes),
        },
    ))
}

fn unsupported_route(channel_id: i32, operation: Operation) -> CoreError {
    let operation = if operation.is_content_generation() {
        OperationKey::content_generation(operation, ContentGenerationKind::OpenAiResponses)
    } else {
        OperationKey::provider(operation, Provider::OpenAi)
    };
    CoreError::UnsupportedRoute {
        channel_id,
        operation,
    }
}

async fn mark_success(
    db: &impl ConnectionTrait,
    credential: &credential::Model,
) -> Result<(), CoreError> {
    let mut active: credential::ActiveModel = credential.clone().into();
    active.failure_count = Set(0);
    active.cooldown_until = Set(None);
    active.last_used_at = Set(Some(OffsetDateTime::now_utc()));
    active.update(db).await?;
    Ok(())
}

async fn mark_failure(
    db: &impl ConnectionTrait,
    credential: &credential::Model,
    kind: FailureKind,
) -> Result<(), CoreError> {
    let mut update = credential::Entity::update_many()
        .col_expr(
            credential::Column::FailureCount,
            Expr::col(credential::Column::FailureCount).add(1),
        )
        .filter(credential::Column::Id.eq(credential.id));
    if let Some(duration) = pool::cooldown_after(kind, credential.failure_count + 1) {
        update = update.col_expr(
            credential::Column::CooldownUntil,
            Expr::value(Some(OffsetDateTime::now_utc() + duration)),
        );
    }
    update.exec(db).await?;
    Ok(())
}
