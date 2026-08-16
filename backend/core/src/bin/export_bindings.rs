use guanfu_core::entities::routing_rule::{RoutingKind, RoutingOperation};
use guanfu_core::error::{ApiError, ErrorCode};
use guanfu_core::llm::codec::OperationEvent;
use guanfu_core::llm::ir::realtime::{RealtimeClientEvent, RealtimeServerEvent};
use guanfu_core::llm::ir::{OperationRequest, OperationResponse};
use guanfu_core::services::channels::{ChannelDto, CredentialDto, NewChannel, NewCredential};
use guanfu_core::services::llm::{SemanticLlmRequest, SemanticStreamMessage};
use guanfu_core::services::routing::{
    OperationKeyDto, PutRoutingRule, RoutingImplementation, RoutingRuleDto,
};
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
    Ok(())
}
