//! IR → Gemini generateContent 原生编码(typed wire,直达,不经 canonical 两跳)。

use base64::Engine;
use gproxy_protocol::gemini as wire;
use gproxy_protocol::{ContentGenerationKind, OperationKey};
use serde_json::Value;

use super::claude::{incompatible, wire_build_error};
use super::options;
use crate::llm::ir::generation::*;
use crate::llm::ir::MediaSource;
use crate::CoreError;

pub(super) fn encode_request(
    request: &GenerateRequest,
    target: OperationKey,
) -> Result<Value, CoreError> {
    // Gemini 无 reasoning summary 语义(与 options.rs 原 Gemini 分支一致)。
    if request
        .reasoning
        .as_ref()
        .is_some_and(|reasoning| reasoning.summary.is_some())
    {
        return Err(incompatible(target, "reasoning.summary"));
    }
    let tools = super::gemini_tools::encode_tools(&request.tools, target)?;
    let tool_config = (!tools.is_empty())
        .then(|| super::gemini_tools::encode_tool_choice(&request.tool_choice))
        .transpose()?;
    let body = wire::GenerateContentRequest::builder()
        .model(Some(request.model.0.clone()))
        .contents(encode_contents(request, target)?)
        .tools(tools)
        .tool_config(tool_config)
        .system_instruction(encode_system(&request.instructions, target)?)
        .generation_config(Some(encode_generation_config(request)?))
        .build()
        .map_err(wire_build_error)?;
    let mut value = serde_json::to_value(&body)?;
    let map = value
        .as_object_mut()
        .expect("gemini request serializes as an object");
    options::apply_protocol_options(
        &request.protocol_options,
        ContentGenerationKind::GeminiGenerateContent,
        map,
    );
    Ok(value)
}

/// System 与 Developer instructions 都并入顶层 `systemInstruction`。
fn encode_system(
    instructions: &[Instruction],
    target: OperationKey,
) -> Result<Option<wire::Content>, CoreError> {
    let mut parts = Vec::new();
    for instruction in instructions {
        for part in &instruction.content {
            let InputContent::Text { text } = part else {
                return Err(incompatible(target, "instructions[].content"));
            };
            parts.push(text_part(text.clone())?);
        }
    }
    if parts.is_empty() {
        return Ok(None);
    }
    Ok(Some(content(wire::ContentRoleKnown::System, parts)?))
}

fn encode_contents(
    request: &GenerateRequest,
    target: OperationKey,
) -> Result<Vec<wire::Content>, CoreError> {
    let mut contents = Vec::new();
    for item in &request.input {
        let (role, parts) = match item {
            InputItem::Message { message } => {
                let role = match message.role {
                    MessageRole::User => wire::ContentRoleKnown::User,
                    MessageRole::Assistant => wire::ContentRoleKnown::Model,
                    // 历史 system 消息保留 system 角色进 contents(与 gproxy 一致)。
                    MessageRole::System => wire::ContentRoleKnown::System,
                };
                (role, encode_message_content(&message.content)?)
            }
            InputItem::ToolResult { result } => (
                wire::ContentRoleKnown::User,
                vec![function_response_part(result, target)?],
            ),
            // Gemini 无 MCP 审批应答语义,与 gproxy transform 一致丢弃。
            InputItem::McpApproval { .. } => continue,
            InputItem::Reasoning { reasoning } => {
                (wire::ContentRoleKnown::Model, reasoning_parts(reasoning)?)
            }
        };
        if parts.is_empty() {
            continue;
        }
        contents.push(content(role, parts)?);
    }
    Ok(contents)
}

/// image detail 无 Gemini 落点,静默丢弃(与 gproxy 一致)。
fn encode_message_content(content: &[InputContent]) -> Result<Vec<wire::Part>, CoreError> {
    let mut parts = Vec::new();
    for item in content {
        match item {
            InputContent::Text { text } if text.is_empty() => {}
            InputContent::Text { text } => parts.push(text_part(text.clone())?),
            InputContent::Image { source, .. } | InputContent::Audio { source } => {
                parts.push(media_part(source)?);
            }
            InputContent::File { source } => parts.push(file_part(source)?),
        }
    }
    Ok(parts)
}

