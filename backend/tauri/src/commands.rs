use guanfu_core::assets::chat_history::SessionBindings;
use guanfu_core::entities::asset::AssetKind;
use guanfu_core::error::ApiError;
use guanfu_core::llm::ir::OperationResponse;
use guanfu_core::services::assets::{AssetHeadDto, AssetService};
use guanfu_core::services::channels::{
    ChannelDto, ChannelService, CredentialDto, NewChannel, NewCredential,
};
use guanfu_core::services::chat::{ChatBootstrap, ChatHistoryView, ChatService};
use guanfu_core::services::exchange::{ExchangeService, ImportedCharacter};
use guanfu_core::services::llm::{SemanticLlmOutput, SemanticLlmRequest, SemanticStreamMessage};
use guanfu_core::services::routing::{PutRoutingRule, RoutingRuleDto, RoutingService};
use guanfu_core::services::runner::{ChatRunRequest, PipelineEvent, RunnerService};
use guanfu_core::{AppState, CoreError};
use tauri::{ipc::Channel, State};
use tokio_util::sync::CancellationToken;

use crate::ActiveLlmRequests;

fn api_error(error: CoreError) -> ApiError {
    tracing::error!(error = ?error, "core operation failed");
    error.api_error()
}

fn forget_request(active: &ActiveLlmRequests, request_id: &str) {
    active
        .0
        .lock()
        .expect("active request lock poisoned")
        .remove(request_id);
}

#[tauri::command]
pub async fn execute_llm(
    state: State<'_, AppState>,
    active: State<'_, ActiveLlmRequests>,
    request_id: String,
    input: SemanticLlmRequest,
    on_event: Channel<SemanticStreamMessage>,
) -> Result<Option<OperationResponse>, ApiError> {
    use futures_util::StreamExt;

    let cancellation = CancellationToken::new();
    active
        .0
        .lock()
        .expect("active request lock poisoned")
        .insert(request_id.clone(), cancellation.clone());
    let execute = state
        .llm
        .execute(&state.db, input.channel_id, input.request);
    let outcome = tokio::select! {
        result = execute => Some(result),
        () = cancellation.cancelled() => None,
    };
    let output = match outcome {
        Some(Ok(output)) => output,
        Some(Err(error)) => {
            forget_request(&active, &request_id);
            return Err(api_error(error));
        }
        None => {
            forget_request(&active, &request_id);
            return Ok(None);
        }
    };
    let result = match output {
        SemanticLlmOutput::Complete(response) => Ok(Some(response)),
        SemanticLlmOutput::Stream(mut stream) => {
            loop {
                let item = tokio::select! {
                    item = stream.next() => item,
                    () = cancellation.cancelled() => break,
                };
                let Some(item) = item else { break };
                let message = match item {
                    Ok(event) => SemanticStreamMessage::Event { event },
                    Err(error) => SemanticStreamMessage::Error {
                        error: error.api_error(),
                    },
                };
                if on_event.send(message).is_err() {
                    break;
                }
            }
            Ok(None)
        }
        // Realtime 双工需要专用通道(后续以独立 command + Channel 暴露)。
        SemanticLlmOutput::Realtime(_) => Err(api_error(
            guanfu_core::CoreError::UnsupportedRouteImplementation {
                implementation: "realtime over the generic invoke command",
            },
        )),
    };
    forget_request(&active, &request_id);
    result
}

#[tauri::command]
pub fn cancel_llm(active: State<'_, ActiveLlmRequests>, request_id: String) {
    if let Some(token) = active
        .0
        .lock()
        .expect("active request lock poisoned")
        .remove(&request_id)
    {
        token.cancel();
    }
}

#[tauri::command]
pub async fn create_channel(
    state: State<'_, AppState>,
    input: NewChannel,
) -> Result<ChannelDto, ApiError> {
    ChannelService::create_channel(&state.db, input)
        .await
        .map_err(api_error)
}

#[tauri::command]
pub async fn list_channels(state: State<'_, AppState>) -> Result<Vec<ChannelDto>, ApiError> {
    ChannelService::list_channels(&state.db)
        .await
        .map_err(api_error)
}

#[tauri::command]
pub async fn set_channel_enabled(
    state: State<'_, AppState>,
    id: i32,
    enabled: bool,
) -> Result<(), ApiError> {
    ChannelService::set_channel_enabled(&state.db, id, enabled)
        .await
        .map_err(api_error)
}

