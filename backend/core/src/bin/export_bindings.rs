use guanfu_core::entities::routing_rule::{RoutingKind, RoutingOperation};
use guanfu_core::error::{ApiError, ErrorCode};
use guanfu_core::services::channels::{ChannelDto, CredentialDto, NewChannel, NewCredential};
use guanfu_core::services::llm::{ChatEvent, CompleteReply, LlmRequestDto};
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
    LlmRequestDto::export_all(&config)?;
    CompleteReply::export_all(&config)?;
    ChatEvent::export_all(&config)?;
    Ok(())
}
