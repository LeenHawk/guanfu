use base64::Engine;
use bytes::Bytes;
use gproxy_protocol::{OperationKey, Provider};
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
use crate::llm::ir::{Capability, MediaSource, ModelId, OperationRequest, OperationResponse};
use crate::llm::wire::{
    JsonBody, MultipartBody, MultipartPart, MultipartValue, QueryParam, RequestBody, ResponseMode,
    WireRequest, WireResponse,
};
use crate::CoreError;

pub fn encode(request: &OperationRequest, target: OperationKey) -> Result<WireRequest, CoreError> {
    let (canonical, body, query, response_mode) = canonical_request(request)?;
    let endpoint = gproxy_protocol::endpoint::request_target(
        target,
        request.model_id().unwrap_or_default(),
        is_stream(request),
    )
    .map_err(|error| CoreError::Endpoint(error.to_string()))?;
    let (body, mut query) = transform_request(canonical, target, body, query)?;
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
        body: prepare_body(request, target, body)?,
        response_mode,
    })
}

pub fn decode(
    request: &OperationRequest,
    target: OperationKey,
    response: WireResponse,
) -> Result<DecodedResponse, CoreError> {
    let canonical = canonical_key(request)?;
    match (request, response) {
        (
            OperationRequest::Audio(AudioRequest::Speech(request)),
            WireResponse::Binary(response),
        ) => Ok(DecodedResponse::Complete(OperationResponse::Audio(
            AudioResponse::Speech(AudioArtifact {
                media_type: response
                    .content_type
                    .unwrap_or_else(|| request.format.media_type().to_owned()),
                bytes: response.body,
            }),
        ))),
        (OperationRequest::Audio(AudioRequest::Speech(_)), _) => Err(mode_error()),
        (_, WireResponse::Json(response)) => {
            let body = transform_response(target, canonical, response.body)?;
            Ok(DecodedResponse::Complete(decode_json(request, &body)?))
        }
        (OperationRequest::Images(image_request), WireResponse::JsonSse(response)) => {
            decode_image_stream(image_request, target, response.stream)
        }
        (OperationRequest::Audio(AudioRequest::Transcribe(_)), WireResponse::JsonSse(response)) => {
            decode_transcription_stream(target, response.stream)
        }
        _ => Err(mode_error()),
    }
}

fn canonical_key(request: &OperationRequest) -> Result<OperationKey, CoreError> {
    let provider = match request {
        OperationRequest::Embeddings(EmbeddingRequest {
            input: EmbeddingInput::TextBatch { .. } | EmbeddingInput::TokenBatch { .. },
            ..
        }) => Provider::OpenAi,
        _ => Provider::OpenAi,
    };
    Ok(OperationKey::provider(request.operation(), provider))
}

