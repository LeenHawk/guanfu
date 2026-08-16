use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, put};
use axum::{Json, Router};
use guanfu_core::assets::chat_history::SessionBindings;
use guanfu_core::entities::asset::AssetKind;
use guanfu_core::error::ApiError;
use guanfu_core::llm::codec::OperationEvent;
use guanfu_core::llm::ir::audio::{SpeechRequest, TranscriptionRequest};
use guanfu_core::llm::ir::images::{EditImageRequest, GenerateImageRequest};
use guanfu_core::llm::ir::video::CreateVideoRequest;
use guanfu_core::services::assets::AssetService;
use guanfu_core::services::channels::{ChannelService, NewChannel, NewCredential};
use guanfu_core::services::chat::ChatService;
use guanfu_core::services::exchange::ExchangeService;
use guanfu_core::services::llm::{SemanticLlmOutput, SemanticLlmRequest, SemanticStreamMessage};
use guanfu_core::services::media::{MediaInput, MediaService, VideoJobInput};
use guanfu_core::services::routing::{PutRoutingRule, RoutingService};
use guanfu_core::services::runner::{ChatRunRequest, RunnerService};
use guanfu_core::{AppState, CoreError};

pub fn router(state: AppState, token: Option<crate::auth::Token>) -> Router {
    let api = Router::new()
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
        .route("/api/assets", get(list_assets))
        .route("/api/assets/{id}", delete(delete_asset))
        .route(
            "/api/characters/import",
            axum::routing::post(import_character),
        )
        .route("/api/chat/bootstrap", axum::routing::post(bootstrap_chat))
        .route(
            "/api/chat/histories",
            get(list_histories).post(create_history),
        )
        .route("/api/chat/histories/{id}", get(load_history))
        .route(
            "/api/chat/histories/{id}/fork",
            axum::routing::post(fork_history),
        )
        .route("/api/chat/runs", axum::routing::post(run_chat))
        .route("/api/media/{id}/content", get(media_content))
        .route("/api/media/images", axum::routing::post(generate_image))
        .route("/api/media/images/edit", axum::routing::post(edit_image))
        .route("/api/media/speech", axum::routing::post(create_speech))
        .route("/api/media/transcriptions", axum::routing::post(transcribe))
        .route("/api/media/videos", axum::routing::post(create_video))
        .route("/api/media/videos/poll", axum::routing::post(poll_video))
        .route(
            "/api/media/videos/download",
            axum::routing::post(download_video),
        )
        .with_state(state.clone());
    let api = match token.clone() {
        Some(token) => api.layer(axum::middleware::from_fn_with_state(
            token,
            crate::auth::require_token,
        )),
        None => api,
    };
    // Realtime 走 WebSocket,令牌随首帧校验(见 realtime::session)。
    api.merge(
        Router::new()
            .route("/api/realtime", get(crate::realtime::handler))
            .with_state(RealtimeState { state, token }),
    )
}

/// Realtime 端点自带令牌,不经中间件。
#[derive(Clone)]
pub struct RealtimeState {
    pub state: AppState,
    pub token: Option<crate::auth::Token>,
}

async fn execute_llm(
    State(state): State<AppState>,
    Json(input): Json<SemanticLlmRequest>,
) -> Result<Response, HttpError> {
    match state
        .llm
        .execute(&state.db, input.channel_id, input.request)
        .await?
    {
        SemanticLlmOutput::Complete(response) => Ok(Json(response).into_response()),
        SemanticLlmOutput::Stream(stream) => {
            use futures_util::StreamExt;
            let stream = stream.map(|item| {
                let message = match item {
                    Ok(event) => SemanticStreamMessage::Event { event },
                    Err(error) => SemanticStreamMessage::Error {
                        error: error.api_error(),
                    },
                };
                Ok::<_, std::convert::Infallible>(
                    Event::default()
                        .event(event_name(&message))
                        .json_data(message)
                        .expect("semantic stream messages are serializable"),
                )
            });
            Ok(Sse::new(stream).into_response())
        }
        // Realtime 双工需要专用 WebSocket 端点,通用 /api/llm 不承载。
        SemanticLlmOutput::Realtime(_) => {
            Err(guanfu_core::CoreError::UnsupportedRouteImplementation {
                implementation: "realtime over the generic /api/llm endpoint",
            }
            .into())
        }
    }
}

async fn list_assets(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<AssetQuery>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(AssetService::list(&state.db, query.kind).await?))
}

#[derive(serde::Deserialize)]
struct AssetQuery {
    kind: Option<AssetKind>,
}

async fn delete_asset(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> Result<StatusCode, HttpError> {
    AssetService::delete(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 角色卡导入:PNG 与 JSON 同一入口,按魔数分辨。
async fn import_character(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, HttpError> {
    let imported = if body.starts_with(&[0x89, b'P', b'N', b'G']) {
        ExchangeService::import_ccv2_png(&state.db, &body).await?
    } else {
        ExchangeService::import_ccv2_json(&state.db, &body).await?
    };
    Ok((StatusCode::CREATED, Json(imported)))
}

async fn bootstrap_chat(State(state): State<AppState>) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(ChatService::bootstrap(&state.db).await?))
}

async fn list_histories(State(state): State<AppState>) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        AssetService::list(&state.db, Some(AssetKind::ChatHistory)).await?,
    ))
}

