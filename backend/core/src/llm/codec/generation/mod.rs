pub(in crate::llm::codec) mod request;
pub(in crate::llm::codec) mod response;
mod stream;
#[cfg(test)]
mod tests;

use bytes::Bytes;
use gproxy_protocol::{ContentGenerationKind, Operation, OperationKey};
use gproxy_transform::{dispatch, resolve, TransformContext};
use http::HeaderMap;
use serde_json::{Map, Value};

use super::{DecodedResponse, OperationEvent};
use crate::llm::ir::generation::{GenerateMode, GenerateRequest};
use crate::llm::wire::{
    JsonBody, JsonSseData, QueryParam, RequestBody, ResponseMode, WireRequest, WireResponse,
};
use crate::CoreError;

const CANONICAL_KIND: ContentGenerationKind = ContentGenerationKind::OpenAiResponses;

pub fn encode(request: &GenerateRequest, target: OperationKey) -> Result<WireRequest, CoreError> {
    request::validate_capabilities(request, target)?;
    let source = OperationKey::content_generation(request.operation(), CANONICAL_KIND);
    let canonical = JsonBody::encode(&request::encode_request(request)?)?;
    let body = transform_request(source, target, canonical)?;
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

pub fn decode(
    request: &GenerateRequest,
    target: OperationKey,
    response: WireResponse,
) -> Result<DecodedResponse, CoreError> {
    let canonical = OperationKey::content_generation(request.operation(), CANONICAL_KIND);
    match response {
        WireResponse::Json(response) if request.mode == GenerateMode::Complete => {
            let body = transform_response(target, canonical, response.body)?;
            Ok(DecodedResponse::Complete(
                crate::llm::ir::OperationResponse::Generate(response::decode_complete(&body)?),
            ))
        }
        WireResponse::JsonSse(response) if request.mode == GenerateMode::Stream => {
            let mut decoder = stream::StreamDecoder::new(target, canonical)?;
            Ok(DecodedResponse::Stream(super::map_sse(
                response.stream,
                move |frame| decoder.push(frame),
            )))
        }
        _ => Err(CoreError::Endpoint(
            "generation response mode does not match request".to_owned(),
        )),
    }
}

impl GenerateRequest {
    fn operation(&self) -> Operation {
        match self.mode {
            GenerateMode::Complete => Operation::GenerateContent,
            GenerateMode::Stream => Operation::StreamGenerateContent,
        }
    }
}

fn transform_request(
    source: OperationKey,
    target: OperationKey,
    body: JsonBody,
) -> Result<JsonBody, CoreError> {
    if source == target {
        return Ok(body);
    }
    let pair = resolve(source, target).map_err(transform_error)?;
    let ctx = TransformContext::new(source, target);
    let output =
        dispatch::request_bytes_detailed(pair, &ctx, body.as_bytes()).map_err(transform_error)?;
    if !output.diagnostics.is_empty() {
        return Err(CoreError::Transform(format!(
            "semantic loss while encoding request: {:?}",
            output.diagnostics
        )));
    }
    JsonBody::from_bytes(Bytes::from(output.value))
}

fn transform_response(
    source: OperationKey,
    target: OperationKey,
    body: JsonBody,
) -> Result<JsonBody, CoreError> {
    if source == target {
        return Ok(body);
    }
    let pair = resolve(source, target).map_err(transform_error)?;
    let ctx = TransformContext::new(source, target);
    let output =
        dispatch::response_bytes_detailed(pair, &ctx, body.as_bytes()).map_err(transform_error)?;
    if !output.diagnostics.is_empty() {
        return Err(CoreError::Transform(format!(
            "semantic loss while decoding response: {:?}",
            output.diagnostics
        )));
    }
    JsonBody::from_bytes(Bytes::from(output.value))
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
fn invalid_payload(reason: &str) -> CoreError {
    CoreError::InvalidProviderPayload {
        target: OperationKey::content_generation(Operation::GenerateContent, CANONICAL_KIND),
        reason: reason.to_owned(),
    }
}
fn kind_key(value: Option<&str>) -> &str {
    value.unwrap_or("missing")
}
fn unmodeled(event: &str, operation: Operation) -> CoreError {
    CoreError::UnmodeledProviderEvent {
        target: OperationKey::content_generation(operation, CANONICAL_KIND),
        event: event.into(),
    }
}
fn transform_error(error: gproxy_transform::TransformError) -> CoreError {
    CoreError::Transform(format!("{error:?}"))
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
