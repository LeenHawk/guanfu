mod request;
mod response;
mod stream;
#[cfg(test)]
mod tests;

use base64::Engine;
use bytes::Bytes;
use gproxy_protocol::{Operation, OperationKey, Provider};
use gproxy_transform::{dispatch, resolve, TransformContext};
use http::HeaderMap;
use serde_json::{json, Value};

use super::DecodedResponse;
use crate::llm::ir::audio::*;
use crate::llm::ir::embeddings::*;
use crate::llm::ir::images::*;
use crate::llm::ir::models::*;
use crate::llm::ir::platform::*;
use crate::llm::ir::search::*;
use crate::llm::ir::tokens::*;
use crate::llm::ir::video::*;
use crate::llm::ir::{Capability, MediaSource, ModelId, OperationRequest, OperationResponse};
use crate::llm::wire::{
    JsonBody, MultipartBody, MultipartPart, MultipartValue, QueryParam, RequestBody, ResponseMode,
    WireRequest, WireResponse,
};
use crate::CoreError;

pub fn encode(request: &OperationRequest, target: OperationKey) -> Result<WireRequest, CoreError> {
    if let OperationRequest::Embeddings(request) = request {
        if request.task.is_some() && target.provider_family() != Provider::Gemini {
            return Err(unsupported(Capability::Embeddings, target));
        }
    }
    let (canonical, body, query, response_mode) = request::canonical_request(request)?;
    let endpoint = gproxy_protocol::endpoint::request_target(
        target,
        request.model_id().unwrap_or_default(),
        is_stream(request),
    )
    .map_err(|error| CoreError::Endpoint(error.to_string()))?;
    let (body, mut query) = match request {
        OperationRequest::Embeddings(request) if target.provider_family() == Provider::Gemini => (
            request::encode_gemini_embedding(request, target)?,
            Vec::new(),
        ),
        _ => transform_request(canonical, target, body, query)?,
    };
    if let OperationRequest::Models(ModelRequest::List(request)) = request {
        query = model_query(request, target.provider_family());
    }
    Ok(WireRequest {
        method: endpoint.method.into(),
        path: endpoint.path,
        query: endpoint
            .query
            .as_deref()
            .map(parse_query)
            .transpose()?
            .unwrap_or(query),
        headers: HeaderMap::new(),
        body: request::prepare_body(request, target, body)?,
        response_mode,
    })
}

pub fn decode(
    request: &OperationRequest,
    target: OperationKey,
    response: WireResponse,
) -> Result<DecodedResponse, CoreError> {
    let canonical = request::canonical_key(request)?;
    match (request, response) {
        (
            OperationRequest::Audio(AudioRequest::Speech(request)),
            WireResponse::Binary(response),
        ) => Ok(DecodedResponse::Complete(OperationResponse::Audio(
            AudioResponse::Speech(SpeechArtifact {
                media_type: response
                    .content_type
                    .unwrap_or_else(|| request.format.media_type().to_owned()),
                bytes: response.body,
            }),
        ))),
        (
            OperationRequest::Audio(AudioRequest::Speech(request)),
            WireResponse::BinaryStream(response),
        ) => stream::decode_speech_stream(request, response),
        (OperationRequest::Audio(AudioRequest::Speech(_)), _) => Err(mode_error()),
        (
            OperationRequest::Video(VideoRequest::DownloadContent(_)),
            WireResponse::Binary(response),
        ) => Ok(DecodedResponse::Complete(OperationResponse::Video(
            VideoResponse::Content(VideoContent {
                media_type: response.content_type,
                bytes: response.body,
            }),
        ))),
        (
            OperationRequest::Platform(PlatformRequest::CreateRealtimeCall(_)),
            WireResponse::Binary(response),
        ) => Ok(DecodedResponse::Complete(OperationResponse::Platform(
            PlatformResponse::RealtimeCall(RealtimeCall {
                id: response
                    .metadata
                    .headers
                    .get(http::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|path| path.rsplit('/').next())
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned),
                answer_sdp: String::from_utf8_lossy(&response.body).into_owned(),
            }),
        ))),
        (_, WireResponse::Json(response)) => {
            let body = transform_response(
                target,
                canonical,
                response.body,
                request_capability(request.operation()),
            )?;
            Ok(DecodedResponse::Complete(response::decode_json(
                request, target, &body,
            )?))
        }
        (OperationRequest::Images(image_request), WireResponse::JsonSse(response)) => {
            stream::decode_image_stream(image_request, target, response.stream)
        }
        (OperationRequest::Audio(AudioRequest::Transcribe(_)), WireResponse::JsonSse(response)) => {
            stream::decode_transcription_stream(target, response.stream)
        }
        _ => Err(mode_error()),
    }
}

