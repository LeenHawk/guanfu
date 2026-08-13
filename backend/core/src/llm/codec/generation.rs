use std::collections::BTreeMap;

use base64::Engine;
use bytes::Bytes;
use gproxy_protocol::{ContentGenerationKind, Operation, OperationKey};
use gproxy_transform::{dispatch, resolve, TransformContext};
use http::HeaderMap;
use serde_json::{json, Map, Value};

use super::{DecodedResponse, OperationEvent};
use crate::llm::ir::generation::*;
use crate::llm::ir::{Capability, FileId, MediaSource};
use crate::llm::wire::{
    JsonBody, JsonSseData, QueryParam, RequestBody, ResponseMode, WireRequest, WireResponse,
};
use crate::CoreError;

const CANONICAL_KIND: ContentGenerationKind = ContentGenerationKind::OpenAiResponses;

pub fn encode(request: &GenerateRequest, target: OperationKey) -> Result<WireRequest, CoreError> {
    validate_capabilities(request, target)?;
    let source = OperationKey::content_generation(request.operation(), CANONICAL_KIND);
    let canonical = JsonBody::encode(&encode_request(request)?)?;
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
                crate::llm::ir::OperationResponse::Generate(decode_complete(&body)?),
            ))
        }
        WireResponse::JsonSse(response) if request.mode == GenerateMode::Stream => {
            let mut decoder = StreamDecoder::new(target, canonical)?;
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

fn encode_request(request: &GenerateRequest) -> Result<Value, CoreError> {
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model.0));
    body.insert(
        "input".into(),
        encode_input(&request.instructions, &request.input)?,
    );
    if !request.tools.is_empty() {
        body.insert(
            "tools".into(),
            Value::Array(
                request
                    .tools
                    .iter()
                    .map(encode_tool)
                    .collect::<Result<_, _>>()?,
            ),
        );
    }
    body.insert(
        "tool_choice".into(),
        encode_tool_choice(&request.tool_choice),
    );
    body.insert("text".into(), encode_output_constraint(&request.output));
    insert_option(&mut body, "temperature", request.sampling.temperature);
    insert_option(&mut body, "top_p", request.sampling.top_p);
    insert_option(
        &mut body,
        "max_output_tokens",
        request.limits.max_output_tokens,
    );
    insert_option(&mut body, "max_tool_calls", request.limits.max_tool_calls);
    if !request.modalities.is_empty() {
        body.insert(
            "modalities".into(),
            Value::Array(
                request
                    .modalities
                    .iter()
                    .map(|modality| Value::String(snake(modality)))
                    .collect(),
            ),
        );
    }
    if request.mode == GenerateMode::Stream {
        body.insert("stream".into(), Value::Bool(true));
    }
    Ok(Value::Object(body))
}

