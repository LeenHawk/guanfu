//! IR → Claude Messages 原生编码(typed wire,直达,不经 canonical 两跳)。

use base64::Engine;
use gproxy_protocol::claude as wire;
use gproxy_protocol::{ContentGenerationKind, OperationKey};
use serde_json::Value;

use super::options;
use crate::llm::ir::generation::*;
use crate::llm::ir::MediaSource;
use crate::CoreError;

/// 与 gproxy transform 相同的缺省 max_tokens(Claude 该字段必填)。
const DEFAULT_MAX_TOKENS: u64 = 16_384;

pub(super) fn encode_request(
    request: &GenerateRequest,
    target: OperationKey,
) -> Result<Value, CoreError> {
    if request
        .reasoning
        .as_ref()
        .and_then(|reasoning| reasoning.summary)
        .is_some_and(|summary| summary != ReasoningSummary::Auto)
    {
        return Err(incompatible(target, "reasoning.summary"));
    }
    let (tools, mcp_servers) = super::claude_tools::encode_tools(&request.tools, target)?;
    let tool_choice = (!tools.is_empty() || !mcp_servers.is_empty())
        .then(|| super::claude_tools::encode_tool_choice(&request.tool_choice))
        .transpose()?;
    let body = wire::CreateMessageRequestBody::builder()
        .model(wire::ClaudeModel::Unknown(request.model.0.clone()))
        .messages(encode_messages(request, target)?)
        .max_tokens(
            request
                .limits
                .max_output_tokens
                .unwrap_or(DEFAULT_MAX_TOKENS),
        )
        .mcp_servers((!mcp_servers.is_empty()).then_some(mcp_servers))
        .output_config(encode_output_config(request)?)
        .stop_sequences((!request.sampling.stop.is_empty()).then(|| request.sampling.stop.clone()))
        .stream((request.mode == GenerateMode::Stream).then_some(true))
        .system(encode_system(&request.instructions, target)?)
        .temperature(request.sampling.temperature.map(f64::from))
        .thinking(encode_thinking(request.reasoning.as_ref()))
        .tool_choice(tool_choice)
        .tools((!tools.is_empty()).then_some(tools))
        .top_k(request.sampling.top_k.map(i64::from))
        .top_p(request.sampling.top_p.map(f64::from))
        .build()
        .map_err(wire_build_error)?;
    let mut value = serde_json::to_value(&body)?;
    let map = value
        .as_object_mut()
        .expect("claude request serializes as an object");
    options::apply_protocol_options(
        &request.protocol_options,
        ContentGenerationKind::ClaudeMessages,
        map,
    );
    Ok(value)
}

/// System 与 Developer instructions 都并入 Claude 顶层 `system`。
fn encode_system(
    instructions: &[Instruction],
    target: OperationKey,
) -> Result<Option<wire::SystemPrompt>, CoreError> {
    let mut blocks = Vec::new();
    for instruction in instructions {
        for part in &instruction.content {
            let InputContent::Text { text } = part else {
                return Err(incompatible(target, "instructions[].content"));
            };
            blocks.push(text_block(text.clone())?);
        }
    }
    Ok((!blocks.is_empty()).then_some(wire::SystemPrompt::Array(blocks)))
}

fn encode_messages(
    request: &GenerateRequest,
    target: OperationKey,
) -> Result<Vec<wire::MessageParam>, CoreError> {
    // 历史 system 消息:支持 mid-conversation system 的模型保留 system 角色,
    // 老模型降级为 assistant 轮(与 gproxy claude_mid_conv_system 策略一致)。
    let system_role = if gproxy_transform::common::supports_mid_conv_system(&request.model.0) {
        wire::MessageRoleKnown::System
    } else {
        wire::MessageRoleKnown::Assistant
    };
    let mut messages = Vec::new();
    for item in &request.input {
        let (role, blocks) = match item {
            InputItem::Message { message } => {
                let role = match message.role {
                    MessageRole::User => wire::MessageRoleKnown::User,
                    MessageRole::Assistant => wire::MessageRoleKnown::Assistant,
                    MessageRole::System => system_role.clone(),
                };
                (role, encode_content(&message.content, target)?)
            }
            InputItem::ToolResult { result } => (
                wire::MessageRoleKnown::User,
                vec![tool_result_block(result, target)?],
            ),
            // Claude MCP 无审批应答语义,与 gproxy transform 一致丢弃。
            InputItem::McpApproval { .. } => continue,
            InputItem::Reasoning { reasoning } => (
                wire::MessageRoleKnown::Assistant,
                reasoning_blocks(reasoning)?,
            ),
        };
        if blocks.is_empty() {
            continue;
        }
        messages.push(
            wire::MessageParam::builder()
                .role(wire::MessageRole::Known(role))
                .content(wire::MessageContent::Array(blocks))
                .build()
                .map_err(wire_build_error)?,
        );
    }
    Ok(messages)
}

