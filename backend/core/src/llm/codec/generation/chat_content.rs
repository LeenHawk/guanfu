//! IR 输入(instructions / messages / 工具结果 / reasoning 回放)→
//! Chat Completions messages 编码,与 chat.rs 的请求组装解耦。

use base64::Engine;
use gproxy_protocol::openai as wire;
use gproxy_protocol::OperationKey;

use super::claude::{incompatible, wire_build_error};
use crate::llm::ir::generation::*;
use crate::llm::ir::MediaSource;
use crate::CoreError;

pub(super) fn encode_messages(
    request: &GenerateRequest,
    target: OperationKey,
) -> Result<Vec<wire::ChatCompletionMessageParam>, CoreError> {
    let mut messages = Vec::new();
    // instructions 保留角色进首部 system/developer 消息(与 gproxy 一致)。
    for instruction in &request.instructions {
        let parts = text_parts(&instruction.content, target, "instructions[].content")?;
        if parts.is_empty() {
            continue;
        }
        let content = wire::ChatTextContent::Parts(parts);
        messages.push(match instruction.role {
            InstructionRole::System => wire::ChatCompletionMessageParam::System {
                content,
                name: None,
                extra: Default::default(),
            },
            InstructionRole::Developer => wire::ChatCompletionMessageParam::Developer {
                content,
                name: None,
                extra: Default::default(),
            },
        });
    }
    let mut pending_reasoning: Vec<String> = Vec::new();
    for item in &request.input {
        let message = match item {
            InputItem::Message { message } => encode_message(message, target)?,
            InputItem::ToolResult { result } => Some(tool_message(result, target)?),
            // Chat 无 MCP 审批应答语义,与 gproxy transform 一致丢弃。
            InputItem::McpApproval { .. } => None,
            InputItem::Reasoning { reasoning } => {
                collect_reasoning(reasoning, &mut pending_reasoning);
                None
            }
        };
        let Some(mut message) = message else { continue };
        // 无延续的 reasoning 文本挂到下一条 assistant 消息的 reasoning_content,
        // 否则独立为纯 reasoning_content 的 assistant 消息(与 gproxy 一致)。
        if let Some(text) = joined_reasoning(&mut pending_reasoning) {
            if let wire::ChatCompletionMessageParam::Assistant {
                reasoning_content, ..
            } = &mut message
            {
                *reasoning_content = Some(text);
            } else {
                messages.push(reasoning_message(text));
            }
        }
        messages.push(message);
    }
    if let Some(text) = joined_reasoning(&mut pending_reasoning) {
        messages.push(reasoning_message(text));
    }
    Ok(messages)
}

fn encode_message(
    message: &Message,
    target: OperationKey,
) -> Result<Option<wire::ChatCompletionMessageParam>, CoreError> {
    Ok(match message.role {
        MessageRole::User => {
            let parts = content_parts(&message.content, target)?;
            if parts.is_empty() {
                return Ok(None);
            }
            Some(wire::ChatCompletionMessageParam::User {
                content: collapse_user_content(parts),
                name: None,
                extra: Default::default(),
            })
        }
        // assistant 历史仅支持文本回放(Chat wire 无 assistant 媒体槽位)。
        MessageRole::Assistant => {
            let parts = assistant_parts(&message.content, target)?;
            if parts.is_empty() {
                return Ok(None);
            }
            Some(wire::ChatCompletionMessageParam::Assistant {
                content: Some(wire::ChatAssistantContent::Parts(parts)),
                audio: None,
                function_call: None,
                name: None,
                reasoning_content: None,
                refusal: None,
                tool_calls: None,
                extra: Default::default(),
            })
        }
        // 历史 system 消息保留 system 角色(与 gproxy 一致)。
        MessageRole::System => {
            let parts = text_parts(&message.content, target, "input[].message.content")?;
            if parts.is_empty() {
                return Ok(None);
            }
            Some(wire::ChatCompletionMessageParam::System {
                content: wire::ChatTextContent::Parts(parts),
                name: None,
                extra: Default::default(),
            })
        }
    })
}