pub(super) fn encode_input(
    instructions: &[Instruction],
    input: &[InputItem],
) -> Result<Value, CoreError> {
    let mut values = instructions
        .iter()
        .map(|instruction| {
            Ok(json!({
                "type": "message",
                "role": match instruction.role {
                    InstructionRole::System => "system",
                    InstructionRole::Developer => "developer",
                },
                "content": encode_content(&instruction.content)?,
            }))
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    values.extend(
        input
        .iter()
        .map(|item| match item {
            InputItem::Message { message } => Ok(json!({
                "type": "message",
                "role": match message.role { MessageRole::User => "user", MessageRole::Assistant => "assistant" },
                "content": encode_content(&message.content)?,
            })),
            InputItem::ToolResult { result } => encode_tool_result(result),
            InputItem::Reasoning { reasoning } => Ok(json!({
                "type": "reasoning",
                "id": reasoning.previous.id.0,
                "summary": reasoning.previous.summary.iter().map(|text| json!({"type":"summary_text","text":text})).collect::<Vec<_>>(),
                "encrypted_content": reasoning.previous.encrypted_content,
            })),
        })
        .collect::<Result<Vec<_>, CoreError>>()?,
    );
    Ok(Value::Array(values))
}

fn encode_content(content: &[InputContent]) -> Result<Value, CoreError> {
    Ok(Value::Array(
        content
            .iter()
            .map(|part| match part {
                InputContent::Text { text } => Ok(json!({"type":"input_text","text":text})),
                InputContent::Image { source, detail } => {
                    let (field, value) = encode_media_ref(source)?;
                    Ok(json!({"type":"input_image",field:value,"detail":snake(detail)}))
                }
                InputContent::Audio { source } => {
                    let MediaSource::Data { media_type, bytes } = source else {
                        return Err(CoreError::UnsupportedRouteImplementation { implementation: "Responses audio input requires inline data" });
                    };
                    let format = media_type.0.rsplit('/').next().unwrap_or("wav");
                    Ok(json!({"type":"input_audio","input_audio":{"data":base64::engine::general_purpose::STANDARD.encode(bytes),"format":format}}))
                }
                InputContent::File { source } => match source {
                    FileSource::Id { id } => Ok(json!({"type":"input_file","file_id":id.0})),
                    FileSource::Media { source } => {
                        let (field, value) = encode_file_ref(source)?;
                        Ok(json!({"type":"input_file",field:value}))
                    }
                    FileSource::Text { filename, text } => Ok(json!({"type":"input_file","filename":filename,"file_data":text})),
                },
            })
            .collect::<Result<Vec<_>, CoreError>>()?,
    ))
}

fn encode_media_ref(source: &MediaSource) -> Result<(&'static str, String), CoreError> {
    match source {
        MediaSource::Url { url } => Ok(("image_url", url.clone())),
        MediaSource::File { id: FileId(id) } => Ok(("file_id", id.clone())),
        MediaSource::Data { media_type, bytes } => Ok((
            "image_url",
            format!(
                "data:{};base64,{}",
                media_type.0,
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ),
        )),
    }
}

fn encode_file_ref(source: &MediaSource) -> Result<(&'static str, String), CoreError> {
    match source {
        MediaSource::Url { url } => Ok(("file_url", url.clone())),
        MediaSource::File { id: FileId(id) } => Ok(("file_id", id.clone())),
        MediaSource::Data { media_type, bytes } => Ok((
            "file_data",
            format!(
                "data:{};base64,{}",
                media_type.0,
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ),
        )),
    }
}

fn encode_tool_result(result: &ToolResult) -> Result<Value, CoreError> {
    let output = match &result.outcome {
        ToolOutcome::Success { content } => Value::Array(
            content
                .iter()
                .map(|part| match part {
                    ToolResultContent::Text { text } => {
                        Ok(json!({"type":"input_text","text":text}))
                    }
                    ToolResultContent::Image { source } => {
                        let (field, value) = encode_media_ref(source)?;
                        Ok(json!({"type":"input_image",field:value}))
                    }
                    ToolResultContent::Json { value } => {
                        Ok(json!({"type":"input_text","text":value.to_string()}))
                    }
                })
                .collect::<Result<Vec<_>, CoreError>>()?,
        ),
        ToolOutcome::Error { code, message } => json!({"error":{"code":code,"message":message}}),
    };
    let type_name = match result.kind {
        ToolResultKind::Function => "function_call_output",
        ToolResultKind::Custom => "custom_tool_call_output",
        ToolResultKind::ComputerUse => "computer_call_output",
        ToolResultKind::CodeExecution => "code_interpreter_call_output",
        ToolResultKind::Shell => "shell_call_output",
        ToolResultKind::TextEditor => "apply_patch_call_output",
        ToolResultKind::Mcp => "mcp_approval_response",
        ToolResultKind::Memory => "function_call_output",
        ToolResultKind::ToolSearch => "tool_search_output",
    };
    Ok(json!({"type":type_name,"call_id":result.call_id.0,"output":output}))
}

pub(super) fn encode_tool(tool: &ToolDefinition) -> Result<Value, CoreError> {
    Ok(match tool {
        ToolDefinition::Function(tool) => {
            json!({"type":"function","name":tool.name,"description":tool.description,"parameters":tool.parameters.0,"strict":tool.strict})
        }
        ToolDefinition::Custom(tool) => {
            json!({"type":"custom","name":tool.name,"description":tool.description,"format":match &tool.input_format { CustomToolInputFormat::Text=>Value::Null, CustomToolInputFormat::Grammar{syntax,definition}=>json!({"type":snake(syntax),"definition":definition}) }})
        }
        ToolDefinition::WebSearch(tool) => {
            json!({"type":"web_search","search_context_size":tool.search_context_size.as_ref().map(snake),"user_location":tool.user_location,"filters":{"allowed_domains":tool.allowed_domains,"blocked_domains":tool.blocked_domains}})
        }
        ToolDefinition::WebFetch(tool) => {
            json!({"type":"web_fetch","allowed_domains":tool.allowed_domains,"blocked_domains":tool.blocked_domains,"max_uses":tool.max_uses})
        }
        ToolDefinition::FileSearch(tool) => {
            json!({"type":"file_search","vector_store_ids":tool.vector_store_ids,"max_num_results":tool.max_results,"ranking_options":tool.ranking,"filters":tool.filters})
        }
        ToolDefinition::ComputerUse(tool) => {
            json!({"type":"computer_use_preview","environment":snake(&tool.environment),"display_width":tool.display_width,"display_height":tool.display_height})
        }
        ToolDefinition::CodeExecution(tool) => {
            json!({"type":"code_interpreter","container":tool.container.as_deref().unwrap_or("auto")})
        }
        ToolDefinition::Shell(tool) => json!({"type":"shell","environment":tool.environment}),
        ToolDefinition::TextEditor(_) => json!({"type":"apply_patch"}),
        ToolDefinition::ImageGeneration(tool) => {
            json!({"type":"image_generation","size":tool.size,"quality":tool.quality,"background":tool.background,"output_format":tool.output_format})
        }
        ToolDefinition::Mcp(tool) => {
            json!({"type":"mcp","server_label":tool.server_label,"server_url":tool.server_url,"allowed_tools":tool.allowed_tools,"require_approval":snake(&tool.approval)})
        }
        ToolDefinition::Memory(tool) => {
            json!({"type":"function","name":tool.name,"description":"Persistent memory operation","parameters":{"type":"object"},"strict":false})
        }
        ToolDefinition::ToolSearch(tool) => {
            json!({"type":"tool_search","parameters":{"deferred_tools":tool.deferred_tools}})
        }
    })
}

fn encode_tool_choice(choice: &ToolChoice) -> Value {
    match choice {
        ToolChoice::Auto => Value::String("auto".into()),
        ToolChoice::None => Value::String("none".into()),
        ToolChoice::Required => Value::String("required".into()),
        ToolChoice::Tool { name } => json!({"type":"function","name":name}),
        ToolChoice::Allowed { names, mode } => {
            json!({"type":"allowed_tools","mode":snake(mode),"tools":names.iter().map(|name|json!({"type":"function","name":name})).collect::<Vec<_>>() })
        }
    }
}

fn encode_output_constraint(output: &OutputConstraint) -> Value {
    match output {
        OutputConstraint::Text => json!({"format":{"type":"text"}}),
        OutputConstraint::JsonObject => json!({"format":{"type":"json_object"}}),
        OutputConstraint::JsonSchema {
            name,
            schema,
            strict,
        } => json!({"format":{"type":"json_schema","name":name,"schema":schema.0,"strict":strict}}),
    }
}

fn validate_capabilities(request: &GenerateRequest, target: OperationKey) -> Result<(), CoreError> {
    for (enabled, capability) in [
        (request.sampling.top_k.is_some(), Capability::TopKSampling),
        (request.sampling.seed.is_some(), Capability::SeededSampling),
        (!request.sampling.stop.is_empty(), Capability::StopSequences),
    ] {
        if enabled {
            return Err(CoreError::UnsupportedCapability { capability, target });
        }
    }
    for tool in &request.tools {
        let capability = match tool {
            ToolDefinition::Function(_) => Capability::FunctionTool,
            ToolDefinition::Custom(_) => Capability::CustomTool,
            ToolDefinition::WebSearch(_) => Capability::WebSearchTool,
            ToolDefinition::WebFetch(_) => Capability::WebFetchTool,
            ToolDefinition::FileSearch(_) => Capability::FileSearchTool,
            ToolDefinition::ComputerUse(_) => Capability::ComputerUseTool,
            ToolDefinition::CodeExecution(_) => Capability::CodeExecutionTool,
            ToolDefinition::Shell(_) => Capability::ShellTool,
            ToolDefinition::TextEditor(_) => Capability::TextEditorTool,
            ToolDefinition::ImageGeneration(_) => Capability::ImageGenerationTool,
            ToolDefinition::Mcp(_) => Capability::McpTool,
            ToolDefinition::Memory(_) => Capability::MemoryTool,
            ToolDefinition::ToolSearch(_) => Capability::ToolSearchTool,
        };
        let source = OperationKey::content_generation(request.operation(), CANONICAL_KIND);
        if target != source {
            let probe = GenerateRequest {
                tools: vec![tool.clone()],
                ..request.clone()
            };
            let canonical = JsonBody::encode(&encode_request(&probe)?)?;
            let pair = resolve(source, target).map_err(transform_error)?;
            let ctx = TransformContext::new(source, target);
            let transformed = dispatch::request_bytes_detailed(pair, &ctx, canonical.as_bytes())
                .map_err(transform_error)?;
            if !transformed.diagnostics.is_empty() {
                return Err(CoreError::UnsupportedCapability { capability, target });
            }
        }
    }
    Ok(())
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

fn decode_complete(body: &JsonBody) -> Result<GenerateResponse, CoreError> {
    let value: Value = body.decode()?;
    let id = required_str(&value, "id")?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let output = value
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::Endpoint("Responses output is missing".into()))?
        .iter()
        .map(decode_output_item)
        .collect::<Result<Vec<_>, _>>()?;
    let finish = decode_finish(&value);
    let usage = decode_usage(value.get("usage"));
    Ok(GenerateResponse {
        id: crate::llm::ir::GenerationId(id.into()),
        model: crate::llm::ir::ModelId(model.into()),
        output,
        finish,
        usage,
    })
}

fn decode_output_item(value: &Value) -> Result<OutputItem, CoreError> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message");
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let output_id = crate::llm::ir::OutputId(id.clone());
    Ok(match kind {
        "message" => OutputItem::Message(OutputMessage {
            id: output_id,
            content: value
                .get("content")
                .and_then(Value::as_array)
                .unwrap_or(&vec![])
                .iter()
                .map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("output_text") => Ok(OutputContent::Text {
                        text: part
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .into(),
                        citations: Vec::new(),
                    }),
                    Some("refusal") => Ok(OutputContent::Refusal {
                        text: part
                            .get("refusal")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .into(),
                    }),
                    other => Err(unmodeled(kind_key(other), Operation::GenerateContent)),
                })
                .collect::<Result<_, _>>()?,
        }),
        "reasoning" => OutputItem::Reasoning(ReasoningOutput {
            id: output_id,
            summary: value
                .get("summary")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|part| part.get("text").and_then(Value::as_str).map(str::to_owned))
                .collect(),
            encrypted_content: value
                .get("encrypted_content")
                .and_then(Value::as_str)
                .map(str::to_owned),
            signature: None,
        }),
        "function_call" => OutputItem::ToolCall(ToolCall::Function(FunctionCall {
            id: output_id,
            call_id: crate::llm::ir::ToolCallId(required_str(value, "call_id")?.into()),
            name: required_str(value, "name")?.into(),
            arguments: serde_json::from_str(
                value
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}"),
            )?,
        })),
        "custom_tool_call" => OutputItem::ToolCall(ToolCall::Custom(CustomToolCall {
            id: output_id,
            call_id: crate::llm::ir::ToolCallId(required_str(value, "call_id")?.into()),
            name: required_str(value, "name")?.into(),
            input: value
                .get("input")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
        })),
        "web_search_call" | "file_search_call" => {
            let call = HostedToolCall {
                id: output_id,
                call_id: crate::llm::ir::ToolCallId(id),
                name: kind.trim_end_matches("_call").into(),
                input: value.clone(),
            };
            OutputItem::ToolCall(if kind == "web_search_call" {
                ToolCall::WebSearch(call)
            } else {
                ToolCall::FileSearch(call)
            })
        }
        "computer_call" => OutputItem::ToolCall(ToolCall::ComputerUse(ComputerActionCall {
            id: output_id,
            call_id: crate::llm::ir::ToolCallId(required_str(value, "call_id")?.into()),
            action: value.get("action").cloned().unwrap_or(Value::Null),
        })),
        "code_interpreter_call" => {
            OutputItem::ToolCall(ToolCall::CodeExecution(CodeExecutionCall {
                id: output_id,
                call_id: crate::llm::ir::ToolCallId(id),
                input: value.clone(),
            }))
        }
        "shell_call" | "local_shell_call" => OutputItem::ToolCall(ToolCall::Shell(ShellCall {
            id: output_id,
            call_id: crate::llm::ir::ToolCallId(
                value
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .into(),
            ),
            input: value.clone(),
        })),
        "apply_patch_call" => OutputItem::ToolCall(ToolCall::TextEditor(TextEditorCall {
            id: output_id,
            call_id: crate::llm::ir::ToolCallId(
                value
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .into(),
            ),
            input: value.clone(),
        })),
        "image_generation_call" => {
            OutputItem::ToolCall(ToolCall::ImageGeneration(ImageGenerationCall {
                id: output_id,
                call_id: crate::llm::ir::ToolCallId(id),
                input: value.clone(),
            }))
        }
        "mcp_call" => OutputItem::ToolCall(ToolCall::Mcp(McpCall {
            id: output_id,
            call_id: crate::llm::ir::ToolCallId(id),
            server_label: required_str(value, "server_label")?.into(),
            name: required_str(value, "name")?.into(),
            arguments: serde_json::from_str(
                value
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}"),
            )?,
        })),
        "tool_search_call" => OutputItem::ToolCall(ToolCall::ToolSearch(ToolSearchCall {
            id: output_id,
            call_id: crate::llm::ir::ToolCallId(
                value
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .into(),
            ),
            input: value.clone(),
        })),
        other => return Err(unmodeled(other, Operation::GenerateContent)),
    })
}

