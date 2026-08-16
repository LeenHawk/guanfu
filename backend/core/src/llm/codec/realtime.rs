//! Realtime 语义模型与 OpenAI GA wire 的映射(纯函数,无传输)。

use gproxy_protocol::openai as wire;
use serde_json::Value;

use crate::llm::ir::generation::{
    InputContent, InputItem, Instruction, MessageRole, ToolOutcome, ToolResultContent,
};
use crate::llm::ir::realtime::*;
use crate::llm::ir::{MediaSource, OperationFailure, Usage};
use crate::CoreError;

pub(crate) fn encode_session(
    session: &RealtimeSession,
) -> Result<wire::RealtimeSessionConfig, CoreError> {
    let tools = session
        .tools
        .iter()
        .map(|tool| {
            serde_json::from_value(super::generation::request::encode_tool(tool)?)
                .map_err(CoreError::from)
        })
        .collect::<Result<Vec<wire::ResponseTool>, _>>()?;
    let tool_choice = serde_json::from_value(super::generation::request::encode_tool_choice(
        &session.tool_choice,
        &session.tools,
    ))?;
    let input = wire::RealtimeAudioInputConfig::builder()
        .format(session.input_audio_format.as_ref().map(encode_format))
        .noise_reduction(
            session
                .noise_reduction
                .map(encode_noise_reduction)
                .transpose()?,
        )
        .transcription(
            session
                .input_transcription
                .as_ref()
                .map(encode_transcription)
                .transpose()?,
        )
        .turn_detection(session.turn_detection.as_ref().map(encode_turn_detection))
        .build()
        .map_err(wire_build_error)?;
    let output = wire::RealtimeAudioOutputConfig::builder()
        .format(session.output_audio_format.as_ref().map(encode_format))
        .voice(session.voice.clone())
        .speed(session.speed)
        .build()
        .map_err(wire_build_error)?;
    let audio = wire::RealtimeAudioConfig::builder()
        .input(Some(input))
        .output(Some(output))
        .build()
        .map_err(wire_build_error)?;
    wire::RealtimeSessionConfig::builder()
        .type_(Some(wire::RealtimeSessionType::Known(
            wire::RealtimeSessionTypeKnown::Realtime,
        )))
        .model(Some(wire::OpenAiModelId::Unknown(session.model.0.clone())))
        .output_modalities(Some(
            session
                .modalities
                .iter()
                .map(|modality| {
                    wire::RealtimeOutputModality::Known(match modality {
                        RealtimeModality::Text => wire::RealtimeOutputModalityKnown::Text,
                        RealtimeModality::Audio => wire::RealtimeOutputModalityKnown::Audio,
                    })
                })
                .collect(),
        ))
        .instructions(encode_instructions(&session.instructions))
        .audio(Some(audio))
        .tools((!tools.is_empty()).then_some(tools))
        .tool_choice(Some(tool_choice))
        .max_output_tokens(
            session
                .max_output_tokens
                .map(wire::RealtimeMaxTokens::Count),
        )
        .build()
        .map_err(wire_build_error)
}

fn wire_build_error(error: gproxy_protocol::WireBuildError) -> CoreError {
    CoreError::Transform(error.to_string())
}

fn encode_noise_reduction(mode: NoiseReductionMode) -> Result<wire::NoiseReduction, CoreError> {
    wire::NoiseReduction::builder()
        .type_(Some(wire::NoiseReductionType::Known(match mode {
            NoiseReductionMode::NearField => wire::NoiseReductionTypeKnown::NearField,
            NoiseReductionMode::FarField => wire::NoiseReductionTypeKnown::FarField,
        })))
        .build()
        .map_err(wire_build_error)
}

fn encode_transcription(
    config: &RealtimeTranscription,
) -> Result<wire::RealtimeTranscriptionConfig, CoreError> {
    wire::RealtimeTranscriptionConfig::builder()
        .model(config.model.clone())
        .language(config.language.clone())
        .prompt(config.prompt.clone())
        .build()
        .map_err(wire_build_error)
}