fn encode_content(
    content: &[InputContent],
    target: OperationKey,
) -> Result<Vec<wire::ContentBlockParam>, CoreError> {
    let mut blocks = Vec::new();
    for part in content {
        match part {
            InputContent::Text { text } if text.is_empty() => {}
            InputContent::Text { text } => {
                blocks.push(wire::ContentBlockParam::Text(text_block(text.clone())?));
            }
            InputContent::Image { source, .. } => {
                blocks.push(wire::ContentBlockParam::Image(
                    wire::ImageBlock::builder()
                        .source(image_source(source, target)?)
                        .type_(wire::ImageBlockType::Image)
                        .build()
                        .map_err(wire_build_error)?,
                ));
            }
            InputContent::Audio { .. } => {
                return Err(incompatible(target, "input[].message.content.audio"));
            }
            InputContent::File { source } => {
                let (source, title) = document_source(source, target)?;
                let mut builder = wire::DocumentBlock::builder()
                    .source(source)
                    .type_(wire::DocumentBlockType::Document);
                if title.is_some() {
                    builder = builder.title(title);
                }
                blocks.push(wire::ContentBlockParam::Document(
                    builder.build().map_err(wire_build_error)?,
                ));
            }
        }
    }
    Ok(blocks)
}

fn image_source(
    source: &MediaSource,
    target: OperationKey,
) -> Result<wire::ImageSource, CoreError> {
    Ok(match source {
        MediaSource::Url { url } => wire::ImageSource::Url(
            wire::UrlImageSource::builder()
                .type_(wire::UrlSourceType::Url)
                .url(url.clone())
                .build()
                .map_err(wire_build_error)?,
        ),
        MediaSource::File { id } => wire::ImageSource::File(
            wire::FileImageSource::builder()
                .file_id(id.0.clone())
                .type_(wire::FileSourceType::File)
                .build()
                .map_err(wire_build_error)?,
        ),
        MediaSource::Data { media_type, bytes } => {
            let media_type = match media_type.0.as_str() {
                "image/jpeg" => wire::ImageMediaType::Jpeg,
                "image/png" => wire::ImageMediaType::Png,
                "image/gif" => wire::ImageMediaType::Gif,
                "image/webp" => wire::ImageMediaType::Webp,
                _ => {
                    return Err(incompatible(
                        target,
                        "input[].message.content.image.media_type",
                    ))
                }
            };
            wire::ImageSource::Base64(
                wire::Base64ImageSource::builder()
                    .data(base64::engine::general_purpose::STANDARD.encode(bytes))
                    .media_type(media_type)
                    .type_(wire::Base64SourceType::Base64)
                    .build()
                    .map_err(wire_build_error)?,
            )
        }
    })
}

fn document_source(
    source: &FileSource,
    target: OperationKey,
) -> Result<(wire::DocumentSource, Option<String>), CoreError> {
    let source = match source {
        FileSource::Id { id } => file_document_source(id.0.clone())?,
        FileSource::Text { filename, text } => {
            return Ok((plain_text_source(text.clone())?, filename.clone()))
        }
        FileSource::Media { source } => match source {
            MediaSource::Url { url } => wire::DocumentSource::Url(
                wire::UrlDocumentSource::builder()
                    .type_(wire::UrlSourceType::Url)
                    .url(url.clone())
                    .build()
                    .map_err(wire_build_error)?,
            ),
            MediaSource::File { id } => file_document_source(id.0.clone())?,
            MediaSource::Data { media_type, bytes } => match media_type.0.as_str() {
                "application/pdf" => wire::DocumentSource::Base64(
                    wire::Base64PdfSource::builder()
                        .data(base64::engine::general_purpose::STANDARD.encode(bytes))
                        .media_type(wire::PdfMediaType::ApplicationPdf)
                        .type_(wire::Base64SourceType::Base64)
                        .build()
                        .map_err(wire_build_error)?,
                ),
                "text/plain" => plain_text_source(String::from_utf8_lossy(bytes).into_owned())?,
                _ => {
                    return Err(incompatible(
                        target,
                        "input[].message.content.file.media_type",
                    ))
                }
            },
        },
    };
    Ok((source, None))
}