fn decode_finish(value: &Value) -> FinishReason {
    match value.get("status").and_then(Value::as_str) {
        Some("completed") => FinishReason::Stop,
        Some("incomplete") => FinishReason::Incomplete,
        Some("failed") => FinishReason::ContentFilter,
        _ => FinishReason::Stop,
    }
}

fn decode_usage(value: Option<&Value>) -> crate::llm::ir::Usage {
    let value = value.unwrap_or(&Value::Null);
    crate::llm::ir::Usage {
        input_tokens: value
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: value
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cached_input_tokens: value
            .pointer("/input_tokens_details/cached_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        reasoning_tokens: value
            .pointer("/output_tokens_details/reasoning_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        total_tokens: value
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}

struct StreamDecoder {
    canonical: OperationKey,
    converter: Option<gproxy_transform::dispatch::StreamConverter>,
}

impl StreamDecoder {
    fn new(target: OperationKey, canonical: OperationKey) -> Result<Self, CoreError> {
        let converter = if target == canonical {
            None
        } else {
            let pair = resolve(target, canonical).map_err(transform_error)?;
            Some(
                gproxy_transform::dispatch::StreamConverter::new(
                    pair,
                    TransformContext::new(target, canonical),
                )
                .map_err(transform_error)?,
            )
        };
        Ok(Self {
            canonical,
            converter,
        })
    }

    fn push(
        &mut self,
        frame: crate::llm::wire::JsonSseFrame,
    ) -> Result<Vec<OperationEvent>, CoreError> {
        let values = match (frame.data, self.converter.as_mut()) {
            (JsonSseData::Done, None) => Vec::new(),
            (JsonSseData::Done, Some(converter)) => converter
                .finish_detailed()
                .map_err(transform_error)
                .and_then(strict_stream_output)?,
            (JsonSseData::Json(body), None) => vec![body.decode::<Value>()?],
            (JsonSseData::Json(body), Some(converter)) => converter
                .push_detailed(
                    std::str::from_utf8(body.as_bytes())
                        .map_err(|error| CoreError::Transform(error.to_string()))?,
                )
                .map_err(transform_error)
                .and_then(strict_stream_output)?,
        };
        values
            .into_iter()
            .filter_map(|value| match decode_stream_event(&value, self.canonical) {
                Ok(Some(event)) => Some(Ok(OperationEvent::Generate(event))),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }
}

fn strict_stream_output(
    output: gproxy_transform::TransformOutput<Vec<gproxy_transform::dispatch::StreamEventOut>>,
) -> Result<Vec<Value>, CoreError> {
    if !output.diagnostics.is_empty() {
        return Err(CoreError::Transform(format!(
            "semantic loss while decoding stream: {:?}",
            output.diagnostics
        )));
    }
    output
        .value
        .into_iter()
        .map(|event| match event {
            gproxy_transform::dispatch::StreamEventOut::Responses(event) => {
                serde_json::to_value(event).map_err(CoreError::from)
            }
            gproxy_transform::dispatch::StreamEventOut::Encoded { data, .. } => {
                serde_json::from_str(&data).map_err(CoreError::from)
            }
        })
        .collect()
}

fn decode_stream_event(
    value: &Value,
    target: OperationKey,
) -> Result<Option<GenerateEvent>, CoreError> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok(Some(match kind {
        "response.created" => GenerateEvent::Started(GenerationStarted {
            id: crate::llm::ir::GenerationId(
                value
                    .pointer("/response/id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
            ),
            model: crate::llm::ir::ModelId(
                value
                    .pointer("/response/model")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
            ),
        }),
        "response.output_item.added" => GenerateEvent::OutputStarted(OutputStarted {
            output_index: u32_field(value, "output_index"),
            output_id: crate::llm::ir::OutputId(
                value
                    .pointer("/item/id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
            ),
            kind: output_kind(value.pointer("/item/type").and_then(Value::as_str))?,
        }),
        "response.content_part.added" => GenerateEvent::ContentStarted(ContentStarted {
            output_index: u32_field(value, "output_index"),
            content_index: u32_field(value, "content_index"),
            content_id: crate::llm::ir::ContentId(format!(
                "{}:{}",
                u32_field(value, "output_index"),
                u32_field(value, "content_index")
            )),
            kind: content_kind(value.pointer("/part/type").and_then(Value::as_str))?,
        }),
        "response.output_text.delta" => {
            GenerateEvent::Delta(GenerateDelta::Text(content_delta(value)))
        }
        "response.refusal.delta" => {
            GenerateEvent::Delta(GenerateDelta::Refusal(content_delta(value)))
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            GenerateEvent::Delta(GenerateDelta::ReasoningText(content_delta(value)))
        }
        "response.function_call_arguments.delta" => {
            GenerateEvent::Delta(GenerateDelta::FunctionArguments(JsonFragmentDelta {
                output_index: u32_field(value, "output_index"),
                delta: string_field(value, "delta"),
            }))
        }
        "response.custom_tool_call_input.delta" => {
            GenerateEvent::Delta(GenerateDelta::CustomToolInput(OutputTextDelta {
                output_index: u32_field(value, "output_index"),
                delta: string_field(value, "delta"),
            }))
        }
        "response.audio.delta" => GenerateEvent::Delta(GenerateDelta::Audio(BinaryDelta {
            output_index: 0,
            content_index: 0,
            encoded: string_field(value, "delta"),
        })),
        "response.audio.transcript.delta" => {
            GenerateEvent::Delta(GenerateDelta::Transcript(ContentTextDelta {
                output_index: 0,
                content_index: 0,
                delta: string_field(value, "delta"),
            }))
        }
        "response.image_generation_call.partial_image" => {
            GenerateEvent::Delta(GenerateDelta::Image(ImageDelta {
                output_index: u32_field(value, "output_index"),
                content_index: 0,
                encoded: string_field(value, "partial_image_b64"),
                sequence: u32_field(value, "partial_image_index"),
            }))
        }
        "response.output_text.annotation.added" => {
            GenerateEvent::Delta(GenerateDelta::Citation(CitationDelta {
                output_index: u32_field(value, "output_index"),
                content_index: u32_field(value, "content_index"),
                citation: decode_citation(value.get("annotation").unwrap_or(&Value::Null))?,
            }))
        }
        "response.file_search_call.in_progress"
        | "response.web_search_call.in_progress"
        | "response.code_interpreter_call.in_progress"
        | "response.mcp_call.in_progress"
        | "response.mcp_list_tools.in_progress"
        | "response.image_generation_call.in_progress"
        | "response.file_search_call.searching"
        | "response.web_search_call.searching"
        | "response.code_interpreter_call.interpreting"
        | "response.image_generation_call.generating" => {
            tool_execution_delta(value, ToolExecutionState::Running)
        }
        "response.file_search_call.completed"
        | "response.web_search_call.completed"
        | "response.code_interpreter_call.completed"
        | "response.mcp_call.completed"
        | "response.mcp_list_tools.completed"
        | "response.image_generation_call.completed" => {
            tool_execution_delta(value, ToolExecutionState::Completed)
        }
        "response.mcp_call.failed" | "response.mcp_list_tools.failed" => {
            tool_execution_delta(value, ToolExecutionState::Failed)
        }
        "response.code_interpreter_call_code.delta" => {
            GenerateEvent::Delta(GenerateDelta::CustomToolInput(OutputTextDelta {
                output_index: u32_field(value, "output_index"),
                delta: string_field(value, "delta"),
            }))
        }
        "response.mcp_call_arguments.delta" => {
            GenerateEvent::Delta(GenerateDelta::FunctionArguments(JsonFragmentDelta {
                output_index: u32_field(value, "output_index"),
                delta: string_field(value, "delta"),
            }))
        }
        "response.content_part.done" => GenerateEvent::ContentFinished(ContentFinished {
            output_index: u32_field(value, "output_index"),
            content_index: u32_field(value, "content_index"),
            content_id: crate::llm::ir::ContentId(format!(
                "{}:{}",
                u32_field(value, "output_index"),
                u32_field(value, "content_index")
            )),
        }),
        "response.output_item.done" => GenerateEvent::OutputFinished(OutputFinished {
            output_index: u32_field(value, "output_index"),
            item: decode_output_item(value.get("item").unwrap_or(&Value::Null))?,
        }),
        "response.completed" => GenerateEvent::Finished(GenerationFinished {
            finish: FinishReason::Stop,
            usage: decode_usage(value.pointer("/response/usage")),
        }),
        "response.incomplete" => GenerateEvent::Finished(GenerationFinished {
            finish: FinishReason::Incomplete,
            usage: decode_usage(value.pointer("/response/usage")),
        }),
        "response.failed" | "error" => GenerateEvent::Failed(GenerationFailure {
            error: crate::llm::ir::OperationFailure {
                code: value
                    .pointer("/response/error/code")
                    .or_else(|| value.get("code"))
                    .and_then(Value::as_str)
                    .unwrap_or("generation_failed")
                    .into(),
                message: value
                    .pointer("/response/error/message")
                    .or_else(|| value.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("generation failed")
                    .into(),
                retryable: false,
                details: BTreeMap::new(),
            },
            usage: None,
        }),
        "response.in_progress"
        | "response.queued"
        | "response.output_text.done"
        | "response.refusal.done"
        | "response.reasoning_summary_part.added"
        | "response.reasoning_summary_part.done"
        | "response.reasoning_summary_text.done"
        | "response.reasoning_text.done"
        | "response.function_call_arguments.done"
        | "response.custom_tool_call_input.done"
        | "response.audio.done"
        | "response.audio.transcript.done"
        | "response.code_interpreter_call_code.done"
        | "response.mcp_call_arguments.done" => return Ok(None),
        other => {
            return Err(CoreError::UnmodeledProviderEvent {
                target,
                event: other.into(),
            })
        }
    }))
}

fn tool_execution_delta(value: &Value, state: ToolExecutionState) -> GenerateEvent {
    GenerateEvent::Delta(GenerateDelta::ToolExecution(ToolExecutionDelta {
        output_index: u32_field(value, "output_index"),
        output_id: crate::llm::ir::OutputId(
            value
                .get("item_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        ),
        state,
    }))
}

fn decode_citation(value: &Value) -> Result<Citation, CoreError> {
    let source = match value.get("type").and_then(Value::as_str) {
        Some("url_citation") => CitationSource::Url {
            url: required_str(value, "url")?.to_owned(),
        },
        Some("file_citation") => CitationSource::File {
            file_id: required_str(value, "file_id")?.to_owned(),
        },
        Some("file_path") | Some("container_file_citation") => CitationSource::Document {
            document_id: value
                .get("file_id")
                .or_else(|| value.get("container_id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        },
        other => return Err(unmodeled(kind_key(other), Operation::StreamGenerateContent)),
    };
    Ok(Citation {
        start: value
            .get("start_index")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        end: value.get("end_index").and_then(Value::as_u64).unwrap_or(0),
        source,
        title: value
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn output_kind(kind: Option<&str>) -> Result<OutputKind, CoreError> {
    Ok(match kind.unwrap_or_default() {
        "message" => OutputKind::Message,
        "reasoning" => OutputKind::Reasoning,
        "image_generation_call" => OutputKind::Image,
        "audio" => OutputKind::Audio,
        value if value.ends_with("call") => OutputKind::ToolCall,
        other => return Err(unmodeled(other, Operation::StreamGenerateContent)),
    })
}
fn content_kind(kind: Option<&str>) -> Result<ContentKind, CoreError> {
    Ok(match kind.unwrap_or_default() {
        "output_text" => ContentKind::Text,
        "refusal" => ContentKind::Refusal,
        "reasoning_text" => ContentKind::ReasoningText,
        "audio" => ContentKind::Audio,
        "transcript" => ContentKind::Transcript,
        "image" => ContentKind::Image,
        other => return Err(unmodeled(other, Operation::StreamGenerateContent)),
    })
}
fn content_delta(value: &Value) -> ContentTextDelta {
    ContentTextDelta {
        output_index: u32_field(value, "output_index"),
        content_index: u32_field(value, "content_index"),
        delta: string_field(value, "delta"),
    }
}
fn u32_field(value: &Value, key: &str) -> u32 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0)
}
fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .into()
}
fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, CoreError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::Endpoint(format!("missing {key}")))
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
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use futures_util::{stream, StreamExt};
    use http::{HeaderMap, StatusCode};

    use super::*;
    use crate::llm::ir::OperationResponse;
    use crate::llm::wire::{parse_json_sse, JsonResponse, JsonSseResponse, ResponseMetadata};

    fn request(mode: GenerateMode) -> GenerateRequest {
        GenerateRequest {
            model: crate::llm::ir::ModelId("claude-test".into()),
            input: vec![InputItem::Message {
                message: Message {
                    role: MessageRole::User,
                    content: vec![InputContent::Text { text: "hi".into() }],
                },
            }],
            instructions: Vec::new(),
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            output: OutputConstraint::Text,
            sampling: SamplingOptions::default(),
            limits: GenerationLimits {
                max_output_tokens: Some(32),
                max_tool_calls: None,
            },
            modalities: vec![OutputModality::Text],
            mode,
        }
    }

    fn metadata() -> ResponseMetadata {
        ResponseMetadata {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
        }
    }

    #[test]
    fn encodes_semantic_request_as_claude_messages() {
        let target = OperationKey::content_generation(
            Operation::GenerateContent,
            ContentGenerationKind::ClaudeMessages,
        );
        let encoded = encode(&request(GenerateMode::Complete), target).unwrap();
        let RequestBody::Json(body) = encoded.body else {
            panic!("expected JSON body")
        };
        let value: Value = body.decode().unwrap();
        assert_eq!(value["model"], "claude-test");
        assert_eq!(value["messages"][0]["content"][0]["text"], "hi");
    }

    #[test]
    fn decodes_claude_complete_response_into_semantic_output() {
        let target = OperationKey::content_generation(
            Operation::GenerateContent,
            ContentGenerationKind::ClaudeMessages,
        );
        let body = JsonBody::encode(&json!({
            "id":"msg_1","type":"message","role":"assistant","model":"claude-test",
            "content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn",
            "usage":{"input_tokens":2,"output_tokens":1}
        }))
        .unwrap();
        let decoded = decode(
            &request(GenerateMode::Complete),
            target,
            WireResponse::Json(JsonResponse {
                metadata: metadata(),
                body,
            }),
        )
        .unwrap();
        let DecodedResponse::Complete(OperationResponse::Generate(response)) = decoded else {
            panic!("expected complete generation response")
        };
        assert!(matches!(response.output[0], OutputItem::Message(_)));
    }

    #[tokio::test]
    async fn decodes_claude_sse_into_semantic_text_delta() {
        let target = OperationKey::content_generation(
            Operation::StreamGenerateContent,
            ContentGenerationKind::ClaudeMessages,
        );
        let bytes = Bytes::from_static(
            b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-test\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        let decoded = decode(
            &request(GenerateMode::Stream),
            target,
            WireResponse::JsonSse(JsonSseResponse {
                metadata: metadata(),
                stream: parse_json_sse(Box::pin(stream::iter([Ok(bytes)]))),
            }),
        )
        .unwrap();
        let DecodedResponse::Stream(stream) = decoded else {
            panic!("expected stream")
        };
        let events = stream.collect::<Vec<_>>().await;
        assert!(events.iter().any(|event| matches!(
            event,
            Ok(OperationEvent::Generate(GenerateEvent::Delta(GenerateDelta::Text(
                ContentTextDelta { delta, .. }
            )))) if delta == "hello"
        )));
    }
}
