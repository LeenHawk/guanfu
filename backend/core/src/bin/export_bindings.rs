use guanfu_core::assets::chat_history::ChatHistoryDefinition;
use guanfu_core::assets::edit::HashEdit;
use guanfu_core::assets::{
    CharacterDefinition, MediaDefinition, OpenAiChatPresetDefinition, PersonaDefinition,
    PipelineDefinition, RegexScriptDefinition, WorldBookDefinition,
};
use guanfu_core::entities::asset::AssetKind;
use guanfu_core::entities::routing_rule::{RoutingKind, RoutingOperation};
use guanfu_core::entities::run::RunStatus;
use guanfu_core::error::{ApiError, ErrorCode};
use guanfu_core::llm::codec::OperationEvent;
use guanfu_core::llm::ir::realtime::{RealtimeClientEvent, RealtimeServerEvent};
use guanfu_core::llm::ir::{OperationRequest, OperationResponse};
use guanfu_core::services::assets::AssetHeadDto;
use guanfu_core::services::channels::{ChannelDto, CredentialDto, NewChannel, NewCredential};
use guanfu_core::services::llm::{SemanticLlmRequest, SemanticStreamMessage};
use guanfu_core::services::routing::{
    OperationKeyDto, PutRoutingRule, RoutingImplementation, RoutingRuleDto,
};
use guanfu_core::services::runs::{ResolvedSlot, SlotBinding};
use ts_rs::{Config, TS};

fn main() -> Result<(), ts_rs::ExportError> {
    let config = Config::from_env();
    RoutingOperation::export_all(&config)?;
    RoutingKind::export_all(&config)?;
    ErrorCode::export_all(&config)?;
    ApiError::export_all(&config)?;
    ChannelDto::export_all(&config)?;
    CredentialDto::export_all(&config)?;
    NewChannel::export_all(&config)?;
    NewCredential::export_all(&config)?;
    OperationKeyDto::export_all(&config)?;
    RoutingImplementation::export_all(&config)?;
    RoutingRuleDto::export_all(&config)?;
    PutRoutingRule::export_all(&config)?;
    SemanticLlmRequest::export_all(&config)?;
    OperationRequest::export_all(&config)?;
    OperationResponse::export_all(&config)?;
    RealtimeClientEvent::export_all(&config)?;
    RealtimeServerEvent::export_all(&config)?;
    OperationEvent::export_all(&config)?;
    SemanticStreamMessage::export_all(&config)?;
    AssetKind::export_all(&config)?;
    AssetHeadDto::export_all(&config)?;
    CharacterDefinition::export_all(&config)?;
    PersonaDefinition::export_all(&config)?;
    WorldBookDefinition::export_all(&config)?;
    OpenAiChatPresetDefinition::export_all(&config)?;
    RegexScriptDefinition::export_all(&config)?;
    PipelineDefinition::export_all(&config)?;
    MediaDefinition::export_all(&config)?;
    ChatHistoryDefinition::export_all(&config)?;
    HashEdit::export_all(&config)?;
    RunStatus::export_all(&config)?;
    SlotBinding::export_all(&config)?;
    ResolvedSlot::export_all(&config)?;
    Ok(())
}