#[derive(serde::Deserialize)]
struct NewHistory {
    title: String,
    #[serde(default)]
    bindings: SessionBindings,
}

async fn create_history(
    State(state): State<AppState>,
    Json(input): Json<NewHistory>,
) -> Result<impl IntoResponse, HttpError> {
    Ok((
        StatusCode::CREATED,
        Json(ChatService::create_history(&state.db, &input.title, input.bindings).await?),
    ))
}

async fn load_history(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(ChatService::load_history(&state.db, id).await?))
}

/// 直接吐字节:图片与音频要能被 <img>/<audio> 直接引用。
async fn media_content(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> Result<Response, HttpError> {
    let (media, bytes) = AssetService::read_media(&state.db, state.assets.as_ref(), id).await?;
    Ok(([(axum::http::header::CONTENT_TYPE, media.mime_type)], bytes).into_response())
}

async fn generate_image(
    State(state): State<AppState>,
    Json(input): Json<MediaInput<GenerateImageRequest>>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        MediaService::generate_image(
            &state.db,
            &state.llm,
            &state.assets,
            input.channel_id,
            &input.name,
            input.request,
        )
        .await?,
    ))
}

async fn edit_image(
    State(state): State<AppState>,
    Json(input): Json<MediaInput<EditImageRequest>>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        MediaService::edit_image(
            &state.db,
            &state.llm,
            &state.assets,
            input.channel_id,
            &input.name,
            input.request,
        )
        .await?,
    ))
}

async fn create_speech(
    State(state): State<AppState>,
    Json(input): Json<MediaInput<SpeechRequest>>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        MediaService::speech(
            &state.db,
            &state.llm,
            &state.assets,
            input.channel_id,
            &input.name,
            input.request,
        )
        .await?,
    ))
}

async fn transcribe(
    State(state): State<AppState>,
    Json(input): Json<MediaInput<TranscriptionRequest>>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        MediaService::transcribe(&state.db, &state.llm, input.channel_id, input.request).await?,
    ))
}

async fn create_video(
    State(state): State<AppState>,
    Json(input): Json<MediaInput<CreateVideoRequest>>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        MediaService::create_video(&state.db, &state.llm, input.channel_id, input.request).await?,
    ))
}

async fn poll_video(
    State(state): State<AppState>,
    Json(input): Json<VideoJobInput>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        MediaService::poll_video(&state.db, &state.llm, input.channel_id, input.id).await?,
    ))
}

async fn download_video(
    State(state): State<AppState>,
    Json(input): Json<VideoJobInput>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        MediaService::download_video(
            &state.db,
            &state.llm,
            &state.assets,
            input.channel_id,
            &input.name,
            input.id,
        )
        .await?,
    ))
}

#[derive(serde::Deserialize)]
struct ForkHistory {
    title: String,
    message_count: u32,
}

async fn fork_history(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(input): Json<ForkHistory>,
) -> Result<impl IntoResponse, HttpError> {
    Ok((
        StatusCode::CREATED,
        Json(ChatService::fork_history(&state.db, id, input.message_count, &input.title).await?),
    ))
}

/// 聊天 run:pipeline 事件以 SSE 推给前端;客户端断开即取消。
async fn run_chat(
    State(state): State<AppState>,
    Json(input): Json<ChatRunRequest>,
) -> Result<Response, HttpError> {
    use futures_util::StreamExt;
    let events = RunnerService::run_chat(
        state.db.clone(),
        state.llm.clone(),
        state.assets.clone(),
        input,
    )
    .await?;
    let stream = events.map(|event| {
        Ok::<_, std::convert::Infallible>(
            Event::default()
                .event(pipeline_event_name(&event))
                .json_data(event)
                .expect("pipeline events are serializable"),
        )
    });
    Ok(Sse::new(stream).into_response())
}

fn pipeline_event_name(event: &guanfu_core::services::runner::PipelineEvent) -> &'static str {
    use guanfu_core::services::runner::PipelineEvent;
    match event {
        PipelineEvent::Started { .. } => "started",
        PipelineEvent::Progress { .. } => "progress",
        PipelineEvent::Committed { .. } => "committed",
        PipelineEvent::Failed { .. } => "failed",
    }
}

fn event_name(message: &SemanticStreamMessage) -> &'static str {
    match message {
        SemanticStreamMessage::Event {
            event: OperationEvent::Generate(_),
        } => "generate",
        SemanticStreamMessage::Event {
            event: OperationEvent::Image(_),
        } => "image",
        SemanticStreamMessage::Event {
            event: OperationEvent::Speech(_),
        } => "speech",
        SemanticStreamMessage::Event {
            event: OperationEvent::Transcription(_),
        } => "transcription",
        SemanticStreamMessage::Error { .. } => "error",
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