fn canonical_request(
    request: &OperationRequest,
) -> Result<(OperationKey, RequestBody, Vec<QueryParam>, ResponseMode), CoreError> {
    let key = canonical_key(request)?;
    let result = match request {
        OperationRequest::Models(ModelRequest::List(request)) => {
            let query = [
                request.limit.map(|value| QueryParam {
                    name: "limit".into(),
                    value: value.to_string(),
                }),
                request.cursor.as_ref().map(|value| QueryParam {
                    name: "after_id".into(),
                    value: value.clone(),
                }),
            ]
            .into_iter()
            .flatten()
            .collect();
            (RequestBody::Empty, query, ResponseMode::Json)
        }
        OperationRequest::Models(ModelRequest::Get(_)) => {
            (RequestBody::Empty, Vec::new(), ResponseMode::Json)
        }
        OperationRequest::CountTokens(request) => {
            let input = match &request.input {
                TokenCountInput::Text { values } => {
                    Value::Array(values.iter().cloned().map(Value::String).collect())
                }
                TokenCountInput::Generation(input) => {
                    super::generation::encode_input(&input.instructions, &input.input)?
                }
            };
            let tools = match &request.input {
                TokenCountInput::Generation(input) if !input.tools.is_empty() => {
                    Some(Value::Array(
                        input
                            .tools
                            .iter()
                            .map(super::generation::encode_tool)
                            .collect::<Result<_, _>>()?,
                    ))
                }
                _ => None,
            };
            (
                json_body(json!({"model":request.model.0,"input":input,"tools":tools}))?,
                Vec::new(),
                ResponseMode::Json,
            )
        }
        OperationRequest::Embeddings(request) => {
            let input = match &request.input {
                EmbeddingInput::Text { value } => json!(value),
                EmbeddingInput::TextBatch { values } => json!(values),
                EmbeddingInput::Tokens { value } => json!(value),
                EmbeddingInput::TokenBatch { values } => json!(values),
            };
            (
                json_body(
                    json!({"model":request.model.0,"input":input,"dimensions":request.dimensions}),
                )?,
                Vec::new(),
                ResponseMode::Json,
            )
        }
        OperationRequest::Images(ImageRequest::Generate(request)) => (
            json_body(
                json!({"model":request.model.0,"prompt":request.prompt,"n":request.count,"size":image_size(&request.options),"quality":enum_string(request.options.quality),"background":enum_string(request.options.background),"output_format":enum_string(request.options.output_format),"output_compression":request.options.compression,"moderation":enum_string(request.options.moderation),"partial_images":request.options.partial_images,"stream":request.mode==ImageMode::Stream}),
            )?,
            Vec::new(),
            mode(request.mode),
        ),
        OperationRequest::Images(ImageRequest::Edit(request)) => (
            json_body(
                json!({"model":request.model.0,"prompt":request.prompt,"image":request.images.iter().map(media_json).collect::<Result<Vec<_>,_>>()?,"mask":request.mask.as_ref().map(media_json).transpose()?,"n":request.count,"input_fidelity":enum_string(request.input_fidelity),"size":image_size(&request.options),"quality":enum_string(request.options.quality),"background":enum_string(request.options.background),"output_format":enum_string(request.options.output_format),"output_compression":request.options.compression,"moderation":enum_string(request.options.moderation),"partial_images":request.options.partial_images,"stream":request.mode==ImageMode::Stream}),
            )?,
            Vec::new(),
            mode(request.mode),
        ),
        OperationRequest::Audio(AudioRequest::Speech(request)) => {
            if request.mode == SpeechMode::Stream {
                return Err(unsupported(Capability::Speech, key));
            }
            (
                json_body(
                    json!({"model":request.model.0,"input":request.input,"voice":request.voice.0,"instructions":request.instructions,"response_format":enum_string(Some(request.format)),"speed":request.speed}),
                )?,
                Vec::new(),
                ResponseMode::Binary,
            )
        }
        OperationRequest::Audio(AudioRequest::Transcribe(request)) => (
            RequestBody::Empty,
            Vec::new(),
            match request.mode {
                TranscriptionMode::Complete => ResponseMode::Json,
                TranscriptionMode::Stream => ResponseMode::JsonSse,
            },
        ),
        OperationRequest::Audio(AudioRequest::Translate(_)) => {
            (RequestBody::Empty, Vec::new(), ResponseMode::Json)
        }
        OperationRequest::Search(SearchRequest::Rerank(request)) => (
            json_body(
                json!({"model":request.model.0,"query":request.query,"documents":request.documents.iter().map(|doc|json!({"text":doc.text,"title":doc.title,"id":doc.id})).collect::<Vec<_>>(),"top_n":request.top_n,"return_documents":request.return_documents}),
            )?,
            Vec::new(),
            ResponseMode::Json,
        ),
        OperationRequest::Search(SearchRequest::Web(request)) => (
            json_body(
                json!({"model":request.model.as_ref().map(|v|&v.0),"query":request.query,"max_results":request.max_results,"allowed_domains":request.allowed_domains,"blocked_domains":request.blocked_domains,"location":request.location}),
            )?,
            Vec::new(),
            ResponseMode::Json,
        ),
        OperationRequest::Platform(PlatformRequest::Compact(request)) => (
            json_body(
                json!({"model":request.model.0,"input":super::generation::encode_input(&request.instructions,&request.input)?,"max_output_tokens":request.max_output_tokens}),
            )?,
            Vec::new(),
            ResponseMode::Json,
        ),
        OperationRequest::Platform(PlatformRequest::CreateConversation(request)) => (
            json_body(
                json!({"items":super::generation::encode_input(&[],&request.items)?,"metadata":request.metadata}),
            )?,
            Vec::new(),
            ResponseMode::Json,
        ),
        OperationRequest::Platform(PlatformRequest::CreateRealtimeCall(request)) => (
            json_body(json!({"session":request.session,"sdp":request.offer_sdp}))?,
            Vec::new(),
            ResponseMode::Json,
        ),
        OperationRequest::Platform(PlatformRequest::ConnectRealtime(_)) => {
            return Err(unsupported(Capability::Realtime, key))
        }
        OperationRequest::Generate(_) => unreachable!("generation codec handles generation"),
    };
    Ok((key, result.0, result.1, result.2))
}

