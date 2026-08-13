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
    let (canonical, body, query, response_mode) = canonical_request(request)?;
    let endpoint = gproxy_protocol::endpoint::request_target(
        target,
        request.model_id().unwrap_or_default(),
        is_stream(request),
    )
    .map_err(|error| CoreError::Endpoint(error.to_string()))?;
    let (body, mut query) = match request {
        OperationRequest::Embeddings(request) if target.provider_family() == Provider::Gemini => {
            (encode_gemini_embedding(request, target)?, Vec::new())
        }
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
        (
            OperationRequest::Audio(AudioRequest::Speech(request)),
            WireResponse::BinaryStream(response),
        ) => decode_speech_stream(request, response),
        (OperationRequest::Audio(AudioRequest::Speech(_)), _) => Err(mode_error()),
        (_, WireResponse::Json(response)) => {
            let body = transform_response(
                target,
                canonical,
                response.body,
                request_capability(request.operation()),
            )?;
            Ok(DecodedResponse::Complete(decode_json(
                request, canonical, &body,
            )?))
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
        OperationRequest::Audio(AudioRequest::Speech(request)) => (
            json_body(
                json!({"model":request.model.0,"input":request.input,"voice":request.voice.0,"instructions":request.instructions,"response_format":enum_string(Some(request.format)),"speed":request.speed}),
            )?,
            Vec::new(),
            match request.mode {
                SpeechMode::Complete => ResponseMode::Binary,
                SpeechMode::Stream => ResponseMode::BinaryStream,
            },
        ),
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
        OperationRequest::Platform(
            PlatformRequest::CreateRealtimeCall(_) | PlatformRequest::ConnectRealtime(_),
        ) => return Err(unsupported(Capability::Realtime, key)),
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

fn encode_gemini_embedding(
    request: &EmbeddingRequest,
    target: OperationKey,
) -> Result<RequestBody, CoreError> {
    let text = match &request.input {
        EmbeddingInput::Text { value } => value,
        EmbeddingInput::TextBatch { .. }
        | EmbeddingInput::Tokens { .. }
        | EmbeddingInput::TokenBatch { .. } => {
            return Err(unsupported(Capability::Embeddings, target))
        }
    };
    let task_type = request.task.map(|task| match task {
        EmbeddingTask::RetrievalQuery => "RETRIEVAL_QUERY",
        EmbeddingTask::RetrievalDocument => "RETRIEVAL_DOCUMENT",
        EmbeddingTask::SemanticSimilarity => "SEMANTIC_SIMILARITY",
        EmbeddingTask::Classification => "CLASSIFICATION",
        EmbeddingTask::Clustering => "CLUSTERING",
        EmbeddingTask::QuestionAnswering => "QUESTION_ANSWERING",
        EmbeddingTask::FactVerification => "FACT_VERIFICATION",
        EmbeddingTask::CodeRetrievalQuery => "CODE_RETRIEVAL_QUERY",
    });
    json_body(json!({
        "model": request.model.0,
        "content": {"parts": [{"text": text}]},
        "taskType": task_type,
        "outputDimensionality": request.dimensions,
    }))
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

fn decode_json(
    request: &OperationRequest,
    target: OperationKey,
    body: &JsonBody,
) -> Result<OperationResponse, CoreError> {
    let value: Value = body.decode()?;
    Ok(match request {
        OperationRequest::Models(ModelRequest::List(_)) => {
            OperationResponse::Models(ModelResponse::List(ModelPage {
                models: array_field(&value, "data", target)?
                    .iter()
                    .map(|model| decode_model(model, target))
                    .collect::<Result<_, _>>()?,
                next_cursor: value
                    .get("next_cursor")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            }))
        }
        OperationRequest::Models(ModelRequest::Get(_)) => {
            OperationResponse::Models(ModelResponse::One(decode_model(&value, target)?))
        }
        OperationRequest::CountTokens(_) => OperationResponse::CountTokens(CountTokensResponse {
            input_tokens: required_u64(&value, "input_tokens", target)?,
        }),
        OperationRequest::Embeddings(_) => OperationResponse::Embeddings(EmbeddingResponse {
            model: value
                .get("model")
                .and_then(Value::as_str)
                .map(|v| ModelId(v.into())),
            vectors: array_field(&value, "data", target)?
                .iter()
                .map(|item| {
                    Ok(EmbeddingVector {
                        index: required_u32(item, "index", target)?,
                        values: array_field(item, "embedding", target)?
                            .iter()
                            .map(|value| {
                                value.as_f64().map(|value| value as f32).ok_or_else(|| {
                                    invalid_payload(target, "embedding value must be numeric")
                                })
                            })
                            .collect::<Result<_, _>>()?,
                    })
                })
                .collect::<Result<_, CoreError>>()?,
            usage: value.get("usage").map(decode_usage),
        }),
        OperationRequest::Images(_) => OperationResponse::Images(decode_images(&value, target)?),
        OperationRequest::Audio(AudioRequest::Transcribe(_)) => OperationResponse::Audio(
            AudioResponse::Transcription(decode_transcription(&value, target)?),
        ),
        OperationRequest::Audio(AudioRequest::Translate(_)) => {
            OperationResponse::Audio(AudioResponse::Translation(Translation {
                text: required_str(&value, "text", target)?.into(),
                source_language: value
                    .get("language")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                duration_seconds: value.get("duration").and_then(Value::as_f64),
                segments: decode_segments(&value, target)?,
            }))
        }
        OperationRequest::Search(SearchRequest::Rerank(_)) => {
            OperationResponse::Search(SearchResponse::Rerank(RerankResponse {
                results: array_field(&value, "results", target)?
                    .iter()
                    .map(|result| {
                        Ok(RerankResult {
                            index: required_u32(result, "index", target)?,
                            relevance_score: required_f64(result, "relevance_score", target)?
                                as f32,
                            document: result
                                .get("document")
                                .map(|document| {
                                    Ok::<_, CoreError>(RerankDocument {
                                        id: document
                                            .get("id")
                                            .and_then(Value::as_str)
                                            .map(str::to_owned),
                                        text: required_str(document, "text", target)?.into(),
                                        title: document
                                            .get("title")
                                            .and_then(Value::as_str)
                                            .map(str::to_owned),
                                    })
                                })
                                .transpose()?,
                        })
                    })
                    .collect::<Result<_, CoreError>>()?,
                usage: value.get("usage").map(decode_usage),
            }))
        }
        OperationRequest::Search(SearchRequest::Web(_)) => {
            let results = value
                .get("data")
                .or_else(|| value.get("results"))
                .and_then(Value::as_array)
                .ok_or_else(|| invalid_payload(target, "missing data/results array"))?;
            OperationResponse::Search(SearchResponse::Web(WebSearchResponse {
                results: results
                    .iter()
                    .map(|result| {
                        Ok(WebSearchResult {
                            url: required_str(result, "url", target)?.into(),
                            title: required_str(result, "title", target)?.into(),
                            snippet: result
                                .get("snippet")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                            published_at: result
                                .get("published_at")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                            score: result
                                .get("score")
                                .and_then(Value::as_f64)
                                .map(|value| value as f32),
                        })
                    })
                    .collect::<Result<_, CoreError>>()?,
                usage: value.get("usage").map(decode_usage),
            }))
        }
        OperationRequest::Platform(PlatformRequest::Compact(_)) => {
            OperationResponse::Platform(PlatformResponse::Compact(decode_compact(&value, target)?))
        }
        OperationRequest::Platform(PlatformRequest::CreateConversation(_)) => {
            OperationResponse::Platform(PlatformResponse::Conversation(Conversation {
                id: required_str(&value, "id", target)?.into(),
                metadata: decode_string_map(value.get("metadata"), target)?,
            }))
        }
        _ => return Err(mode_error()),
    })
}

fn decode_model(value: &Value, target: OperationKey) -> Result<Model, CoreError> {
    Ok(Model {
        id: ModelId(required_str(value, "id", target)?.into()),
        display_name: value
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
        created_at: value.get("created").and_then(Value::as_i64),
        capabilities: None,
        context_limit: value.get("context_window").and_then(Value::as_u64),
        output_limit: value.get("max_output_tokens").and_then(Value::as_u64),
    })
}
fn decode_images(value: &Value, target: OperationKey) -> Result<ImageResponse, CoreError> {
    Ok(ImageResponse {
        images: array_field(value, "data", target)?
            .iter()
            .enumerate()
            .map(|(index, item)| {
                Ok(ImageArtifact {
                    id: crate::llm::ir::OutputId(index.to_string()),
                    source: if let Some(url) = item.get("url").and_then(Value::as_str) {
                        MediaSource::Url { url: url.into() }
                    } else {
                        let encoded = required_str(item, "b64_json", target)?;
                        MediaSource::Data {
                            media_type: crate::llm::ir::MediaType(format!(
                                "image/{}",
                                value
                                    .get("output_format")
                                    .and_then(Value::as_str)
                                    .unwrap_or("png")
                            )),
                            bytes: Bytes::from(
                                base64::engine::general_purpose::STANDARD
                                    .decode(encoded)
                                    .map_err(|error| {
                                        invalid_payload(
                                            target,
                                            &format!("invalid base64 image: {error}"),
                                        )
                                    })?,
                            ),
                        }
                    },
                    revised_prompt: item
                        .get("revised_prompt")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
            })
            .collect::<Result<_, CoreError>>()?,
        usage: value.get("usage").map(decode_usage),
    })
}
fn decode_transcription(value: &Value, target: OperationKey) -> Result<Transcription, CoreError> {
    Ok(Transcription {
        text: required_str(value, "text", target)?.into(),
        language: value
            .get("language")
            .and_then(Value::as_str)
            .map(str::to_owned),
        duration_seconds: value.get("duration").and_then(Value::as_f64),
        words: value
            .get("words")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|word| {
                Ok(TranscriptWord {
                    text: required_str(word, "word", target)?.into(),
                    start_seconds: required_f64(word, "start", target)?,
                    end_seconds: required_f64(word, "end", target)?,
                    speaker: word
                        .get("speaker")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
            })
            .collect::<Result<_, CoreError>>()?,
        segments: decode_segments(value, target)?,
        usage: None,
    })
}
fn decode_segments(
    value: &Value,
    target: OperationKey,
) -> Result<Vec<TranscriptSegment>, CoreError> {
    value
        .get("segments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|segment| decode_segment(segment, target))
        .collect()
}

fn decode_compact(value: &Value, target: OperationKey) -> Result<CompactResponse, CoreError> {
    let output = array_field(value, "output", target)?;
    let mut content = None;
    let mut encrypted_content = None;
    let mut decoded = Vec::with_capacity(output.len());
    for item in output {
        if item.get("type").and_then(Value::as_str) == Some("message") {
            let parts = array_field(item, "content", target)?;
            for part in parts {
                if part.get("type").and_then(Value::as_str) == Some("summary_text") {
                    content = Some(required_str(part, "text", target)?.to_owned());
                }
            }
        }
        let item = super::generation::decode_output_item(item)?;
        if let crate::llm::ir::generation::OutputItem::Compaction(compaction) = &item {
            encrypted_content = Some(compaction.encrypted_content.clone());
        }
        decoded.push(item);
    }
    if let Some(content) = content {
        if let Some(crate::llm::ir::generation::OutputItem::Compaction(compaction)) = decoded
            .iter_mut()
            .find(|item| matches!(item, crate::llm::ir::generation::OutputItem::Compaction(_)))
        {
            compaction.content = Some(content);
        }
    }
    Ok(CompactResponse {
        output: decoded,
        encrypted_content,
        usage: value.get("usage").map(decode_usage),
    })
}

fn decode_string_map(
    value: Option<&Value>,
    target: OperationKey,
) -> Result<std::collections::BTreeMap<String, String>, CoreError> {
    let Some(value) = value else {
        return Ok(Default::default());
    };
    let object = value
        .as_object()
        .ok_or_else(|| invalid_payload(target, "metadata must be an object"))?;
    object
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.to_owned()))
                .ok_or_else(|| invalid_payload(target, "metadata values must be strings"))
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
                let sequence = required_u32(&value, "partial_image_index", target)?;
                crate::llm::ir::images::ImageEvent::Preview(ImagePreview {
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
                })
            } else if kind == format!("{expected}.completed") {
                crate::llm::ir::images::ImageEvent::Finished(decode_images(&value, target)?)
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

fn decode_speech_stream(
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
                "transcript.text.segment" => {
                    TranscriptionEvent::Segment(decode_segment(&value, target)?)
                }
                "transcript.text.done" => {
                    TranscriptionEvent::Finished(decode_transcription(&value, target)?)
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
    v.and_then(|v| serde_json::to_value(v).ok())
        .and_then(|v| v.as_str().map(str::to_owned))
}
fn decode_usage(v: &Value) -> crate::llm::ir::Usage {
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
