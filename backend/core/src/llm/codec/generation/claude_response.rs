//! Claude Messages 完整响应 → IR 解码,以及与 SSE 状态机共享的映射。

use gproxy_protocol::claude as wire;
use gproxy_protocol::OperationKey;
use serde_json::Value;

use crate::llm::ir::generation::*;
use crate::llm::ir::{GenerationId, ModelId, OutputId, ToolCallId, Usage};
use crate::llm::wire::JsonBody;
use crate::CoreError;

pub(super) fn decode_complete(
    body: &JsonBody,
    target: OperationKey,
) -> Result<GenerateResponse, CoreError> {
    let message: wire::CreateMessageResponseBody = body.decode()?;
    let output = decode_content(&message.id, message.content, target)?;
    let finish = finish_reason(&message.stop_reason, has_tool_call(&output));
    Ok(GenerateResponse {
        id: GenerationId(message.id.clone()),
        model: ModelId(model_string(&message.model)),
        output,
        finish,
        usage: Some(usage(&message.usage)),
    })
}

fn decode_content(
    message_id: &str,
    content: Vec<wire::ContentBlock>,
    target: OperationKey,
) -> Result<Vec<OutputItem>, CoreError> {
    let mut output = Vec::new();
    let mut texts: Vec<OutputContent> = Vec::new();
    let mut message_count = 0usize;
    for (index, block) in content.into_iter().enumerate() {
        if let wire::ContentBlock::Text(block) = block {
            // Claude citations 定位源文档片段,IR Citation 要求响应文本区间,静默丢弃。
            texts.push(OutputContent::Text {
                text: block.text,
                citations: Vec::new(),
            });
            continue;
        }
        flush_message(message_id, &mut texts, &mut message_count, &mut output);
        let item_id = OutputId(format!("{message_id}:{index}"));
        match block {
            wire::ContentBlock::Text(_) => unreachable!("text handled above"),
            wire::ContentBlock::Thinking(block) => {
                output.push(OutputItem::Reasoning(thinking_item(
                    item_id,
                    block.thinking,
                    block.signature,
                )));
            }
            wire::ContentBlock::RedactedThinking(block) => {
                output.push(OutputItem::Reasoning(redacted_item(item_id, block.data)));
            }
            wire::ContentBlock::ToolUse(block) => {
                output.push(OutputItem::ToolCall(client_tool_call(
                    block.id,
                    block.name,
                    Value::Object(block.input.into_iter().collect()),
                )));
            }
            wire::ContentBlock::ServerToolUse(_) | wire::ContentBlock::McpToolUse(_) => {
                let value = serde_json::to_value(&block)?;
                output.push(OutputItem::ToolExecution(execution_item(
                    value, "id", None, target,
                )?));
            }
            wire::ContentBlock::McpToolResult(block) => {
                let failed = block.is_error;
                let value = serde_json::to_value(&block)?;
                output.push(OutputItem::ToolExecution(execution_item(
                    value,
                    "tool_use_id",
                    Some(failed),
                    target,
                )?));
            }
            wire::ContentBlock::WebSearchToolResult(_)
            | wire::ContentBlock::WebFetchToolResult(_)
            | wire::ContentBlock::AdvisorToolResult(_)
            | wire::ContentBlock::CodeExecutionToolResult(_)
            | wire::ContentBlock::BashCodeExecutionToolResult(_)
            | wire::ContentBlock::TextEditorCodeExecutionToolResult(_)
            | wire::ContentBlock::ToolSearchToolResult(_) => {
                let value = serde_json::to_value(&block)?;
                output.push(OutputItem::ToolExecution(execution_item(
                    value,
                    "tool_use_id",
                    None,
                    target,
                )?));
            }
            wire::ContentBlock::Compaction(block) => {
                output.push(OutputItem::Compaction(CompactionOutput {
                    id: item_id,
                    content: block.content,
                    encrypted_content: block.encrypted_content,
                }));
            }
            // 文件产物/回退标注不改变语义输出,跳过。
            wire::ContentBlock::ContainerUpload(_) | wire::ContentBlock::Fallback(_) => {}
            other => {
                let value = serde_json::to_value(&other)?;
                return Err(CoreError::UnmodeledProviderEvent {
                    target,
                    event: block_type(&value).to_owned(),
                });
            }
        }
    }
    flush_message(message_id, &mut texts, &mut message_count, &mut output);
    Ok(output)
}