fn encode_instructions(instructions: &[Instruction]) -> Option<String> {
    let text = instructions
        .iter()
        .flat_map(|instruction| &instruction.content)
        .filter_map(|part| match part {
            InputContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.is_empty()).then_some(text)
}

fn encode_format(format: &RealtimeAudioFormat) -> wire::RealtimeAudioFormat {
    match format {
        RealtimeAudioFormat::Pcm16 { rate } => wire::RealtimeAudioFormat::Pcm {
            rate: *rate,
            extra: Default::default(),
        },
        RealtimeAudioFormat::G711Ulaw => wire::RealtimeAudioFormat::Pcmu {
            extra: Default::default(),
        },
        RealtimeAudioFormat::G711Alaw => wire::RealtimeAudioFormat::Pcma {
            extra: Default::default(),
        },
    }
}

fn encode_turn_detection(setting: &TurnDetection) -> wire::RealtimeTurnDetectionSetting {
    match setting {
        TurnDetection::Off => wire::RealtimeTurnDetectionSetting::Disabled,
        TurnDetection::ServerVad {
            threshold,
            prefix_padding_ms,
            silence_duration_ms,
            idle_timeout_ms,
            create_response,
            interrupt_response,
        } => wire::RealtimeTurnDetectionSetting::Vad(wire::RealtimeTurnDetection::ServerVad {
            threshold: *threshold,
            prefix_padding_ms: *prefix_padding_ms,
            silence_duration_ms: *silence_duration_ms,
            idle_timeout_ms: *idle_timeout_ms,
            create_response: *create_response,
            interrupt_response: *interrupt_response,
            extra: Default::default(),
        }),
        TurnDetection::SemanticVad {
            eagerness,
            create_response,
            interrupt_response,
        } => wire::RealtimeTurnDetectionSetting::Vad(wire::RealtimeTurnDetection::SemanticVad {
            eagerness: eagerness.map(|eagerness| {
                wire::SemanticVadEagerness::Known(match eagerness {
                    SemanticVadEagerness::Low => wire::SemanticVadEagernessKnown::Low,
                    SemanticVadEagerness::Medium => wire::SemanticVadEagernessKnown::Medium,
                    SemanticVadEagerness::High => wire::SemanticVadEagernessKnown::High,
                    SemanticVadEagerness::Auto => wire::SemanticVadEagernessKnown::Auto,
                })
            }),
            create_response: *create_response,
            interrupt_response: *interrupt_response,
            extra: Default::default(),
        }),
    }
}

pub(crate) fn encode_client_event(
    event: &RealtimeClientEvent,
) -> Result<wire::RealtimeClientEvent, CoreError> {
    Ok(match event {
        RealtimeClientEvent::UpdateSession { session } => {
            wire::RealtimeClientEvent::SessionUpdate {
                session: encode_session(session)?,
                event_id: None,
                extra: Default::default(),
            }
        }
        RealtimeClientEvent::AppendAudio { audio } => {
            wire::RealtimeClientEvent::InputAudioBufferAppend {
                audio: audio.clone(),
                event_id: None,
                extra: Default::default(),
            }
        }
        RealtimeClientEvent::CommitAudio => wire::RealtimeClientEvent::InputAudioBufferCommit {
            event_id: None,
            extra: Default::default(),
        },
        RealtimeClientEvent::ClearAudio => wire::RealtimeClientEvent::InputAudioBufferClear {
            event_id: None,
            extra: Default::default(),
        },
        RealtimeClientEvent::CreateItem { item } => {
            wire::RealtimeClientEvent::ConversationItemCreate {
                item: encode_item(item)?,
                previous_item_id: None,
                event_id: None,
                extra: Default::default(),
            }
        }
        RealtimeClientEvent::TruncatePlayback {
            item_id,
            audio_end_ms,
        } => wire::RealtimeClientEvent::ConversationItemTruncate {
            item_id: item_id.clone(),
            content_index: 0,
            audio_end_ms: *audio_end_ms,
            event_id: None,
            extra: Default::default(),
        },
        RealtimeClientEvent::CreateResponse => wire::RealtimeClientEvent::ResponseCreate {
            response: None,
            event_id: None,
            extra: Default::default(),
        },
        RealtimeClientEvent::CancelResponse => wire::RealtimeClientEvent::ResponseCancel {
            response_id: None,
            event_id: None,
            extra: Default::default(),
        },
    })
}

fn encode_item(item: &InputItem) -> Result<wire::RealtimeItem, CoreError> {
    let unsupported = |what: &'static str| CoreError::UnsupportedRouteImplementation {
        implementation: what,
    };
    Ok(wire::RealtimeItem::Known(match item {
        InputItem::Message { message } => wire::KnownRealtimeItem::Message {
            id: None,
            role: wire::RealtimeRole::Known(match message.role {
                MessageRole::System => wire::RealtimeRoleKnown::System,
                MessageRole::User => wire::RealtimeRoleKnown::User,
                MessageRole::Assistant => wire::RealtimeRoleKnown::Assistant,
            }),
            content: message
                .content
                .iter()
                .map(|part| encode_content(part, message.role))
                .collect::<Result<_, _>>()?,
            status: None,
            extra: Default::default(),
        },
        InputItem::ToolResult { result } => wire::KnownRealtimeItem::FunctionCallOutput {
            id: None,
            call_id: result.call_id.0.clone(),
            output: match &result.outcome {
                ToolOutcome::Success { content } => content
                    .iter()
                    .map(|part| match part {
                        ToolResultContent::Text { text } => Ok(text.clone()),
                        ToolResultContent::Json { value } => Ok(value.to_string()),
                        ToolResultContent::Image { .. } => {
                            Err(unsupported("realtime tool result image"))
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .join("\n"),
                ToolOutcome::Error { code, message } => serde_json::json!({
                    "error": {"code": code, "message": message}
                })
                .to_string(),
            },
            status: None,
            extra: Default::default(),
        },
        InputItem::McpApproval { .. } => return Err(unsupported("realtime mcp approval")),
        InputItem::Reasoning { .. } => return Err(unsupported("realtime reasoning replay")),
    }))
}

fn encode_content(
    part: &InputContent,
    role: MessageRole,
) -> Result<wire::RealtimeContentPart, CoreError> {
    Ok(match part {
        InputContent::Text { text } => match role {
            MessageRole::Assistant => wire::RealtimeContentPart::Text {
                text: Some(text.clone()),
                extra: Default::default(),
            },
            _ => wire::RealtimeContentPart::InputText {
                text: Some(text.clone()),
                extra: Default::default(),
            },
        },
        InputContent::Audio {
            source: MediaSource::Data { bytes, .. },
        } => wire::RealtimeContentPart::InputAudio {
            audio: Some(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                bytes,
            )),
            transcript: None,
            extra: Default::default(),
        },
        _ => {
            return Err(CoreError::UnsupportedRouteImplementation {
                implementation: "realtime item content requires text or inline audio",
            })
        }
    })
}

/// 解码服务端事件;`None` 表示有意忽略(item 生命周期细节、限流等)。
pub(crate) fn decode_server_event(event: wire::RealtimeServerEvent) -> Option<RealtimeServerEvent> {
    use wire::KnownRealtimeServerEvent as E;
    let wire::RealtimeServerEvent::Known(event) = event else {
        return None;
    };
    Some(match *event {
        E::SessionCreated { .. } => RealtimeServerEvent::SessionCreated,
        E::SessionUpdated { .. } => RealtimeServerEvent::SessionUpdated,
        E::InputAudioBufferSpeechStarted { item_id, .. } => {
            RealtimeServerEvent::InputSpeechStarted { item_id }
        }
        E::InputAudioBufferSpeechStopped { item_id, .. } => {
            RealtimeServerEvent::InputSpeechStopped { item_id }
        }
        E::InputAudioBufferCommitted { item_id, .. } => {
            RealtimeServerEvent::InputAudioCommitted { item_id }
        }
        E::InputAudioTranscriptionDelta { item_id, delta, .. } => {
            RealtimeServerEvent::InputTranscriptDelta { item_id, delta }
        }
        E::InputAudioTranscriptionCompleted {
            item_id,
            transcript,
            ..
        } => RealtimeServerEvent::InputTranscriptCompleted {
            item_id,
            transcript,
        },
        E::InputAudioTranscriptionFailed { item_id, error, .. } => {
            RealtimeServerEvent::InputTranscriptFailed {
                item_id,
                error: decode_error(error),
            }
        }
        E::ResponseCreated { response, .. } => RealtimeServerEvent::ResponseStarted {
            response_id: response.id.unwrap_or_default(),
        },
        E::OutputAudioDelta { item_id, delta, .. } => {
            RealtimeServerEvent::AudioDelta { item_id, delta }
        }
        E::OutputAudioDone { item_id, .. } => RealtimeServerEvent::AudioDone { item_id },
        E::OutputAudioTranscriptDelta { item_id, delta, .. } => {
            RealtimeServerEvent::OutputTranscriptDelta { item_id, delta }
        }
        E::OutputAudioTranscriptDone {
            item_id,
            transcript,
            ..
        } => RealtimeServerEvent::OutputTranscriptDone {
            item_id,
            transcript,
        },
        E::OutputTextDelta { item_id, delta, .. } => {
            RealtimeServerEvent::TextDelta { item_id, delta }
        }
        E::OutputTextDone { item_id, text, .. } => RealtimeServerEvent::TextDone { item_id, text },
        E::FunctionCallArgumentsDone {
            item_id,
            call_id,
            arguments,
            name,
            ..
        } => RealtimeServerEvent::ToolCall {
            call: crate::llm::ir::generation::FunctionCall {
                id: crate::llm::ir::OutputId(item_id),
                call_id: crate::llm::ir::ToolCallId(call_id),
                name: name.unwrap_or_default(),
                arguments: serde_json::from_str(&arguments).unwrap_or(Value::String(arguments)),
            },
        },
        E::ResponseDone { response, .. } => RealtimeServerEvent::ResponseFinished {
            response_id: response.id.clone().unwrap_or_default(),
            status: match response.status {
                Some(wire::RealtimeResponseStatus::Known(
                    wire::RealtimeResponseStatusKnown::Completed,
                )) => RealtimeFinish::Completed,
                Some(wire::RealtimeResponseStatus::Known(
                    wire::RealtimeResponseStatusKnown::Cancelled,
                )) => RealtimeFinish::Cancelled,
                Some(wire::RealtimeResponseStatus::Known(
                    wire::RealtimeResponseStatusKnown::Failed,
                )) => RealtimeFinish::Failed,
                _ => RealtimeFinish::Incomplete,
            },
            usage: response.usage.map(decode_usage),
        },
        E::Error { error, .. } => RealtimeServerEvent::Error {
            error: decode_error(error),
        },
        _ => return None,
    })
}

fn decode_error(error: wire::RealtimeError) -> OperationFailure {
    OperationFailure {
        code: error
            .code
            .or(error.type_)
            .unwrap_or_else(|| "realtime_error".to_owned()),
        message: error.message.unwrap_or_else(|| "realtime error".to_owned()),
        retryable: false,
        details: Default::default(),
    }
}

fn decode_usage(usage: wire::RealtimeUsage) -> Usage {
    let input_tokens = usage.input_tokens.unwrap_or_default();
    let output_tokens = usage.output_tokens.unwrap_or_default();
    Usage {
        input_tokens,
        output_tokens,
        cached_input_tokens: usage
            .input_token_details
            .and_then(|details| details.cached_tokens)
            .unwrap_or_default(),
        reasoning_tokens: 0,
        total_tokens: usage.total_tokens.unwrap_or(input_tokens + output_tokens),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::llm::ir::generation::ToolChoice;
    use crate::llm::ir::ModelId;

    #[test]
    fn encodes_session_and_decodes_server_events() {
        let session = RealtimeSession {
            model: ModelId("gpt-realtime".into()),
            instructions: vec![Instruction {
                role: crate::llm::ir::generation::InstructionRole::System,
                content: vec![InputContent::Text {
                    text: "扮演角色".into(),
                }],
            }],
            modalities: vec![RealtimeModality::Audio],
            voice: Some("marin".into()),
            speed: None,
            input_audio_format: Some(RealtimeAudioFormat::Pcm16 { rate: Some(24000) }),
            output_audio_format: None,
            input_transcription: None,
            noise_reduction: None,
            turn_detection: Some(TurnDetection::SemanticVad {
                eagerness: Some(SemanticVadEagerness::Low),
                create_response: None,
                interrupt_response: None,
            }),
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            max_output_tokens: None,
        };
        let event = encode_client_event(&RealtimeClientEvent::UpdateSession {
            session: Box::new(session),
        })
        .unwrap();
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], "session.update");
        assert_eq!(value["session"]["model"], "gpt-realtime");
        assert_eq!(value["session"]["instructions"], "扮演角色");
        assert_eq!(value["session"]["audio"]["input"]["format"]["rate"], 24000);
        assert_eq!(
            value["session"]["audio"]["input"]["turn_detection"]["eagerness"],
            "low"
        );
        assert_eq!(value["session"]["audio"]["output"]["voice"], "marin");

        let audio: wire::RealtimeServerEvent = serde_json::from_value(json!({
            "type":"response.output_audio.delta",
            "response_id":"resp_1","item_id":"item_1",
            "output_index":0,"content_index":0,"delta":"b64"
        }))
        .unwrap();
        assert!(matches!(
            decode_server_event(audio),
            Some(RealtimeServerEvent::AudioDelta { item_id, delta })
                if item_id == "item_1" && delta == "b64"
        ));

        let ignored: wire::RealtimeServerEvent = serde_json::from_value(json!({
            "type":"rate_limits.updated","rate_limits":[]
        }))
        .unwrap();
        assert!(decode_server_event(ignored).is_none());
    }
}