fn prepare_body(
    request: &OperationRequest,
    target: OperationKey,
    transformed: RequestBody,
) -> Result<RequestBody, CoreError> {
    if target.provider_family() != Provider::OpenAi {
        return Ok(transformed);
    }
    match request {
        OperationRequest::Images(ImageRequest::Edit(request)) => image_edit_multipart(request),
        OperationRequest::Audio(AudioRequest::Transcribe(request)) => {
            transcription_multipart(request)
        }
        OperationRequest::Audio(AudioRequest::Translate(request)) => translation_multipart(request),
        _ => Ok(transformed),
    }
}

fn image_edit_multipart(request: &EditImageRequest) -> Result<RequestBody, CoreError> {
    let mut parts = vec![
        text_part("model", &request.model.0),
        text_part("prompt", &request.prompt),
    ];
    for image in &request.images {
        parts.push(file_part("image[]", image)?);
    }
    if let Some(mask) = &request.mask {
        parts.push(file_part("mask", mask)?);
    }
    push_opt(&mut parts, "n", request.count);
    push_opt(&mut parts, "size", image_size(&request.options));
    push_opt(&mut parts, "quality", enum_string(request.options.quality));
    push_opt(
        &mut parts,
        "input_fidelity",
        enum_string(request.input_fidelity),
    );
    push_opt(
        &mut parts,
        "output_format",
        enum_string(request.options.output_format),
    );
    Ok(RequestBody::Multipart(MultipartBody { parts }))
}

fn transcription_multipart(request: &TranscriptionRequest) -> Result<RequestBody, CoreError> {
    let mut parts = vec![
        file_part("file", &request.audio)?,
        text_part("model", &request.model.0),
    ];
    push_opt(&mut parts, "language", request.language.clone());
    push_opt(&mut parts, "prompt", request.prompt.clone());
    push_opt(&mut parts, "temperature", request.temperature);
    if request.mode == TranscriptionMode::Stream {
        parts.push(text_part("stream", "true"));
    }
    for value in &request.timestamps {
        parts.push(text_part(
            "timestamp_granularities[]",
            &enum_string(Some(*value)).unwrap_or_default(),
        ));
    }
    Ok(RequestBody::Multipart(MultipartBody { parts }))
}

fn translation_multipart(request: &TranslationRequest) -> Result<RequestBody, CoreError> {
    let mut parts = vec![
        file_part("file", &request.audio)?,
        text_part("model", &request.model.0),
    ];
    push_opt(&mut parts, "prompt", request.prompt.clone());
    push_opt(&mut parts, "temperature", request.temperature);
    Ok(RequestBody::Multipart(MultipartBody { parts }))
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
            strict(output.diagnostics, target)?;
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
) -> Result<JsonBody, CoreError> {
    if source == target {
        return Ok(body);
    }
    let pair = resolve(source, target).map_err(transform_error)?;
    let ctx = TransformContext::new(source, target);
    let output =
        dispatch::response_bytes_detailed(pair, &ctx, body.as_bytes()).map_err(transform_error)?;
    strict(output.diagnostics, target)?;
    JsonBody::from_bytes(Bytes::from(output.value))
}

