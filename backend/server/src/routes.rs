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
use guanfu_core::services::auth::{Actor, AuthService, Credentials};
use guanfu_core::services::channels::{ChannelService, NewChannel, NewCredential};
use guanfu_core::services::chat::ChatService;
use guanfu_core::services::exchange::ExchangeService;
use guanfu_core::services::llm::{SemanticLlmOutput, SemanticLlmRequest, SemanticStreamMessage};
use guanfu_core::services::media::{MediaInput, MediaService, VideoJobInput};
use guanfu_core::services::routing::{PutRoutingRule, RoutingService};
use guanfu_core::services::runner::{ChatRunRequest, RunnerService};
use guanfu_core::{AppState, CoreError};

pub fn router(state: AppState) -> Router {
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
        .route("/api/assets/{id}/share", axum::routing::put(share_asset))
        .route("/api/users", get(list_users))
        .route(
            "/api/auth/sessions",
            get(list_sessions).delete(revoke_other_sessions),
        )
        .route("/api/auth/sessions/{id}", delete(revoke_session))
        .route("/api/auth/logout", axum::routing::post(logout))
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
    let api = api.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        crate::auth::require_session,
    ));

    // 登录与首次注册在会话之前,必须放在中间件之外。
    let public = Router::new()
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/register", axum::routing::post(register))
        .route("/api/auth/login", axum::routing::post(login))
        // Realtime 走 WebSocket,令牌随首帧校验(见 realtime::session)。
        .route("/api/realtime", get(crate::realtime::handler))
        .with_state(state);

    api.merge(public)
}

#[derive(serde::Serialize)]
struct AuthStatus {
    needs_setup: bool,
}

async fn auth_status(State(state): State<AppState>) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(AuthStatus {
        needs_setup: AuthService::needs_setup(&state.db).await?,
    }))
}

#[derive(serde::Deserialize)]
struct RegisterInput {
    #[serde(flatten)]
    credentials: Credentials,
    /// 引导期的门槛;非回环监听时首次注册必须带(见 auth::guard_public_bind)。
    #[serde(default)]
    bootstrap_token: Option<String>,
    #[serde(default)]
    is_admin: bool,
}

/// 注册。首个账号免令牌(此时还没有账号可登录),之后必须由管理员发起。
///
/// 端点在会话中间件之外(引导期没有会话可用),所以这里自己解析令牌——
/// 否则管理员的身份永远到不了这个 handler,建号会一直被拒。
async fn register(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(input): Json<RegisterInput>,
) -> Result<impl IntoResponse, HttpError> {
    let actor = match presented_token(&headers) {
        Some(token) => Some(AuthService::actor_for(&state.db, &token).await?.0),
        None => None,
    };
    if AuthService::needs_setup(&state.db).await? {
        if let Some(expected) = crate::auth::bootstrap_secret() {
            if input.bootstrap_token.as_deref() != Some(expected.as_str()) {
                return Err(guanfu_core::CoreError::Forbidden {
                    reason: "bootstrap token required for the first account".to_owned(),
                }
                .into());
            }
        }
    }
    Ok((
        StatusCode::CREATED,
        Json(AuthService::register(&state.db, actor, &input.credentials, input.is_admin).await?),
    ))
}

async fn login(
    State(state): State<AppState>,
    Json(input): Json<Credentials>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(AuthService::login(&state.db, &input).await?))
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
    axum::Extension(actor): axum::Extension<Actor>,
    axum::extract::Query(query): axum::extract::Query<AssetQuery>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        AssetService::list(&state.db, actor, query.kind).await?,
    ))
}

#[derive(serde::Deserialize)]
struct ShareInput {
    shared: bool,
}

/// 共享 / 取消共享;只有归属者与管理员能改。
async fn share_asset(
    State(state): State<AppState>,
    axum::Extension(actor): axum::Extension<Actor>,
    Path(id): Path<i32>,
    Json(input): Json<ShareInput>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        AssetService::set_shared(&state.db, actor, id, input.shared).await?,
    ))
}

async fn list_users(State(state): State<AppState>) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(AuthService::list_users(&state.db).await?))
}

async fn list_sessions(
    State(state): State<AppState>,
    axum::Extension(actor): axum::Extension<Actor>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        AuthService::list_sessions(&state.db, actor, presented_token(&headers).as_deref()).await?,
    ))
}

