use super::*;

pub(super) fn canonical_key(request: &OperationRequest) -> Result<OperationKey, CoreError> {
    let provider = match request {
        OperationRequest::Embeddings(EmbeddingRequest {
            input: EmbeddingInput::TextBatch { .. } | EmbeddingInput::TokenBatch { .. },
            ..
        }) => Provider::OpenAi,
        _ => Provider::OpenAi,
    };
    Ok(OperationKey::provider(request.operation(), provider))
}

pub(super) fn canonical_request(
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
                    crate::llm::codec::generation::request::encode_input(
                        &input.instructions,
                        &input.input,
                        key,
                    )?
                }
            };
            let tools = match &request.input {
                TokenCountInput::Generation(input) if !input.tools.is_empty() => {
                    Some(Value::Array(
                        input
                            .tools
                            .iter()
                            .map(crate::llm::codec::generation::request::encode_tool)
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
        OperationRequest::Video(VideoRequest::Create(request)) => (
            json_body(json!({
                "model": request.model.0,
                "prompt": request.prompt,
                "seconds": request.seconds.map(|value| value.to_string()),
                "size": request.size,
                "input_reference": request
                    .input_reference
                    .as_ref()
                    .map(video_input_reference)
                    .transpose()?,
            }))?,
            Vec::new(),
            ResponseMode::Json,
        ),
        OperationRequest::Video(VideoRequest::List(request)) => {
            let query = [
                request.limit.map(|value| QueryParam {
                    name: "limit".into(),
                    value: value.to_string(),
                }),
                request.cursor.as_ref().map(|value| QueryParam {
                    name: "after".into(),
                    value: value.clone(),
                }),
            ]
            .into_iter()
            .flatten()
            .collect();
            (RequestBody::Empty, query, ResponseMode::Json)
        }
        OperationRequest::Video(VideoRequest::Retrieve(_) | VideoRequest::Delete(_)) => {
            (RequestBody::Empty, Vec::new(), ResponseMode::Json)
        }
        OperationRequest::Video(VideoRequest::DownloadContent(_)) => {
            (RequestBody::Empty, Vec::new(), ResponseMode::Binary)
        }
        OperationRequest::Platform(PlatformRequest::Compact(request)) => (
            json_body(
                json!({"model":request.model.0,"input":crate::llm::codec::generation::request::encode_input(&request.instructions,&request.input,key)?,"max_output_tokens":request.max_output_tokens}),
            )?,
            Vec::new(),
            ResponseMode::Json,
        ),
        OperationRequest::Platform(PlatformRequest::CreateConversation(request)) => (
            json_body(
                json!({"items":crate::llm::codec::generation::request::encode_input(&[],&request.items,key)?,"metadata":request.metadata}),
            )?,
            Vec::new(),
            ResponseMode::Json,
        ),
        OperationRequest::Platform(PlatformRequest::CreateRealtimeCall(request)) => {
            let session = serde_json::to_string(&crate::llm::codec::realtime::encode_session(
                &request.session,
            )?)?;
            (
                RequestBody::Multipart(MultipartBody {
                    parts: vec![
                        MultipartPart {
                            name: "sdp".into(),
                            value: MultipartValue::Text(request.offer_sdp.clone()),
                        },
                        MultipartPart {
                            name: "session".into(),
                            value: MultipartValue::Text(session),
                        },
                    ],
                }),
                Vec::new(),
                ResponseMode::Binary,
            )
        }
        // WebSocket 连接不走 HTTP codec,service 层在此之前拦截。
        OperationRequest::Platform(PlatformRequest::ConnectRealtime(_)) => {
            return Err(unsupported(Capability::Realtime, key))
        }
        OperationRequest::Generate(_) => unreachable!("generation codec handles generation"),
    };
    Ok((key, result.0, result.1, result.2))
}

pub(super) fn prepare_body(
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

pub(super) fn encode_gemini_embedding(
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
    push_opt(
        &mut parts,
        "background",
        enum_string(request.options.background),
    );
    push_opt(
        &mut parts,
        "output_compression",
        request.options.compression,
    );
    push_opt(
        &mut parts,
        "moderation",
        enum_string(request.options.moderation),
    );
    push_opt(&mut parts, "partial_images", request.options.partial_images);
    if request.mode == ImageMode::Stream {
        parts.push(text_part("stream", "true"));
    }
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
            &enum_string(Some(*value)).expect("timestamp granularity serializes as a string"),
        ));
    }
    if let Some(diarization) = &request.diarization {
        for speaker in &diarization.known_speakers {
            parts.push(text_part("known_speaker_names[]", &speaker.name));
            parts.push(text_part(
                "known_speaker_references[]",
                &media_json(&speaker.reference)?,
            ));
        }
        if let Some(chunking) = &diarization.chunking {
            parts.push(text_part(
                "chunking_strategy",
                &chunking_strategy(chunking)?,
            ));
        }
    }
    let response_format = if request.diarization.is_some() {
        Some("diarized_json")
    } else if !request.timestamps.is_empty() {
        Some("verbose_json")
    } else {
        None
    };
    push_opt(&mut parts, "response_format", response_format);
    Ok(RequestBody::Multipart(MultipartBody { parts }))
}

fn chunking_strategy(chunking: &AudioChunking) -> Result<String, CoreError> {
    Ok(match chunking {
        AudioChunking::Auto => "auto".into(),
        AudioChunking::ServerVad {
            threshold,
            prefix_padding_ms,
            silence_duration_ms,
        } => {
            let mut object = serde_json::Map::new();
            object.insert("type".into(), json!("server_vad"));
            if let Some(value) = threshold {
                object.insert("threshold".into(), json!(value));
            }
            if let Some(value) = prefix_padding_ms {
                object.insert("prefix_padding_ms".into(), json!(value));
            }
            if let Some(value) = silence_duration_ms {
                object.insert("silence_duration_ms".into(), json!(value));
            }
            serde_json::to_string(&Value::Object(object))?
        }
    })
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

/// 参考图编码为 canonical OpenAI 形态:内联数据 → data URL,URL → image_url。
fn video_input_reference(source: &MediaSource) -> Result<Value, CoreError> {
    Ok(match source {
        MediaSource::Data { media_type, bytes } => Value::String(format!(
            "data:{};base64,{}",
            media_type.0,
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )),
        MediaSource::Url { url } => json!({ "image_url": url }),
        MediaSource::File { id } => json!({ "file_id": id.0 }),
    })
}