/// 连续 text 块合并为一个 Message item,保持块序(首个沿用消息 id)。
fn flush_message(
    message_id: &str,
    texts: &mut Vec<OutputContent>,
    message_count: &mut usize,
    output: &mut Vec<OutputItem>,
) {
    if texts.is_empty() {
        return;
    }
    let id = if *message_count == 0 {
        message_id.to_owned()
    } else {
        format!("{message_id}:text:{message_count}")
    };
    *message_count += 1;
    output.push(OutputItem::Message(OutputMessage {
        id: OutputId(id),
        content: std::mem::take(texts),
    }));
}

/// 服务端执行的托管工具块统一保留为 ToolExecution,原始块进 output。
fn execution_item(
    value: Value,
    id_key: &str,
    failed: Option<bool>,
    target: OperationKey,
) -> Result<ToolExecution, CoreError> {
    let id = value
        .get(id_key)
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::InvalidProviderPayload {
            target,
            reason: format!("server tool block is missing {id_key}"),
        })?
        .to_owned();
    Ok(ToolExecution {
        id: OutputId(id.clone()),
        call_id: ToolCallId(id),
        state: if failed == Some(true) {
            ToolExecutionState::Failed
        } else {
            ToolExecutionState::Completed
        },
        output: Some(value),
        error: None,
    })
}

fn block_type(value: &Value) -> &str {
    value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

pub(super) fn thinking_item(id: OutputId, thinking: String, signature: String) -> ReasoningOutput {
    let continuation =
        (!signature.is_empty()).then_some(ReasoningContinuation::ClaudeSignature { signature });
    ReasoningOutput {
        id,
        parts: vec![ReasoningPart::Text {
            text: thinking,
            continuation,
        }],
    }
}

pub(super) fn redacted_item(id: OutputId, data: String) -> ReasoningOutput {
    ReasoningOutput {
        id,
        parts: vec![ReasoningPart::Opaque {
            continuation: ReasoningContinuation::ClaudeRedacted { data },
        }],
    }
}

/// 客户端工具调用:bash / text editor 工具名映射到专用 variant,其余为 Function。
pub(super) fn client_tool_call(id: String, name: String, input: Value) -> ToolCall {
    let output_id = OutputId(id.clone());
    let call_id = ToolCallId(id);
    match name.as_str() {
        "bash" => ToolCall::Shell(ShellCall {
            id: output_id,
            call_id,
            input,
        }),
        "str_replace_editor" | "str_replace_based_edit_tool" => {
            ToolCall::TextEditor(TextEditorCall {
                id: output_id,
                call_id,
                input,
            })
        }
        _ => ToolCall::Function(FunctionCall {
            id: output_id,
            call_id,
            name,
            arguments: input,
        }),
    }
}

pub(super) fn has_tool_call(output: &[OutputItem]) -> bool {
    output
        .iter()
        .any(|item| matches!(item, OutputItem::ToolCall(_)))
}

pub(super) fn finish_reason(stop_reason: &wire::StopReason, tool_calls: bool) -> FinishReason {
    use wire::StopReasonKnown as Known;
    match stop_reason {
        wire::StopReason::Known(Known::MaxTokens | Known::ModelContextWindowExceeded) => {
            FinishReason::Length
        }
        wire::StopReason::Known(Known::Refusal) => FinishReason::ContentFilter,
        wire::StopReason::Known(Known::ToolUse) => FinishReason::ToolCalls,
        _ if tool_calls => FinishReason::ToolCalls,
        _ => FinishReason::Stop,
    }
}

pub(super) fn model_string(model: &wire::ClaudeModel) -> String {
    match serde_json::to_value(model) {
        Ok(Value::String(model)) => model,
        _ => String::new(),
    }
}

/// 与 transform 相同的口径:input_tokens 计入缓存读写,total 为两侧之和。
pub(super) fn usage(usage: &wire::Usage) -> Usage {
    let cached = usage.cache_read_input_tokens.unwrap_or(0);
    let cache_write = usage.cache_creation_total().unwrap_or(0);
    let input_tokens = usage
        .input_tokens
        .unwrap_or(0)
        .saturating_add(cached)
        .saturating_add(cache_write);
    let output_tokens = usage.output_tokens.unwrap_or(0);
    Usage {
        input_tokens,
        output_tokens,
        cached_input_tokens: cached,
        reasoning_tokens: usage
            .output_tokens_details
            .as_ref()
            .map(|details| details.thinking_tokens)
            .unwrap_or(0),
        total_tokens: input_tokens.saturating_add(output_tokens),
    }
}