async fn revoke_session(
    State(state): State<AppState>,
    axum::Extension(actor): axum::Extension<Actor>,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    AuthService::revoke_session(&state.db, actor, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 吊销除当前会话外的全部会话——"在其他设备上登出"。
async fn revoke_other_sessions(
    State(state): State<AppState>,
    axum::Extension(actor): axum::Extension<Actor>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, HttpError> {
    let revoked =
        AuthService::revoke_all_sessions(&state.db, actor, presented_token(&headers).as_deref())
            .await?;
    Ok(Json(serde_json::json!({ "revoked": revoked })))
}

async fn logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<StatusCode, HttpError> {
    if let Some(token) = presented_token(&headers) {
        AuthService::logout(&state.db, &token).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

fn presented_token(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned)
}

#[derive(serde::Deserialize)]
struct AssetQuery {
    kind: Option<AssetKind>,
}

async fn delete_asset(
    State(state): State<AppState>,
    axum::Extension(actor): axum::Extension<Actor>,
    Path(id): Path<i32>,
) -> Result<StatusCode, HttpError> {
    AssetService::delete(&state.db, actor, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 角色卡导入:PNG 与 JSON 同一入口,按魔数分辨。
async fn import_character(
    State(state): State<AppState>,
    axum::Extension(actor): axum::Extension<Actor>,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, HttpError> {
    let imported = if body.starts_with(&[0x89, b'P', b'N', b'G']) {
        ExchangeService::import_ccv2_png(&state.db, actor, &body).await?
    } else {
        ExchangeService::import_ccv2_json(&state.db, actor, &body).await?
    };
    Ok((StatusCode::CREATED, Json(imported)))
}

async fn bootstrap_chat(
    State(state): State<AppState>,
    axum::Extension(actor): axum::Extension<Actor>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(ChatService::bootstrap(&state.db, actor).await?))
}

async fn list_histories(
    State(state): State<AppState>,
    axum::Extension(actor): axum::Extension<Actor>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        AssetService::list(&state.db, actor, Some(AssetKind::ChatHistory)).await?,
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
    axum::Extension(actor): axum::Extension<Actor>,
    Json(input): Json<NewHistory>,
) -> Result<impl IntoResponse, HttpError> {
    Ok((
        StatusCode::CREATED,
        Json(ChatService::create_history(&state.db, actor, &input.title, input.bindings).await?),
    ))
}

async fn load_history(
    State(state): State<AppState>,
    axum::Extension(actor): axum::Extension<Actor>,
    Path(id): Path<i32>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(ChatService::load_history(&state.db, actor, id).await?))
}

/// 直接吐字节:图片与音频要能被 <img>/<audio> 直接引用。
async fn media_content(
    State(state): State<AppState>,
    axum::Extension(actor): axum::Extension<Actor>,
    Path(id): Path<i32>,
) -> Result<Response, HttpError> {
    let (media, bytes) =
        AssetService::read_media(&state.db, actor, state.assets.as_ref(), id).await?;
    Ok(([(axum::http::header::CONTENT_TYPE, media.mime_type)], bytes).into_response())
}

async fn generate_image(
    State(state): State<AppState>,
    axum::Extension(actor): axum::Extension<Actor>,
    Json(input): Json<MediaInput<GenerateImageRequest>>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        MediaService::generate_image(
            &state.db,
            actor,
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
    axum::Extension(actor): axum::Extension<Actor>,
    Json(input): Json<MediaInput<EditImageRequest>>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        MediaService::edit_image(
            &state.db,
            actor,
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
    axum::Extension(actor): axum::Extension<Actor>,
    Json(input): Json<MediaInput<SpeechRequest>>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        MediaService::speech(
            &state.db,
            actor,
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
    axum::Extension(actor): axum::Extension<Actor>,
    Json(input): Json<VideoJobInput>,
) -> Result<impl IntoResponse, HttpError> {
    Ok(Json(
        MediaService::download_video(
            &state.db,
            actor,
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
    axum::Extension(actor): axum::Extension<Actor>,
    Path(id): Path<i32>,
    Json(input): Json<ForkHistory>,
) -> Result<impl IntoResponse, HttpError> {
    Ok((
        StatusCode::CREATED,
        Json(
            ChatService::fork_history(&state.db, actor, id, input.message_count, &input.title)
                .await?,
        ),
    ))
}

/// 聊天 run:pipeline 事件以 SSE 推给前端;客户端断开即取消。
async fn run_chat(
    State(state): State<AppState>,
    axum::Extension(actor): axum::Extension<Actor>,
    Json(input): Json<ChatRunRequest>,
) -> Result<Response, HttpError> {
    use futures_util::StreamExt;
    let events = RunnerService::run_chat(
        state.db.clone(),
        actor,
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