fn file_document_source(file_id: String) -> Result<wire::DocumentSource, CoreError> {
    Ok(wire::DocumentSource::File(
        wire::FileDocumentSource::builder()
            .file_id(file_id)
            .type_(wire::FileSourceType::File)
            .build()
            .map_err(wire_build_error)?,
    ))
}

fn plain_text_source(data: String) -> Result<wire::DocumentSource, CoreError> {
    Ok(wire::DocumentSource::Text(
        wire::PlainTextSource::builder()
            .data(data)
            .media_type(wire::PlainTextMediaType::TextPlain)
            .type_(wire::TextSourceType::Text)
            .build()
            .map_err(wire_build_error)?,
    ))
}

fn tool_result_block(
    result: &ToolResult,
    target: OperationKey,
) -> Result<wire::ContentBlockParam, CoreError> {
    let (content, is_error) = match &result.outcome {
        ToolOutcome::Success { content } => {
            let mut blocks = Vec::new();
            for part in content {
                blocks.push(match part {
                    ToolResultContent::Text { text } => {
                        wire::ToolResultContentBlock::Text(text_block(text.clone())?)
                    }
                    ToolResultContent::Json { value } => {
                        wire::ToolResultContentBlock::Text(text_block(value.to_string())?)
                    }
                    ToolResultContent::Image { source } => wire::ToolResultContentBlock::Image(
                        wire::ImageBlock::builder()
                            .source(image_source(source, target)?)
                            .type_(wire::ImageBlockType::Image)
                            .build()
                            .map_err(wire_build_error)?,
                    ),
                });
            }
            let content = (!blocks.is_empty()).then_some(wire::ToolResultContent::Blocks(blocks));
            (content, None)
        }
        ToolOutcome::Error { code, message } => {
            let text = match code {
                Some(code) => format!("{code}: {message}"),
                None => message.clone(),
            };
            (Some(wire::ToolResultContent::Text(text)), Some(true))
        }
    };
    let mut builder = wire::ToolResultBlock::builder()
        .tool_use_id(normalize_tool_id(&result.call_id.0))
        .type_(wire::ToolResultBlockType::ToolResult);
    if content.is_some() {
        builder = builder.content(content);
    }
    if is_error.is_some() {
        builder = builder.is_error(is_error);
    }
    Ok(wire::ContentBlockParam::ToolResult(
        builder.build().map_err(wire_build_error)?,
    ))
}

/// Reasoning 回放:文本 + 签名 → thinking 块;仅不透明延续 → redacted_thinking;
/// 无签名文本与 summary 退化为 text 块(与 gproxy transform 一致)。
fn reasoning_blocks(reasoning: &ReasoningInput) -> Result<Vec<wire::ContentBlockParam>, CoreError> {
    let mut thinking = String::new();
    let mut continuation = None;
    let mut summaries = Vec::new();
    for part in &reasoning.previous.parts {
        match part {
            ReasoningPart::Summary { text } => summaries.push(text.clone()),
            ReasoningPart::Text {
                text,
                continuation: part_continuation,
            } => {
                thinking.push_str(text);
                continuation = continuation.or(part_continuation.as_ref());
            }
            ReasoningPart::Opaque {
                continuation: part_continuation,
            } => continuation = continuation.or(Some(part_continuation)),
        }
    }
    let mut blocks = Vec::new();
    match (thinking.is_empty(), continuation) {
        (false, Some(continuation)) => blocks.push(wire::ContentBlockParam::Thinking(
            wire::ThinkingBlock::builder()
                .signature(continuation.opaque_value().to_owned())
                .thinking(thinking)
                .type_(wire::ThinkingBlockType::Thinking)
                .build()
                .map_err(wire_build_error)?,
        )),
        (false, None) => blocks.push(wire::ContentBlockParam::Text(text_block(thinking)?)),
        (true, Some(continuation)) => blocks.push(wire::ContentBlockParam::RedactedThinking(
            wire::RedactedThinkingBlock::builder()
                .data(continuation.opaque_value().to_owned())
                .type_(wire::RedactedThinkingBlockType::RedactedThinking)
                .build()
                .map_err(wire_build_error)?,
        )),
        (true, None) => {}
    }
    for summary in summaries {
        if !summary.is_empty() {
            blocks.push(wire::ContentBlockParam::Text(text_block(summary)?));
        }
    }
    Ok(blocks)
}

