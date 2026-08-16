mod generation;
mod other;
pub(crate) mod realtime;

use std::pin::Pin;

use futures_util::Stream;
use gproxy_protocol::OperationKey;

use crate::llm::ir::generation::GenerateEvent;
use crate::llm::ir::{OperationRequest, OperationResponse};
use crate::llm::wire::{JsonSseStream, WireRequest, WireResponse};
use crate::CoreError;

pub type SemanticEventStream =
    Pin<Box<dyn Stream<Item = Result<OperationEvent, CoreError>> + Send>>;

#[derive(Clone, Debug, PartialEq, serde::Serialize, ts_rs::TS)]
#[serde(tag = "operation", content = "event", rename_all = "snake_case")]
pub enum OperationEvent {
    Generate(GenerateEvent),
    Image(crate::llm::ir::images::ImageEvent),
    Speech(crate::llm::ir::audio::SpeechEvent),
    Transcription(crate::llm::ir::audio::TranscriptionEvent),
}

pub enum DecodedResponse {
    Complete(OperationResponse),
    Stream(SemanticEventStream),
}

pub struct ProviderCodec;

impl ProviderCodec {
    pub fn encode(
        request: &OperationRequest,
        target: OperationKey,
    ) -> Result<WireRequest, CoreError> {
        match request {
            OperationRequest::Generate(request) => generation::encode(request, target),
            _ => other::encode(request, target),
        }
    }

    pub fn decode(
        request: &OperationRequest,
        target: OperationKey,
        response: WireResponse,
    ) -> Result<DecodedResponse, CoreError> {
        match request {
            OperationRequest::Generate(request) => generation::decode(request, target, response),
            _ => other::decode(request, target, response),
        }
    }
}

fn map_sse(
    stream: JsonSseStream,
    mut f: impl FnMut(crate::llm::wire::JsonSseFrame) -> Result<Vec<OperationEvent>, CoreError>
        + Send
        + 'static,
) -> SemanticEventStream {
    use futures_util::StreamExt;
    let stream = stream
        .map(move |frame| frame.and_then(&mut f))
        .flat_map(|batch| match batch {
            Ok(events) => futures_util::stream::iter(
                events
                    .into_iter()
                    .map(Ok::<_, CoreError>)
                    .collect::<Vec<_>>(),
            ),
            Err(error) => futures_util::stream::iter(vec![Err(error)]),
        });
    Box::pin(stream)
}