#[tauri::command]
pub async fn delete_channel(state: State<'_, AppState>, id: i32) -> Result<(), ApiError> {
    ChannelService::delete_channel(&state.db, id)
        .await
        .map_err(api_error)
}

#[tauri::command]
pub async fn add_credential(
    state: State<'_, AppState>,
    input: NewCredential,
) -> Result<CredentialDto, ApiError> {
    ChannelService::add_credential(&state.db, input)
        .await
        .map_err(api_error)
}

#[tauri::command]
pub async fn list_credentials(
    state: State<'_, AppState>,
    channel_id: i32,
) -> Result<Vec<CredentialDto>, ApiError> {
    ChannelService::list_credentials(&state.db, channel_id)
        .await
        .map_err(api_error)
}

#[tauri::command]
pub async fn remove_credential(state: State<'_, AppState>, id: i32) -> Result<(), ApiError> {
    ChannelService::remove_credential(&state.db, id)
        .await
        .map_err(api_error)
}

#[tauri::command]
pub async fn put_routing_rule(
    state: State<'_, AppState>,
    input: PutRoutingRule,
) -> Result<RoutingRuleDto, ApiError> {
    RoutingService::put_rule(&state.db, input)
        .await
        .map_err(api_error)
}

#[tauri::command]
pub async fn list_routing_rules(
    state: State<'_, AppState>,
    channel_id: i32,
) -> Result<Vec<RoutingRuleDto>, ApiError> {
    RoutingService::list_rules(&state.db, channel_id)
        .await
        .map_err(api_error)
}

#[tauri::command]
pub async fn remove_routing_rule(state: State<'_, AppState>, id: i32) -> Result<(), ApiError> {
    RoutingService::remove_rule(&state.db, id)
        .await
        .map_err(api_error)
}

#[tauri::command]
pub async fn list_assets(
    state: State<'_, AppState>,
    kind: Option<AssetKind>,
) -> Result<Vec<AssetHeadDto>, ApiError> {
    AssetService::list(&state.db, kind).await.map_err(api_error)
}

#[tauri::command]
pub async fn delete_asset(state: State<'_, AppState>, id: i32) -> Result<(), ApiError> {
    AssetService::delete(&state.db, id).await.map_err(api_error)
}

/// 角色卡导入:PNG 与 JSON 同一入口,按魔数分辨。
#[tauri::command]
pub async fn import_character(
    state: State<'_, AppState>,
    bytes: Vec<u8>,
) -> Result<ImportedCharacter, ApiError> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        ExchangeService::import_ccv2_png(&state.db, &bytes).await
    } else {
        ExchangeService::import_ccv2_json(&state.db, &bytes).await
    }
    .map_err(api_error)
}

#[tauri::command]
pub async fn bootstrap_chat(state: State<'_, AppState>) -> Result<ChatBootstrap, ApiError> {
    ChatService::bootstrap(&state.db).await.map_err(api_error)
}

#[tauri::command]
pub async fn create_chat_history(
    state: State<'_, AppState>,
    title: String,
    bindings: SessionBindings,
) -> Result<AssetHeadDto, ApiError> {
    ChatService::create_history(&state.db, &title, bindings)
        .await
        .map_err(api_error)
}

#[tauri::command]
pub async fn load_chat_history(
    state: State<'_, AppState>,
    id: i32,
) -> Result<ChatHistoryView, ApiError> {
    ChatService::load_history(&state.db, id)
        .await
        .map_err(api_error)
}

/// 聊天 run:pipeline 事件走 Channel,取消复用 LLM 的取消表。
#[tauri::command]
pub async fn run_chat(
    state: State<'_, AppState>,
    active: State<'_, ActiveLlmRequests>,
    request_id: String,
    input: ChatRunRequest,
    on_event: Channel<PipelineEvent>,
) -> Result<(), ApiError> {
    use futures_util::StreamExt;

    let cancellation = CancellationToken::new();
    active
        .0
        .lock()
        .expect("active request lock poisoned")
        .insert(request_id.clone(), cancellation.clone());
    let started = RunnerService::run_chat(state.db.clone(), state.llm.clone(), input).await;
    let result = match started {
        Ok(events) => {
            let mut events = std::pin::pin!(events);
            loop {
                let event = tokio::select! {
                    event = events.next() => event,
                    () = cancellation.cancelled() => break,
                };
                let Some(event) = event else { break };
                if on_event.send(event).is_err() {
                    break;
                }
            }
            Ok(())
        }
        Err(error) => Err(api_error(error)),
    };
    forget_request(&active, &request_id);
    result
}