fn decode_json(
    request: &OperationRequest,
    body: &JsonBody,
) -> Result<OperationResponse, CoreError> {
    let value: Value = body.decode()?;
    Ok(match request {
        OperationRequest::Models(ModelRequest::List(_)) => {
            OperationResponse::Models(ModelResponse::List(ModelPage {
                models: value
                    .get("data")
                    .and_then(Value::as_array)
                    .unwrap_or(&vec![])
                    .iter()
                    .map(decode_model)
                    .collect(),
                next_cursor: value
                    .get("next_cursor")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            }))
        }
        OperationRequest::Models(ModelRequest::Get(_)) => {
            OperationResponse::Models(ModelResponse::One(decode_model(&value)))
        }
        OperationRequest::CountTokens(_) => OperationResponse::CountTokens(CountTokensResponse {
            input_tokens: value
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        }),
        OperationRequest::Embeddings(_) => OperationResponse::Embeddings(EmbeddingResponse {
            model: value
                .get("model")
                .and_then(Value::as_str)
                .map(|v| ModelId(v.into())),
            vectors: value
                .get("data")
                .and_then(Value::as_array)
                .unwrap_or(&vec![])
                .iter()
                .map(|item| EmbeddingVector {
                    index: item
                        .get("index")
                        .and_then(Value::as_u64)
                        .and_then(|v| u32::try_from(v).ok())
                        .unwrap_or(0),
                    values: item
                        .get("embedding")
                        .and_then(Value::as_array)
                        .unwrap_or(&vec![])
                        .iter()
                        .filter_map(Value::as_f64)
                        .map(|v| v as f32)
                        .collect(),
                })
                .collect(),
            usage: Some(decode_usage(value.get("usage"))),
        }),
        OperationRequest::Images(_) => OperationResponse::Images(decode_images(&value)?),
        OperationRequest::Audio(AudioRequest::Transcribe(_)) => {
            OperationResponse::Audio(AudioResponse::Transcription(decode_transcription(&value)))
        }
        OperationRequest::Audio(AudioRequest::Translate(_)) => {
            OperationResponse::Audio(AudioResponse::Translation(Translation {
                text: value
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                source_language: value
                    .get("language")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                duration_seconds: value.get("duration").and_then(Value::as_f64),
                segments: decode_segments(&value),
            }))
        }
        OperationRequest::Search(SearchRequest::Rerank(_)) => {
            OperationResponse::Search(SearchResponse::Rerank(RerankResponse {
                results: value
                    .get("results")
                    .and_then(Value::as_array)
                    .unwrap_or(&vec![])
                    .iter()
                    .map(|r| RerankResult {
                        index: u32v(r, "index"),
                        relevance_score: r
                            .get("relevance_score")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0) as f32,
                        document: r.get("document").map(|d| RerankDocument {
                            id: None,
                            text: d
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .into(),
                            title: d.get("title").and_then(Value::as_str).map(str::to_owned),
                        }),
                    })
                    .collect(),
                usage: Some(decode_usage(value.get("usage"))),
            }))
        }
        OperationRequest::Search(SearchRequest::Web(_)) => {
            OperationResponse::Search(SearchResponse::Web(WebSearchResponse {
                results: value
                    .get("data")
                    .or_else(|| value.get("results"))
                    .and_then(Value::as_array)
                    .unwrap_or(&vec![])
                    .iter()
                    .map(|r| WebSearchResult {
                        url: r
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .into(),
                        title: r
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .into(),
                        snippet: r.get("snippet").and_then(Value::as_str).map(str::to_owned),
                        published_at: r
                            .get("published_at")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        score: r.get("score").and_then(Value::as_f64).map(|v| v as f32),
                    })
                    .collect(),
                usage: Some(decode_usage(value.get("usage"))),
            }))
        }
        OperationRequest::Platform(PlatformRequest::Compact(_)) => {
            OperationResponse::Platform(PlatformResponse::Compact(CompactResponse {
                output: Vec::new(),
                encrypted_content: value
                    .get("encrypted_content")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                usage: Some(decode_usage(value.get("usage"))),
            }))
        }
        OperationRequest::Platform(PlatformRequest::CreateConversation(_)) => {
            OperationResponse::Platform(PlatformResponse::Conversation(Conversation {
                id: value
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                items: Vec::new(),
                metadata: Default::default(),
            }))
        }
        OperationRequest::Platform(PlatformRequest::CreateRealtimeCall(_)) => {
            OperationResponse::Platform(PlatformResponse::RealtimeCall(RealtimeCall {
                id: value
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                answer_sdp: value
                    .get("sdp")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
            }))
        }
        _ => return Err(mode_error()),
    })
}

