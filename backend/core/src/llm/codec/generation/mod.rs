mod chat;
mod chat_content;
mod chat_response;
mod chat_stream;
mod chat_tools;
mod claude;
mod claude_response;
mod claude_stream;
mod claude_tools;
mod gemini;
mod gemini_response;
mod gemini_stream;
mod gemini_tools;
mod options;
pub(in crate::llm::codec) mod request;
pub(in crate::llm::codec) mod response;
mod stream;
#[cfg(test)]
mod tests;

use gproxy_protocol::{ContentGenerationKind, Operation, OperationKey, OperationKind};
use http::HeaderMap;
use serde_json::{Map, Value};

use super::{DecodedResponse, OperationEvent};
use crate::llm::ir::generation::{
    GenerateMode, GenerateRequest, InputItem, ReasoningContinuation, ReasoningPart,
};
use crate::llm::wire::{
    JsonBody, JsonSseData, QueryParam, RequestBody, ResponseMode, WireRequest, WireResponse,
};
use crate::CoreError;

pub fn encode(request: &GenerateRequest, target: OperationKey) -> Result<WireRequest, CoreError> {
    let kind = generation_kind(target)?;
    validate_reasoning_continuations(request, target, kind)?;
    request::validate_target_tools(request, target)?;
    // 全部生成目标走 IR ↔ typed wire 直达编码,gproxy-transform 不再参与。
    let value = match kind {
        ContentGenerationKind::ClaudeMessages => claude::encode_request(request, target)?,
        ContentGenerationKind::GeminiGenerateContent => gemini::encode_request(request, target)?,
        ContentGenerationKind::OpenAiChatCompletions => chat::encode_request(request, target)?,
        ContentGenerationKind::OpenAiResponses => {
            let mut value = request::encode_request(request)?;
            options::apply(request, target, &mut value)?;
            value
        }
        // guanfu 的生成客户端走 HTTP,WS 帧无法在此通路传输;realtime 另有专用通路。
        ContentGenerationKind::OpenAiResponsesWebSocket => {
            return Err(CoreError::UnsupportedRouteImplementation {
                implementation: "websocket generation transport",
            })
        }
        _ => return Err(unsupported_kind(target)),
    };
    let body = JsonBody::encode(&value)?;
    let endpoint = gproxy_protocol::endpoint::request_target(
        target,
        &request.model.0,
        request.mode == GenerateMode::Stream,
    )
    .map_err(|error| CoreError::Endpoint(error.to_string()))?;
    Ok(WireRequest {
        method: endpoint.method.into(),
        path: endpoint.path,
        query: endpoint
            .query
            .as_deref()
            .map(parse_query)
            .transpose()?
            .unwrap_or_default(),
        headers: HeaderMap::new(),
        body: RequestBody::Json(body),
        response_mode: match request.mode {
            GenerateMode::Complete => ResponseMode::Json,
            GenerateMode::Stream => ResponseMode::JsonSse,
        },
    })
}

fn validate_reasoning_continuations(
    request: &GenerateRequest,
    target: OperationKey,
    kind: ContentGenerationKind,
) -> Result<(), CoreError> {
    let compatible = request.input.iter().all(|item| {
        let InputItem::Reasoning { reasoning } = item else {
            return true;
        };
        reasoning.previous.parts.iter().all(|part| {
            let continuation = match part {
                ReasoningPart::Text {
                    continuation: Some(continuation),
                    ..
                }
                | ReasoningPart::Opaque { continuation } => continuation,
                _ => return true,
            };
            continuation_matches(continuation, kind)
        })
    });
    if compatible {
        Ok(())
    } else {
        Err(CoreError::IncompatibleRoute {
            target,
            fields: vec!["input.reasoning.continuation".into()],
        })
    }
}

fn continuation_matches(continuation: &ReasoningContinuation, kind: ContentGenerationKind) -> bool {
    match continuation {
        ReasoningContinuation::OpenAiEncrypted { .. } => matches!(
            kind,
            ContentGenerationKind::OpenAiResponses
                | ContentGenerationKind::OpenAiResponsesWebSocket
        ),
        ReasoningContinuation::ClaudeSignature { .. }
        | ReasoningContinuation::ClaudeRedacted { .. } => {
            kind == ContentGenerationKind::ClaudeMessages
        }
        ReasoningContinuation::GeminiThoughtSignature { .. } => {
            kind == ContentGenerationKind::GeminiGenerateContent
        }
    }
}

