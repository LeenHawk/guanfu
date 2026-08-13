use guanfu_core::error::ApiError;
use guanfu_core::services::channels::{
    ChannelDto, ChannelService, CredentialDto, NewChannel, NewCredential,
};
use guanfu_core::services::routing::{PutRoutingRule, RoutingRuleDto, RoutingService};
use guanfu_core::{AppState, CoreError};
use tauri::State;

fn api_error(error: CoreError) -> ApiError {
    tracing::error!(error = ?error, "core operation failed");
    error.api_error()
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
