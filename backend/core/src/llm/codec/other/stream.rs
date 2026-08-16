use super::*;

pub(super) fn decode_image_stream(
    request: &ImageRequest,
    target: OperationKey,
    stream: crate::llm::wire::JsonSseStream,
) -> Result<DecodedResponse, CoreError> {
    let (expected, total) = match request {
        ImageRequest::Generate(request) => ("image_generation", request.options.partial_images),
        ImageRequest::Edit(request) => ("image_edit", request.options.partial_images),
    };
    Ok(DecodedResponse::Stream(crate::llm::codec::map_sse(
        stream,
        move |frame| {
            use crate::llm::codec::OperationEvent;
            use crate::llm::wire::JsonSseData;
            let JsonSseData::Json(body) = frame.data else {
                return Ok(Vec::new());
            };
            let value: Value = body.decode()?;
            let kind = value
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid_payload(target, "image stream event type is missing"))?;
            let events = if kind == format!("{expected}.partial_image") {
                let encoded = value_field(&value, "b64_json")?;
                let sequence = required_u32(&value, "partial_image_index", target)?;
                vec![
                    ImageEvent::Progress(ImageProgress {
                        completed: sequence + 1,
                        total,
                    }),
                    ImageEvent::Preview(ImagePreview {
                        index: 0,
                        sequence,
                        image: ImageArtifact {
                            id: crate::llm::ir::OutputId(sequence.to_string()),
                            source: MediaSource::Data {
                                media_type: crate::llm::ir::MediaType("image/png".into()),
                                bytes: Bytes::from(
                                    base64::engine::general_purpose::STANDARD
                                        .decode(encoded)
                                        .map_err(|error| CoreError::Endpoint(error.to_string()))?,
                                ),
                            },
                            revised_prompt: None,
                        },
                    }),
                ]
            } else if kind == format!("{expected}.completed") {
                vec![ImageEvent::Finished(super::response::decode_images(
                    &completed_response(&value),
                    target,
                )?)]
            } else {
                return Err(CoreError::UnmodeledProviderEvent {
                    target,
                    event: kind.into(),
                });
            };
            Ok(events.into_iter().map(OperationEvent::Image).collect())
        },
    )))
}

/// The completed stream event carries a single top-level `b64_json` artifact;
/// wrap it into the buffered response shape expected by `decode_images`.
fn completed_response(value: &Value) -> Value {
    if value.get("data").is_some() {
        return value.clone();
    }
    let mut wrapper = serde_json::Map::new();
    for field in ["usage", "output_format"] {
        if let Some(entry) = value.get(field) {
            wrapper.insert(field.into(), entry.clone());
        }
    }
    wrapper.insert("data".into(), Value::Array(vec![value.clone()]));
    Value::Object(wrapper)
}

pub(super) fn decode_speech_stream(
    request: &SpeechRequest,
    response: crate::llm::wire::BinaryStreamResponse,
) -> Result<DecodedResponse, CoreError> {
    use futures_util::StreamExt;

    let media_type = response
        .content_type
        .unwrap_or_else(|| request.format.media_type().to_owned());
    let started = futures_util::stream::once(async move {
        Ok(crate::llm::codec::OperationEvent::Speech(
            SpeechEvent::Started { media_type },
        ))
    });
    let deltas = response.stream.map(|chunk| {
        chunk.map(|bytes| {
            crate::llm::codec::OperationEvent::Speech(SpeechEvent::AudioDelta { bytes })
        })
    });
    let finished = futures_util::stream::once(async {
        Ok(crate::llm::codec::OperationEvent::Speech(
            SpeechEvent::Finished,
        ))
    });
    Ok(DecodedResponse::Stream(Box::pin(
        started.chain(deltas).chain(finished),
    )))
}
pub(super) fn decode_transcription_stream(
    target: OperationKey,
    stream: crate::llm::wire::JsonSseStream,
) -> Result<DecodedResponse, CoreError> {
    Ok(DecodedResponse::Stream(crate::llm::codec::map_sse(
        stream,
        move |frame| {
            use crate::llm::codec::OperationEvent;
            use crate::llm::wire::JsonSseData;
            let JsonSseData::Json(body) = frame.data else {
                return Ok(Vec::new());
            };
            let value: Value = body.decode()?;
            let kind = value.get("type").and_then(Value::as_str).ok_or_else(|| {
                invalid_payload(target, "transcription stream event type is missing")
            })?;
            let event = match kind {
                "transcript.text.delta" => TranscriptionEvent::TextDelta {
                    text: value_field(&value, "delta")?.into(),
                },
                "transcript.text.segment" => {
                    TranscriptionEvent::Segment(decode_segment(&value, target)?)
                }
                "transcript.text.done" => TranscriptionEvent::Finished(
                    super::response::decode_transcription(&value, target)?,
                ),
                other => {
                    return Err(CoreError::UnmodeledProviderEvent {
                        target,
                        event: other.into(),
                    })
                }
            };
            Ok(vec![OperationEvent::Transcription(event)])
        },
    )))
}