pub fn decode(
    request: &GenerateRequest,
    target: OperationKey,
    response: WireResponse,
) -> Result<DecodedResponse, CoreError> {
    let kind = generation_kind(target)?;
    match response {
        WireResponse::Json(response) if request.mode == GenerateMode::Complete => {
            let decoded = match kind {
                ContentGenerationKind::ClaudeMessages => {
                    claude_response::decode_complete(&response.body, target)?
                }
                ContentGenerationKind::GeminiGenerateContent => {
                    gemini_response::decode_complete(&response.body)?
                }
                ContentGenerationKind::OpenAiChatCompletions => {
                    chat_response::decode_complete(&response.body, target)?
                }
                ContentGenerationKind::OpenAiResponses => {
                    response::decode_complete(&response.body, target)?
                }
                ContentGenerationKind::OpenAiResponsesWebSocket => {
                    return Err(CoreError::UnsupportedRouteImplementation {
                        implementation: "websocket generation transport",
                    })
                }
                _ => return Err(unsupported_kind(target)),
            };
            Ok(DecodedResponse::Complete(
                crate::llm::ir::OperationResponse::Generate(decoded),
            ))
        }
        WireResponse::JsonSse(response) if request.mode == GenerateMode::Stream => match kind {
            ContentGenerationKind::ClaudeMessages => {
                let mut decoder = claude_stream::StreamDecoder::new(target);
                Ok(DecodedResponse::Stream(super::map_sse(
                    response.stream,
                    move |frame| decoder.push(frame),
                )))
            }
            ContentGenerationKind::GeminiGenerateContent => {
                let mut decoder = gemini_stream::StreamDecoder::default();
                Ok(DecodedResponse::Stream(super::map_sse(
                    response.stream,
                    move |frame| decoder.push(frame),
                )))
            }
            ContentGenerationKind::OpenAiChatCompletions => {
                let mut decoder = chat_stream::StreamDecoder::new(target);
                Ok(DecodedResponse::Stream(super::map_sse(
                    response.stream,
                    move |frame| decoder.push(frame),
                )))
            }
            ContentGenerationKind::OpenAiResponses => {
                let mut decoder = stream::StreamDecoder::new(target);
                Ok(DecodedResponse::Stream(super::map_sse(
                    response.stream,
                    move |frame| decoder.push(frame),
                )))
            }
            ContentGenerationKind::OpenAiResponsesWebSocket => {
                Err(CoreError::UnsupportedRouteImplementation {
                    implementation: "websocket generation transport",
                })
            }
            _ => Err(unsupported_kind(target)),
        },
        _ => Err(CoreError::Endpoint(
            "generation response mode does not match request".to_owned(),
        )),
    }
}

fn generation_kind(target: OperationKey) -> Result<ContentGenerationKind, CoreError> {
    match target.kind() {
        OperationKind::ContentGeneration(kind) => Ok(kind),
        _ => Err(CoreError::InvalidProviderPayload {
            target,
            reason: "target is not a content generation kind".into(),
        }),
    }
}

fn unsupported_kind(target: OperationKey) -> CoreError {
    CoreError::InvalidProviderPayload {
        target,
        reason: "unsupported content generation kind".into(),
    }
}

fn u32_field(value: &Value, key: &str) -> Result<u32, CoreError> {
    u32::try_from(u64_field(value, key)?)
        .map_err(|_| invalid_payload(&format!("{key} exceeds u32")))
}
fn u64_field(value: &Value, key: &str) -> Result<u64, CoreError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_payload(&format!("missing or invalid integer {key}")))
}
fn string_field(value: &Value, key: &str) -> Result<String, CoreError> {
    Ok(required_str(value, key)?.into())
}
fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, CoreError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_payload(&format!("missing or invalid string {key}")))
}
fn pointer_str<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, CoreError> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_payload(&format!("missing or invalid string {pointer}")))
}
/// Responses codec(request.rs/response.rs/stream.rs)共用的错误构造。
fn invalid_payload(reason: &str) -> CoreError {
    CoreError::InvalidProviderPayload {
        target: OperationKey::content_generation(
            Operation::GenerateContent,
            ContentGenerationKind::OpenAiResponses,
        ),
        reason: reason.to_owned(),
    }
}
fn kind_key(value: Option<&str>) -> &str {
    value.unwrap_or("missing")
}
fn unmodeled(event: &str, operation: Operation) -> CoreError {
    CoreError::UnmodeledProviderEvent {
        target: OperationKey::content_generation(operation, ContentGenerationKind::OpenAiResponses),
        event: event.into(),
    }
}
fn parse_query(query: &str) -> Result<Vec<QueryParam>, CoreError> {
    query
        .split('&')
        .map(|part| {
            let (name, value) = part.split_once('=').unwrap_or((part, ""));
            Ok(QueryParam {
                name: name.into(),
                value: value.into(),
            })
        })
        .collect()
}
fn insert_option<T: serde::Serialize>(map: &mut Map<String, Value>, key: &str, value: Option<T>) {
    if let Some(value) = value {
        map.insert(
            key.into(),
            serde_json::to_value(value).expect("scalar is serializable"),
        );
    }
}
fn snake(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .expect("IR enums are serializable")
        .as_str()
        .expect("IR enums serialize as strings")
        .to_owned()
}