fn transform_request(
    source: OperationKey,
    target: OperationKey,
    body: RequestBody,
    query: Vec<QueryParam>,
) -> Result<(RequestBody, Vec<QueryParam>), CoreError> {
    if source == target {
        return Ok((body, query));
    }
    let pair = resolve(source, target).map_err(transform_error)?;
    let query_text = (!query.is_empty()).then(|| {
        query
            .iter()
            .map(|p| format!("{}={}", p.name, p.value))
            .collect::<Vec<_>>()
            .join("&")
    });
    let ctx = TransformContext::new(source, target).with_request("", query_text.as_deref());
    let body = match body {
        RequestBody::Json(body) => {
            let output = dispatch::request_bytes_detailed(pair, &ctx, body.as_bytes())
                .map_err(transform_error)?;
            strict(
                output.diagnostics,
                target,
                request_capability(source.operation()),
            )?;
            RequestBody::Json(JsonBody::from_bytes(Bytes::from(output.value))?)
        }
        RequestBody::Empty => RequestBody::Empty,
        RequestBody::Multipart(_) => {
            return Err(CoreError::Endpoint(
                "cannot transform prepared multipart body".into(),
            ))
        }
    };
    let query = gproxy_transform::models::list::query::request_query(pair, &ctx)
        .as_deref()
        .map(parse_query)
        .transpose()?
        .unwrap_or_default();
    Ok((body, query))
}

fn transform_response(
    source: OperationKey,
    target: OperationKey,
    body: JsonBody,
    capability: Capability,
) -> Result<JsonBody, CoreError> {
    if source == target {
        return Ok(body);
    }
    let pair = resolve(source, target).map_err(transform_error)?;
    let ctx = TransformContext::new(source, target);
    let output =
        dispatch::response_bytes_detailed(pair, &ctx, body.as_bytes()).map_err(transform_error)?;
    strict(output.diagnostics, target, capability)?;
    JsonBody::from_bytes(Bytes::from(output.value))
}

fn model_query(request: &ListModelsRequest, provider: Provider) -> Vec<QueryParam> {
    let (cursor_name, limit_name) = match provider {
        Provider::OpenAi => return Vec::new(),
        Provider::Claude => ("after_id", "limit"),
        Provider::Gemini => ("pageToken", "pageSize"),
        _ => return Vec::new(),
    };
    [
        request.cursor.as_ref().map(|value| QueryParam {
            name: cursor_name.into(),
            value: value.clone(),
        }),
        request.limit.map(|value| QueryParam {
            name: limit_name.into(),
            value: value.to_string(),
        }),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn decode_segment(value: &Value, target: OperationKey) -> Result<TranscriptSegment, CoreError> {
    let id = value
        .get("id")
        .and_then(|id| match id {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
        .ok_or_else(|| invalid_payload(target, "missing or invalid id"))?;
    Ok(TranscriptSegment {
        id,
        text: required_str(value, "text", target)?.into(),
        start_seconds: required_f64(value, "start", target)?,
        end_seconds: required_f64(value, "end", target)?,
        speaker: value
            .get("speaker")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn value_field<'a>(value: &'a Value, field: &str) -> Result<&'a str, CoreError> {
    value.get(field).and_then(Value::as_str).ok_or_else(|| {
        CoreError::Endpoint(format!(
            "provider stream event is missing string field {field}"
        ))
    })
}

fn array_field<'a>(
    value: &'a Value,
    field: &str,
    target: OperationKey,
) -> Result<&'a Vec<Value>, CoreError> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_payload(target, &format!("missing or invalid {field} array")))
}

fn required_str<'a>(
    value: &'a Value,
    field: &str,
    target: OperationKey,
) -> Result<&'a str, CoreError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_payload(target, &format!("missing or invalid string {field}")))
}

fn required_u64(value: &Value, field: &str, target: OperationKey) -> Result<u64, CoreError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_payload(target, &format!("missing or invalid integer {field}")))
}

fn required_u32(value: &Value, field: &str, target: OperationKey) -> Result<u32, CoreError> {
    u32::try_from(required_u64(value, field, target)?)
        .map_err(|_| invalid_payload(target, &format!("{field} exceeds u32")))
}

fn required_f64(value: &Value, field: &str, target: OperationKey) -> Result<f64, CoreError> {
    value
        .get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| invalid_payload(target, &format!("missing or invalid number {field}")))
}

fn invalid_payload(target: OperationKey, reason: &str) -> CoreError {
    CoreError::InvalidProviderPayload {
        target,
        reason: reason.to_owned(),
    }
}