fn file_part(source: &FileSource) -> Result<wire::Part, CoreError> {
    match source {
        FileSource::Media { source } => media_part(source),
        FileSource::Id { id } => file_data_part(id.0.clone()),
        // 文本文件内联为 text/plain;filename 无落点丢弃。
        FileSource::Text { text, .. } => inline_part("text/plain".into(), text.as_bytes()),
    }
}

fn media_part(source: &MediaSource) -> Result<wire::Part, CoreError> {
    match source {
        MediaSource::Url { url } => file_data_part(url.clone()),
        MediaSource::File { id } => file_data_part(id.0.clone()),
        MediaSource::Data { media_type, bytes } => inline_part(media_type.0.clone(), bytes),
    }
}

fn file_data_part(file_uri: String) -> Result<wire::Part, CoreError> {
    data_part(wire::PartData::FileData {
        file_data: wire::FileData::builder()
            .file_uri(file_uri)
            .build()
            .map_err(wire_build_error)?,
    })
}

fn inline_part(mime_type: String, bytes: &[u8]) -> Result<wire::Part, CoreError> {
    data_part(wire::PartData::InlineData {
        inline_data: wire::Blob::builder()
            .mime_type(mime_type)
            .data(base64::engine::general_purpose::STANDARD.encode(bytes))
            .build()
            .map_err(wire_build_error)?,
    })
}

/// 工具结果 → functionResponse。IR 不回放 assistant function call,name 以
/// call_id 兜底(与 gproxy 无 name 关联时的现行为一致);文本/JSON 合并进
/// `response.output`,内联图片走 functionResponse parts。
fn function_response_part(
    result: &ToolResult,
    target: OperationKey,
) -> Result<wire::Part, CoreError> {
    let mut response = wire::JsonMap::new();
    let mut blobs = Vec::new();
    match &result.outcome {
        ToolOutcome::Success { content } => {
            let mut texts = Vec::new();
            for part in content {
                match part {
                    ToolResultContent::Text { text } => texts.push(text.clone()),
                    ToolResultContent::Json { value } => texts.push(value.to_string()),
                    ToolResultContent::Image {
                        source: MediaSource::Data { media_type, bytes },
                    } => blobs.push(function_response_blob(media_type.0.clone(), bytes)?),
                    ToolResultContent::Image { .. } => {
                        return Err(incompatible(target, "input[].tool_result.content.image"));
                    }
                }
            }
            response.insert("output".into(), Value::String(texts.join("\n")));
        }
        ToolOutcome::Error { code, message } => {
            let text = match code {
                Some(code) => format!("{code}: {message}"),
                None => message.clone(),
            };
            response.insert("error".into(), Value::String(text));
        }
    }
    let function_response = wire::FunctionResponse::builder()
        .id(Some(result.call_id.0.clone()))
        .name(result.call_id.0.clone())
        .response(response)
        .parts(blobs)
        .build()
        .map_err(wire_build_error)?;
    data_part(wire::PartData::FunctionResponse { function_response })
}

fn function_response_blob(
    mime_type: String,
    bytes: &[u8],
) -> Result<wire::FunctionResponsePart, CoreError> {
    wire::FunctionResponsePart::builder()
        .data(Some(wire::FunctionResponsePartData::InlineData {
            inline_data: wire::FunctionResponseBlob::builder()
                .mime_type(mime_type)
                .data(base64::engine::general_purpose::STANDARD.encode(bytes))
                .build()
                .map_err(wire_build_error)?,
        }))
        .build()
        .map_err(wire_build_error)
}

/// Reasoning 回放:文本(正文在前、summary 在后)合并为单个 thought part,
/// 签名走 `thoughtSignature` 直达(与 gproxy transform 一致)。
fn reasoning_parts(reasoning: &ReasoningInput) -> Result<Vec<wire::Part>, CoreError> {
    let mut text = String::new();
    let mut summaries = String::new();
    let mut continuation = None;
    for part in &reasoning.previous.parts {
        match part {
            ReasoningPart::Summary { text: summary } => summaries.push_str(summary),
            ReasoningPart::Text {
                text: chunk,
                continuation: part_continuation,
            } => {
                text.push_str(chunk);
                continuation = continuation.or(part_continuation.as_ref());
            }
            ReasoningPart::Opaque {
                continuation: part_continuation,
            } => continuation = continuation.or(Some(part_continuation)),
        }
    }
    text.push_str(&summaries);
    if text.is_empty() && continuation.is_none() {
        return Ok(Vec::new());
    }
    let mut builder = wire::Part::builder()
        .thought(Some(true))
        .thought_signature(continuation.map(|value| value.opaque_value().to_owned()));
    if !text.is_empty() {
        builder = builder.data(Some(wire::PartData::Text { text }));
    }
    Ok(vec![builder.build().map_err(wire_build_error)?])
}

