use super::*;

pub(super) fn decode_json(
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
            usage: value
                .get("usage")
                .map(|usage| decode_usage(usage, target))
                .transpose()?,
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
                usage: value
                    .get("usage")
                    .map(|usage| decode_usage(usage, target))
                    .transpose()?,
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
                usage: value
                    .get("usage")
                    .map(|usage| decode_usage(usage, target))
                    .transpose()?,
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
pub(super) fn decode_images(
    value: &Value,
    target: OperationKey,
) -> Result<ImageResponse, CoreError> {
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
        usage: value
            .get("usage")
            .map(|usage| decode_usage(usage, target))
            .transpose()?,
    })
}
pub(super) fn decode_transcription(
    value: &Value,
    target: OperationKey,
) -> Result<Transcription, CoreError> {
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
        let item = crate::llm::codec::generation::response::decode_output_item(item, target)?;
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
        usage: value
            .get("usage")
            .map(|usage| decode_usage(usage, target))
            .transpose()?,
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