fn encode_thinking(reasoning: Option<&ReasoningOptions>) -> Option<wire::ThinkingConfig> {
    let reasoning = reasoning?;
    let display = reasoning
        .summary
        .map(|_| wire::ThinkingDisplay::Known(wire::ThinkingDisplayKnown::Summarized));
    if let Some(budget_tokens) = reasoning.budget_tokens {
        let mut builder = wire::ThinkingEnabled::builder()
            .budget_tokens(budget_tokens)
            .type_(wire::ThinkingEnabledType::Enabled);
        if display.is_some() {
            builder = builder.display(display);
        }
        return Some(wire::ThinkingConfig::Enabled(
            builder.build().expect("complete thinking config"),
        ));
    }
    match reasoning.effort {
        Some(ReasoningEffort::None) => Some(wire::ThinkingConfig::Disabled(
            wire::ThinkingDisabled::builder()
                .type_(wire::ThinkingDisabledType::Disabled)
                .build()
                .expect("complete thinking config"),
        )),
        Some(_) => Some(adaptive_thinking(display)),
        None => display.map(|display| adaptive_thinking(Some(display))),
    }
}

fn adaptive_thinking(display: Option<wire::ThinkingDisplay>) -> wire::ThinkingConfig {
    let mut builder = wire::ThinkingAdaptive::builder().type_(wire::ThinkingAdaptiveType::Adaptive);
    if display.is_some() {
        builder = builder.display(display);
    }
    wire::ThinkingConfig::Adaptive(builder.build().expect("complete thinking config"))
}

fn encode_output_config(
    request: &GenerateRequest,
) -> Result<Option<wire::OutputConfig>, CoreError> {
    let effort = request.reasoning.as_ref().and_then(|reasoning| {
        let effort = match reasoning.effort? {
            ReasoningEffort::None | ReasoningEffort::Minimal | ReasoningEffort::Low => {
                wire::OutputEffortKnown::Low
            }
            ReasoningEffort::Medium => wire::OutputEffortKnown::Medium,
            ReasoningEffort::High => wire::OutputEffortKnown::High,
            ReasoningEffort::XHigh => wire::OutputEffortKnown::XHigh,
            ReasoningEffort::Max => wire::OutputEffortKnown::Max,
        };
        Some(wire::OutputEffort::Known(effort))
    });
    // JsonObject/Text 约束在 Claude 无落点,静默退化(与 gproxy transform 一致)。
    let format = match &request.output {
        OutputConstraint::JsonSchema { schema, .. } => Some(
            wire::JsonSchemaFormat::builder()
                .type_(wire::JsonSchemaFormatType::Known(
                    wire::JsonSchemaFormatTypeKnown::JsonSchema,
                ))
                .schema(serde_json::from_value(schema.0.clone()).unwrap_or_default())
                .build()
                .map_err(wire_build_error)?,
        ),
        OutputConstraint::Text | OutputConstraint::JsonObject => None,
    };
    if effort.is_none() && format.is_none() {
        return Ok(None);
    }
    let mut builder = wire::OutputConfig::builder();
    if effort.is_some() {
        builder = builder.effort(effort);
    }
    if format.is_some() {
        builder = builder.format(format);
    }
    Ok(Some(builder.build().map_err(wire_build_error)?))
}

pub(super) fn text_block(text: String) -> Result<wire::TextBlock, CoreError> {
    wire::TextBlock::builder()
        .text(text)
        .type_(wire::TextBlockType::Text)
        .build()
        .map_err(wire_build_error)
}

/// 与 gproxy transform 一致:回放的 call id 无 `toolu_` 前缀时补齐。
fn normalize_tool_id(id: &str) -> String {
    if id.starts_with("toolu_") {
        id.to_owned()
    } else {
        format!("toolu_{id}")
    }
}

pub(super) fn incompatible(target: OperationKey, field: &str) -> CoreError {
    CoreError::IncompatibleRoute {
        target,
        fields: vec![field.to_owned()],
    }
}

pub(super) fn wire_build_error(error: gproxy_protocol::WireBuildError) -> CoreError {
    CoreError::Transform(error.to_string())
}