fn encode_generation_config(
    request: &GenerateRequest,
) -> Result<wire::GenerationConfig, CoreError> {
    let sampling = &request.sampling;
    // JsonSchema 的 name/strict 无落点丢弃;schema 走 responseJsonSchema
    // (与 gproxy 一致,不用 OpenAPI 子集的 responseSchema)。
    let (response_mime_type, response_json_schema) = match &request.output {
        OutputConstraint::Text => (wire::ResponseMimeTypeKnown::TextPlain, None),
        OutputConstraint::JsonObject => (wire::ResponseMimeTypeKnown::ApplicationJson, None),
        OutputConstraint::JsonSchema { schema, .. } => (
            wire::ResponseMimeTypeKnown::ApplicationJson,
            Some(schema.0.clone()),
        ),
    };
    wire::GenerationConfig::builder()
        .stop_sequences(sampling.stop.clone())
        .response_mime_type(Some(wire::ResponseMimeType::Known(response_mime_type)))
        .response_json_schema(response_json_schema)
        .response_modalities(request.modalities.iter().map(modality).collect())
        .max_output_tokens(request.limits.max_output_tokens.map(u64_to_i32))
        .temperature(sampling.temperature.map(f64::from))
        .top_p(sampling.top_p.map(f64::from))
        .top_k(sampling.top_k.map(u32_to_i32))
        .seed(sampling.seed)
        .presence_penalty(sampling.presence_penalty.map(f64::from))
        .frequency_penalty(sampling.frequency_penalty.map(f64::from))
        .thinking_config(encode_thinking(request.reasoning.as_ref()))
        .build()
        .map_err(wire_build_error)
}

fn modality(modality: &OutputModality) -> wire::ResponseModality {
    wire::ResponseModality::Known(match modality {
        OutputModality::Text => wire::ResponseModalityKnown::Text,
        OutputModality::Audio => wire::ResponseModalityKnown::Audio,
        OutputModality::Image => wire::ResponseModalityKnown::Image,
    })
}

/// budget → thinkingBudget 直写;effort → thinkingLevel(档位映射与 gproxy
/// openai_reasoning_to_gemini 一致,None/Minimal → MINIMAL)。
fn encode_thinking(reasoning: Option<&ReasoningOptions>) -> Option<wire::ThinkingConfig> {
    let reasoning = reasoning?;
    let level = reasoning.effort.map(|effort| {
        wire::ThinkingLevel::Known(match effort {
            ReasoningEffort::None | ReasoningEffort::Minimal => wire::ThinkingLevelKnown::Minimal,
            ReasoningEffort::Low => wire::ThinkingLevelKnown::Low,
            ReasoningEffort::Medium => wire::ThinkingLevelKnown::Medium,
            ReasoningEffort::High | ReasoningEffort::XHigh | ReasoningEffort::Max => {
                wire::ThinkingLevelKnown::High
            }
        })
    });
    let budget = reasoning.budget_tokens.map(u64_to_i32);
    if level.is_none() && budget.is_none() {
        return None;
    }
    Some(
        wire::ThinkingConfig::builder()
            .thinking_budget(budget)
            .thinking_level(level)
            .build()
            .expect("complete thinking config"),
    )
}

fn text_part(text: String) -> Result<wire::Part, CoreError> {
    data_part(wire::PartData::Text { text })
}

fn data_part(data: wire::PartData) -> Result<wire::Part, CoreError> {
    wire::Part::builder()
        .data(Some(data))
        .build()
        .map_err(wire_build_error)
}

fn content(
    role: wire::ContentRoleKnown,
    parts: Vec<wire::Part>,
) -> Result<wire::Content, CoreError> {
    wire::Content::builder()
        .parts(parts)
        .role(Some(wire::ContentRole::Known(role)))
        .build()
        .map_err(wire_build_error)
}

fn u64_to_i32(value: u64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn u32_to_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
