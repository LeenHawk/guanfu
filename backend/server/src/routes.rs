use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, put};
use axum::{Json, Router};
use guanfu_core::error::ApiError;
use guanfu_core::services::channels::{ChannelService, NewChannel, NewCredential};
use guanfu_core::services::llm::{ChatEvent, LlmOutput, LlmRequestDto};
use guanfu_core::services::routing::{PutRoutingRule, RoutingService};
use guanfu_core::{AppState, CoreError};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/channels", get(list_channels).post(create_channel))
        .route("/api/channels/{id}", delete(delete_channel))
        .route("/api/channels/{id}/enabled", put(set_channel_enabled))
        .route(
            "/api/channels/{id}/credentials",
            get(list_credentials).post(add_credential),
        )
        .route(
            "/api/channels/{id}/routing-rules",
            get(list_routing_rules).put(put_routing_rule),
        )
        .route("/api/credentials/{id}", delete(remove_credential))
        .route("/api/routing-rules/{id}", delete(remove_routing_rule))
        .route("/api/llm", axum::routing::post(execute_llm))
        .with_state(state)
}

async fn execute_llm(
    State(state): State<AppState>,
    Json(input): Json<LlmRequestDto>,
) -> Result<Response, HttpError> {
    match state.llm.execute(&state.db, input.try_into()?).await? {
        LlmOutput::Complete(reply) => Ok(Json(reply).into_response()),
        LlmOutput::Stream(stream) => {
            use futures_util::StreamExt;
            let stream = stream.map(|item| {
                let event = match item {
                    Ok(event) => event,
                    Err(error) => ChatEvent::Error {
                        error: error.api_error(),
                    },
                };
                Ok::<_, std::convert::Infallible>(
                    Event::default()
                        .event(event_name(&event))
                        .data(serde_json::to_string(&event).expect("ChatEvent is serializable")),
                )
            });
            Ok(Sse::new(stream).into_response())
        }
    }
}

fn event_name(event: &ChatEvent) -> &'static str {
    match event {
        ChatEvent::Frame { .. } => "frame",
        ChatEvent::Done => "done",
        ChatEvent::Error { .. } => "error",
    }
}

struct HttpError(ApiError);

impl From<CoreError> for HttpError {
    fn from(error: CoreError) -> Self {
        tracing::error!(error = ?error, "core operation failed");
        Self(error.api_error())
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self.0)).into_response()
    }
}

async fn list_channels(State(state): State<AppState>) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(ChannelService::list_channels(&state.db).await?))
}

async fn create_channel(
    State(state): State<AppState>,
    Json(input): Json<NewChannel>,
) -> Result<impl IntoResponse, HttpError> {
    Ok((
        StatusCode::CREATED,
        Json(ChannelService::create_channel(&state.db, input).await?),
    ))
}

#[derive(serde::Deserialize)]
struct EnabledInput {
    enabled: bool,
}

async fn set_channel_enabled(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(input): Json<EnabledInput>,
) -> Result<StatusCode, HttpError> {
    ChannelService::set_channel_enabled(&state.db, id, input.enabled).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_channel(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> Result<StatusCode, HttpError> {
    ChannelService::delete_channel(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_credentials(
    State(state): State<AppState>,
    Path(channel_id): Path<i32>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        ChannelService::list_credentials(&state.db, channel_id).await?,
    ))
}

async fn add_credential(
    State(state): State<AppState>,
    Path(channel_id): Path<i32>,
    Json(mut input): Json<NewCredential>,
) -> Result<impl IntoResponse, HttpError> {
    input.channel_id = channel_id;
    Ok((
        StatusCode::CREATED,
        Json(ChannelService::add_credential(&state.db, input).await?),
    ))
}

async fn remove_credential(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> Result<StatusCode, HttpError> {
    ChannelService::remove_credential(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_routing_rules(
    State(state): State<AppState>,
    Path(channel_id): Path<i32>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        RoutingService::list_rules(&state.db, channel_id).await?,
    ))
}

async fn put_routing_rule(
    State(state): State<AppState>,
    Path(channel_id): Path<i32>,
    Json(mut input): Json<PutRoutingRule>,
) -> Result<impl IntoResponse, HttpError> {
    input.channel_id = channel_id;
    Ok(Json(RoutingService::put_rule(&state.db, input).await?))
}

async fn remove_routing_rule(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> Result<StatusCode, HttpError> {
    RoutingService::remove_rule(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