fn media_json(source: &MediaSource) -> Result<String, CoreError> {
    Ok(match source {
        MediaSource::Url { url } => url.clone(),
        MediaSource::File { id } => id.0.clone(),
        MediaSource::Data { media_type, bytes } => format!(
            "data:{};base64,{}",
            media_type.0,
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ),
    })
}
fn file_part(name: &str, source: &MediaSource) -> Result<MultipartPart, CoreError> {
    match source {
        MediaSource::Data { media_type, bytes } => Ok(MultipartPart {
            name: name.into(),
            value: MultipartValue::File {
                filename: Some("upload".into()),
                content_type: Some(media_type.0.clone()),
                data: bytes.clone(),
            },
        }),
        _ => Err(CoreError::Endpoint(
            "multipart upload requires inline media data".into(),
        )),
    }
}
fn text_part(name: &str, value: &str) -> MultipartPart {
    MultipartPart {
        name: name.into(),
        value: MultipartValue::Text(value.into()),
    }
}
fn push_opt<T: ToString>(parts: &mut Vec<MultipartPart>, name: &str, value: Option<T>) {
    if let Some(value) = value {
        parts.push(text_part(name, &value.to_string()));
    }
}
fn json_body(v: Value) -> Result<RequestBody, CoreError> {
    Ok(RequestBody::Json(JsonBody::encode(&v)?))
}
fn mode(value: ImageMode) -> ResponseMode {
    match value {
        ImageMode::Complete => ResponseMode::Json,
        ImageMode::Stream => ResponseMode::JsonSse,
    }
}
fn image_size(o: &ImageOptions) -> Option<String> {
    Some(format!("{}x{}", o.width?, o.height?))
}
fn enum_string<T: serde::Serialize>(v: Option<T>) -> Option<String> {
    v.map(|value| {
        serde_json::to_value(value)
            .expect("IR enums are serializable")
            .as_str()
            .expect("IR enums serialize as strings")
            .to_owned()
    })
}
fn decode_usage(v: &Value, target: OperationKey) -> Result<crate::llm::ir::Usage, CoreError> {
    let input_tokens = v
        .get("prompt_tokens")
        .or_else(|| v.get("input_tokens"))
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_payload(target, "usage input token count is missing"))?;
    let output_tokens = v
        .get("output_tokens")
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                invalid_payload(target, "usage output token count must be an integer")
            })
        })
        .transpose()?
        .unwrap_or(0);
    Ok(crate::llm::ir::Usage {
        input_tokens,
        output_tokens,
        cached_input_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: v
            .get("total_tokens")
            .map(|value| {
                value.as_u64().ok_or_else(|| {
                    invalid_payload(target, "usage total token count must be an integer")
                })
            })
            .transpose()?
            .unwrap_or(input_tokens + output_tokens),
    })
}
fn is_stream(r: &OperationRequest) -> bool {
    matches!(
        r,
        OperationRequest::Images(
            ImageRequest::Generate(GenerateImageRequest {
                mode: ImageMode::Stream,
                ..
            }) | ImageRequest::Edit(EditImageRequest {
                mode: ImageMode::Stream,
                ..
            })
        ) | OperationRequest::Audio(AudioRequest::Transcribe(TranscriptionRequest {
            mode: TranscriptionMode::Stream,
            ..
        })) | OperationRequest::Audio(AudioRequest::Speech(SpeechRequest {
            mode: SpeechMode::Stream,
            ..
        }))
    )
}
fn parse_query(q: &str) -> Result<Vec<QueryParam>, CoreError> {
    Ok(q.split('&')
        .filter(|v| !v.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            QueryParam {
                name: k.into(),
                value: v.into(),
            }
        })
        .collect())
}
fn strict(
    d: Vec<gproxy_transform::TransformDiagnostic>,
    target: OperationKey,
    capability: Capability,
) -> Result<(), CoreError> {
    if d.is_empty() {
        Ok(())
    } else {
        Err(CoreError::UnsupportedCapability { capability, target })
    }
}
fn request_capability(operation: Operation) -> Capability {
    match operation {
        Operation::ListModels | Operation::GetModel => Capability::ModelCatalog,
        Operation::CountTokens => Capability::TokenCounting,
        Operation::CreateEmbedding => Capability::Embeddings,
        Operation::CreateImage => Capability::ImageGeneration,
        Operation::EditImage => Capability::ImageEditing,
        Operation::CreateVideo
        | Operation::RetrieveVideo
        | Operation::ListVideos
        | Operation::DeleteVideo
        | Operation::DownloadVideoContent => Capability::VideoGeneration,
        Operation::CreateSpeech => Capability::Speech,
        Operation::CreateTranscription => Capability::Transcription,
        Operation::CreateTranslation => Capability::Translation,
        Operation::WebSearch => Capability::WebSearch,
        Operation::Rerank => Capability::Rerank,
        Operation::CompactContent => Capability::Compaction,
        Operation::CreateConversation => Capability::Conversation,
        Operation::CreateRealtimeCall | Operation::ConnectRealtime => Capability::Realtime,
        _ => Capability::TextGeneration,
    }
}
fn transform_error(e: gproxy_transform::TransformError) -> CoreError {
    CoreError::Transform(format!("{e:?}"))
}
fn unsupported(capability: Capability, target: OperationKey) -> CoreError {
    CoreError::UnsupportedCapability { capability, target }
}
fn mode_error() -> CoreError {
    CoreError::Endpoint("wire response mode does not match semantic operation".into())
}
