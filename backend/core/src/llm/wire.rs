use std::pin::Pin;

use bytes::Bytes;
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt};
use http::{HeaderMap, Method, StatusCode};

use crate::CoreError;

#[derive(Clone, Debug)]
pub struct WireRequest {
    pub method: Method,
    pub path: String,
    pub query: Vec<QueryParam>,
    pub headers: HeaderMap,
    pub body: RequestBody,
    pub response_mode: ResponseMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryParam {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug)]
pub enum RequestBody {
    Empty,
    Json(JsonBody),
    Multipart(MultipartBody),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonBody(Bytes);

impl JsonBody {
    pub fn encode(value: &impl serde::Serialize) -> Result<Self, CoreError> {
        Self::from_bytes(Bytes::from(serde_json::to_vec(value)?))
    }

    pub fn from_bytes(bytes: Bytes) -> Result<Self, CoreError> {
        serde_json::from_slice::<serde_json::Value>(&bytes)?;
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &Bytes {
        &self.0
    }

    pub fn into_bytes(self) -> Bytes {
        self.0
    }

    pub fn decode<T: serde::de::DeserializeOwned>(&self) -> Result<T, CoreError> {
        Ok(serde_json::from_slice(&self.0)?)
    }
}

#[derive(Clone, Debug, Default)]
pub struct MultipartBody {
    pub parts: Vec<MultipartPart>,
}

#[derive(Clone, Debug)]
pub struct MultipartPart {
    pub name: String,
    pub value: MultipartValue,
}

#[derive(Clone, Debug)]
pub enum MultipartValue {
    Text(String),
    File {
        filename: Option<String>,
        content_type: Option<String>,
        data: Bytes,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseMode {
    Json,
    Binary,
    JsonSse,
}

#[derive(Clone, Debug)]
pub struct ResponseMetadata {
    pub status: StatusCode,
    pub headers: HeaderMap,
}

pub enum WireResult {
    Success(WireResponse),
    Rejected(UpstreamErrorResponse),
}

pub enum WireResponse {
    Json(JsonResponse),
    Binary(BinaryResponse),
    JsonSse(JsonSseResponse),
}

#[derive(Clone, Debug)]
pub struct JsonResponse {
    pub metadata: ResponseMetadata,
    pub body: JsonBody,
}

#[derive(Clone, Debug)]
pub struct BinaryResponse {
    pub metadata: ResponseMetadata,
    pub content_type: Option<String>,
    pub body: Bytes,
}

pub struct JsonSseResponse {
    pub metadata: ResponseMetadata,
    pub stream: JsonSseStream,
}

#[derive(Clone, Debug)]
pub struct UpstreamErrorResponse {
    pub metadata: ResponseMetadata,
    pub body: Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonSseFrame {
    pub event: Option<String>,
    pub id: Option<String>,
    pub data: JsonSseData,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsonSseData {
    Json(JsonBody),
    Done,
}

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, CoreError>> + Send>>;
pub type JsonSseStream = Pin<Box<dyn Stream<Item = Result<JsonSseFrame, CoreError>> + Send>>;

pub fn parse_json_sse(stream: ByteStream) -> JsonSseStream {
    let events = stream.eventsource();
    Box::pin(events.map(|result| {
        let event =
            result.map_err(|error| CoreError::Transform(format!("invalid SSE stream: {error}")))?;
        let data = if event.data == "[DONE]" {
            JsonSseData::Done
        } else {
            JsonSseData::Json(JsonBody::from_bytes(Bytes::from(event.data))?)
        };
        Ok(JsonSseFrame {
            event: (!event.event.is_empty()).then_some(event.event),
            id: (!event.id.is_empty()).then_some(event.id),
            data,
        })
    }))
}

#[cfg(test)]
mod tests {
    use futures_util::{stream, StreamExt};

    use super::*;

    #[tokio::test]
    async fn parses_json_frames_and_done_marker() {
        let input = stream::iter([Ok(Bytes::from_static(
            b"id: 7\nevent: delta\ndata: {\"text\":\"hello\"}\n\ndata: [DONE]\n\n",
        ))]);
        let frames = parse_json_sse(Box::pin(input)).collect::<Vec<_>>().await;

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].as_ref().unwrap().event.as_deref(), Some("delta"));
        assert_eq!(frames[0].as_ref().unwrap().id.as_deref(), Some("7"));
        assert!(matches!(
            frames[1].as_ref().unwrap().data,
            JsonSseData::Done
        ));
    }
}