/// 单个纯文本 part 收敛为字符串 content(与 gproxy 现行为一致)。
fn collapse_user_content(mut parts: Vec<wire::ChatContentPart>) -> wire::ChatContent {
    if parts.len() == 1 {
        match parts.remove(0) {
            wire::ChatContentPart::Text { text, .. } => return wire::ChatContent::Text(text),
            other => parts.push(other),
        }
    }
    wire::ChatContent::Parts(parts)
}

fn content_parts(
    content: &[InputContent],
    target: OperationKey,
) -> Result<Vec<wire::ChatContentPart>, CoreError> {
    let mut parts = Vec::new();
    for part in content {
        match part {
            InputContent::Text { text } if text.is_empty() => {}
            InputContent::Text { text } => parts.push(chat_text_part(text.clone())),
            InputContent::Image { source, detail } => {
                parts.push(image_part(source, *detail, target)?)
            }
            InputContent::Audio { source } => parts.push(audio_part(source, target)?),
            InputContent::File { source } => parts.push(file_part(source)?),
        }
    }
    Ok(parts)
}

fn image_part(
    source: &MediaSource,
    detail: ImageDetail,
    _target: OperationKey,
) -> Result<wire::ChatContentPart, CoreError> {
    let url = match source {
        MediaSource::Url { url } => url.clone(),
        MediaSource::Data { media_type, bytes } => format!(
            "data:{};base64,{}",
            media_type.0,
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ),
        // file_id 图片走 file part(与 gproxy 一致);detail 无落点丢弃。
        MediaSource::File { id } => {
            return Ok(wire::ChatContentPart::File {
                file: file_ref(None, Some(id.0.clone()), None)?,
                prompt_cache_breakpoint: None,
                extra: Default::default(),
            })
        }
    };
    Ok(wire::ChatContentPart::ImageUrl {
        image_url: wire::ImageUrl::builder()
            .url(url)
            .detail(Some(match detail {
                ImageDetail::Low => wire::ChatImageDetailLevel::Low,
                ImageDetail::High => wire::ChatImageDetailLevel::High,
                ImageDetail::Auto => wire::ChatImageDetailLevel::Auto,
            }))
            .build()
            .map_err(wire_build_error)?,
        prompt_cache_breakpoint: None,
        extra: Default::default(),
    })
}

fn audio_part(
    source: &MediaSource,
    target: OperationKey,
) -> Result<wire::ChatContentPart, CoreError> {
    let MediaSource::Data { media_type, bytes } = source else {
        return Err(incompatible(target, "input[].message.content.audio.source"));
    };
    let format = match media_type.0.as_str() {
        "audio/wav" | "audio/x-wav" => wire::InputAudioFormat::Wav,
        "audio/mp3" | "audio/mpeg" => wire::InputAudioFormat::Mp3,
        _ => {
            return Err(incompatible(
                target,
                "input[].message.content.audio.media_type",
            ))
        }
    };
    Ok(wire::ChatContentPart::InputAudio {
        input_audio: wire::InputAudio::builder()
            .data(base64::engine::general_purpose::STANDARD.encode(bytes))
            .format(format)
            .build()
            .map_err(wire_build_error)?,
        prompt_cache_breakpoint: None,
        extra: Default::default(),
    })
}

fn file_part(source: &FileSource) -> Result<wire::ChatContentPart, CoreError> {
    let file = match source {
        FileSource::Id { id } => file_ref(None, Some(id.0.clone()), None)?,
        FileSource::Text { filename, text } => {
            file_ref(Some(text.clone()), None, filename.clone())?
        }
        FileSource::Media { source } => match source {
            // 远程文件 URL 无 Chat 槽位,退化为附件说明文本(与 gproxy 一致)。
            MediaSource::Url { url } => {
                return Ok(chat_text_part(format!("Attachment URL: {url}")))
            }
            MediaSource::File { id } => file_ref(None, Some(id.0.clone()), None)?,
            MediaSource::Data { media_type, bytes } => file_ref(
                Some(format!(
                    "data:{};base64,{}",
                    media_type.0,
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                )),
                None,
                None,
            )?,
        },
    };
    Ok(wire::ChatContentPart::File {
        file,
        prompt_cache_breakpoint: None,
        extra: Default::default(),
    })
}