fn decode_model(v: &Value) -> Model {
    Model {
        id: ModelId(
            v.get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
        ),
        display_name: v
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        description: v
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
        created_at: v.get("created").and_then(Value::as_i64),
        capabilities: Vec::new(),
        context_limit: v.get("context_window").and_then(Value::as_u64),
        output_limit: v.get("max_output_tokens").and_then(Value::as_u64),
    }
}
fn decode_images(v: &Value) -> Result<ImageResponse, CoreError> {
    Ok(ImageResponse {
        images: v
            .get("data")
            .and_then(Value::as_array)
            .unwrap_or(&vec![])
            .iter()
            .enumerate()
            .map(|(i, x)| {
                Ok(ImageArtifact {
                    id: crate::llm::ir::OutputId(i.to_string()),
                    source: if let Some(url) = x.get("url").and_then(Value::as_str) {
                        MediaSource::Url { url: url.into() }
                    } else {
                        MediaSource::Data {
                            media_type: crate::llm::ir::MediaType(format!(
                                "image/{}",
                                v.get("output_format")
                                    .and_then(Value::as_str)
                                    .unwrap_or("png")
                            )),
                            bytes: Bytes::from(
                                base64::engine::general_purpose::STANDARD
                                    .decode(
                                        x.get("b64_json")
                                            .and_then(Value::as_str)
                                            .unwrap_or_default(),
                                    )
                                    .map_err(|e| CoreError::Endpoint(e.to_string()))?,
                            ),
                        }
                    },
                    revised_prompt: x
                        .get("revised_prompt")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
            })
            .collect::<Result<_, CoreError>>()?,
        usage: Some(decode_usage(v.get("usage"))),
    })
}
fn decode_transcription(v: &Value) -> Transcription {
    Transcription {
        text: v
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        language: v.get("language").and_then(Value::as_str).map(str::to_owned),
        duration_seconds: v.get("duration").and_then(Value::as_f64),
        words: v
            .get("words")
            .and_then(Value::as_array)
            .unwrap_or(&vec![])
            .iter()
            .map(|w| TranscriptWord {
                text: w
                    .get("word")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                start_seconds: w.get("start").and_then(Value::as_f64).unwrap_or(0.0),
                end_seconds: w.get("end").and_then(Value::as_f64).unwrap_or(0.0),
                speaker: w.get("speaker").and_then(Value::as_str).map(str::to_owned),
            })
            .collect(),
        segments: decode_segments(v),
        usage: None,
    }
}
fn decode_segments(v: &Value) -> Vec<TranscriptSegment> {
    v.get("segments")
        .and_then(Value::as_array)
        .unwrap_or(&vec![])
        .iter()
        .map(|s| TranscriptSegment {
            id: s
                .get("id")
                .map(Value::to_string)
                .unwrap_or_default()
                .trim_matches('"')
                .into(),
            text: s
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            start_seconds: s.get("start").and_then(Value::as_f64).unwrap_or(0.0),
            end_seconds: s.get("end").and_then(Value::as_f64).unwrap_or(0.0),
            speaker: s.get("speaker").and_then(Value::as_str).map(str::to_owned),
        })
        .collect()
}

fn decode_image_stream(
    request: &ImageRequest,
    target: OperationKey,
    stream: crate::llm::wire::JsonSseStream,
) -> Result<DecodedResponse, CoreError> {
    let expected = match request {
        ImageRequest::Generate(_) => "image_generation",
        ImageRequest::Edit(_) => "image_edit",
    };
    Ok(DecodedResponse::Stream(super::map_sse(
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
                .unwrap_or_default();
            let event = if kind == format!("{expected}.partial_image") {
                let encoded = value_field(&value, "b64_json")?;
                crate::llm::ir::images::ImageEvent::Preview(ImagePreview {
                    index: 0,
                    sequence: u32v(&value, "partial_image_index"),
                    image: ImageArtifact {
                        id: crate::llm::ir::OutputId(
                            u32v(&value, "partial_image_index").to_string(),
                        ),
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
                })
            } else if kind == format!("{expected}.completed") {
                crate::llm::ir::images::ImageEvent::Finished(decode_images(&value)?)
            } else {
                return Err(CoreError::UnmodeledProviderEvent {
                    target,
                    event: kind.into(),
                });
            };
            Ok(vec![OperationEvent::Image(event)])
        },
    )))
}
fn decode_transcription_stream(
    target: OperationKey,
    stream: crate::llm::wire::JsonSseStream,
) -> Result<DecodedResponse, CoreError> {
    Ok(DecodedResponse::Stream(super::map_sse(
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
                .unwrap_or_default();
            let event = match kind {
                "transcript.text.delta" => TranscriptionEvent::TextDelta {
                    text: value_field(&value, "delta")?.into(),
                },
                "transcript.text.segment" => TranscriptionEvent::Segment(decode_segment(&value)),
                "transcript.text.done" => {
                    TranscriptionEvent::Finished(decode_transcription(&value))
                }
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

fn decode_segment(value: &Value) -> TranscriptSegment {
    TranscriptSegment {
        id: value
            .get("id")
            .map(Value::to_string)
            .unwrap_or_default()
            .trim_matches('"')
            .into(),
        text: value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        start_seconds: value.get("start").and_then(Value::as_f64).unwrap_or(0.0),
        end_seconds: value.get("end").and_then(Value::as_f64).unwrap_or(0.0),
        speaker: value
            .get("speaker")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

fn value_field<'a>(value: &'a Value, field: &str) -> Result<&'a str, CoreError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::Endpoint(format!("missing {field}")))
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
    v.and_then(|v| serde_json::to_value(v).ok())
        .and_then(|v| v.as_str().map(str::to_owned))
}
fn decode_usage(v: Option<&Value>) -> crate::llm::ir::Usage {
    let v = v.unwrap_or(&Value::Null);
    crate::llm::ir::Usage {
        input_tokens: v
            .get("prompt_tokens")
            .or_else(|| v.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: v.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
        cached_input_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: v.get("total_tokens").and_then(Value::as_u64).unwrap_or(0),
    }
}
fn u32v(v: &Value, k: &str) -> u32 {
    v.get(k)
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0)
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
) -> Result<(), CoreError> {
    if d.is_empty() {
        Ok(())
    } else {
        Err(CoreError::UnsupportedCapability {
            capability: Capability::TextGeneration,
            target,
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use gproxy_protocol::Operation;

    #[test]
    fn openai_image_edit_uses_replayable_multipart() {
        let request = OperationRequest::Images(ImageRequest::Edit(EditImageRequest {
            model: ModelId("gpt-image-1".into()),
            prompt: "edit".into(),
            images: vec![MediaSource::Data {
                media_type: crate::llm::ir::MediaType("image/png".into()),
                bytes: Bytes::from_static(b"png"),
            }],
            mask: None,
            count: Some(1),
            input_fidelity: None,
            options: ImageOptions::default(),
            mode: ImageMode::Complete,
        }));
        let target = OperationKey::provider(Operation::EditImage, Provider::OpenAi);
        let encoded = encode(&request, target).unwrap();
        let RequestBody::Multipart(body) = encoded.body else {
            panic!("expected multipart body")
        };
        assert!(body.parts.iter().any(|part| part.name == "image[]"));
    }
}