fn file_ref(
    file_data: Option<String>,
    file_id: Option<String>,
    filename: Option<String>,
) -> Result<wire::ChatFileRef, CoreError> {
    wire::ChatFileRef::builder()
        .file_data(file_data)
        .file_id(file_id)
        .filename(filename)
        .build()
        .map_err(wire_build_error)
}

fn tool_message(
    result: &ToolResult,
    target: OperationKey,
) -> Result<wire::ChatCompletionMessageParam, CoreError> {
    let content = match &result.outcome {
        ToolOutcome::Success { content } => {
            let mut parts = Vec::new();
            for part in content {
                match part {
                    ToolResultContent::Text { text } => parts.push(text_part(text.clone())),
                    ToolResultContent::Json { value } => parts.push(text_part(value.to_string())),
                    // Chat 工具结果无图片槽位,显式回退(旧两跳为静默丢弃)。
                    ToolResultContent::Image { .. } => {
                        return Err(incompatible(target, "input[].tool_result.content.image"))
                    }
                }
            }
            wire::ChatTextContent::Parts(parts)
        }
        ToolOutcome::Error { code, message } => wire::ChatTextContent::Text(match code {
            Some(code) => format!("{code}: {message}"),
            None => message.clone(),
        }),
    };
    Ok(wire::ChatCompletionMessageParam::Tool {
        content,
        tool_call_id: result.call_id.0.clone(),
        extra: Default::default(),
    })
}

/// 入口 continuation 校验已拒绝带延续的 reasoning,此处只剩纯文本与 summary。
fn collect_reasoning(reasoning: &ReasoningInput, pending: &mut Vec<String>) {
    for part in &reasoning.previous.parts {
        match part {
            ReasoningPart::Summary { text } | ReasoningPart::Text { text, .. } => {
                pending.push(text.clone())
            }
            ReasoningPart::Opaque { .. } => {}
        }
    }
}

fn joined_reasoning(pending: &mut Vec<String>) -> Option<String> {
    let joined = std::mem::take(pending)
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!joined.is_empty()).then_some(joined)
}

fn reasoning_message(text: String) -> wire::ChatCompletionMessageParam {
    wire::ChatCompletionMessageParam::Assistant {
        content: None,
        audio: None,
        function_call: None,
        name: None,
        reasoning_content: Some(text),
        refusal: None,
        tool_calls: None,
        extra: Default::default(),
    }
}

fn assistant_parts(
    content: &[InputContent],
    target: OperationKey,
) -> Result<Vec<wire::ChatAssistantContentPart>, CoreError> {
    let mut parts = Vec::new();
    for part in content {
        let InputContent::Text { text } = part else {
            return Err(incompatible(target, "input[].message.content"));
        };
        if !text.is_empty() {
            parts.push(wire::ChatAssistantContentPart::Text {
                text: text.clone(),
                prompt_cache_breakpoint: None,
                extra: Default::default(),
            });
        }
    }
    Ok(parts)
}

fn text_parts(
    content: &[InputContent],
    target: OperationKey,
    field: &str,
) -> Result<Vec<wire::ChatTextContentPart>, CoreError> {
    let mut parts = Vec::new();
    for part in content {
        let InputContent::Text { text } = part else {
            return Err(incompatible(target, field));
        };
        if !text.is_empty() {
            parts.push(text_part(text.clone()));
        }
    }
    Ok(parts)
}

fn text_part(text: String) -> wire::ChatTextContentPart {
    wire::ChatTextContentPart::Text {
        text,
        prompt_cache_breakpoint: None,
        extra: Default::default(),
    }
}

fn chat_text_part(text: String) -> wire::ChatContentPart {
    wire::ChatContentPart::Text {
        text,
        prompt_cache_breakpoint: None,
        extra: Default::default(),
    }
}
