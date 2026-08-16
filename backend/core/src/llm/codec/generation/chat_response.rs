//! Chat Completions 完整响应 → IR 解码,以及与流式状态机共享的映射。

use gproxy_protocol::openai as wire;
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
    let response: wire::ChatCompletionResponse = body.decode()?;
    let id = response.id;
    let model = model_string(&response.model);
    let usage = response.usage.as_ref().map(usage);
    // 单 choice 通路(n 无 IR 落点),与 gproxy 现行为一致只取首个。
    let Some(choice) = response.choices.into_iter().next() else {
        return Ok(GenerateResponse {
            id: GenerationId(id),
            model: ModelId(model),
            output: Vec::new(),
            finish: FinishReason::Stop,
            usage,
        });
    };
    let output = decode_message(&id, choice.message, target)?;
    let finish = finish_reason(
        &choice.finish_reason,
        has_tool_call(&output),
        has_refusal(&output),
    );
    Ok(GenerateResponse {
        id: GenerationId(id),
        model: ModelId(model),
        output,
        finish,
        usage,
    })
}

/// reasoning_content → Reasoning item(旧两跳在完整响应侧丢弃它);
/// content/refusal 合并为 Message;tool_calls 逐个映射。旧式 function_call
/// 与 audio 输出不在映射范围(guanfu 编码器不会发出 functions/audio 参数)。
fn decode_message(
    response_id: &str,
    message: wire::ChatMessage,
    target: OperationKey,
) -> Result<Vec<OutputItem>, CoreError> {
    let mut output = Vec::new();
    if let Some(text) = message.reasoning_content.filter(|text| !text.is_empty()) {
        output.push(OutputItem::Reasoning(ReasoningOutput {
            id: OutputId(format!("{response_id}:reasoning")),
            parts: vec![ReasoningPart::Text {
                text,
                continuation: None,
            }],
        }));
    }
    let mut content = Vec::new();
    if let Some(text) = message.content.filter(|text| !text.is_empty()) {
        content.push(OutputContent::Text {
            text,
            citations: citations(message.annotations),
        });
    }
    if let Some(text) = message.refusal.filter(|text| !text.is_empty()) {
        content.push(OutputContent::Refusal { text });
    }
    if !content.is_empty() {
        output.push(OutputItem::Message(OutputMessage {
            id: OutputId(response_id.to_owned()),
            content,
        }));
    }
    for call in message.tool_calls.unwrap_or_default() {
        output.push(OutputItem::ToolCall(tool_call(call, target)?));
    }
    Ok(output)
}

fn tool_call(call: wire::ChatToolCall, target: OperationKey) -> Result<ToolCall, CoreError> {
    Ok(match call {
        wire::ChatToolCall::Function { id, function, .. } => ToolCall::Function(FunctionCall {
            id: OutputId(id.clone()),
            call_id: ToolCallId(id),
            name: function.name,
            arguments: parse_arguments(&function.arguments, target)?,
        }),
        wire::ChatToolCall::Custom { id, custom, .. } => ToolCall::Custom(CustomToolCall {
            id: OutputId(id.clone()),
            call_id: ToolCallId(id),
            name: custom.name,
            input: custom.input,
        }),
        _ => {
            return Err(CoreError::UnmodeledProviderEvent {
                target,
                event: "chat tool call variant".into(),
            })
        }
    })
}

pub(super) fn parse_arguments(arguments: &str, target: OperationKey) -> Result<Value, CoreError> {
    if arguments.trim().is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_str(arguments).map_err(|error| CoreError::InvalidProviderPayload {
        target,
        reason: format!("invalid tool call arguments JSON: {error}"),
    })
}

fn citations(annotations: Option<Vec<wire::ChatAnnotation>>) -> Vec<Citation> {
    annotations
        .unwrap_or_default()
        .into_iter()
        .map(|annotation| Citation {
            start: u64::from(annotation.url_citation.start_index),
            end: u64::from(annotation.url_citation.end_index),
            source: CitationSource::Url {
                url: annotation.url_citation.url,
            },
            title: Some(annotation.url_citation.title),
        })
        .collect()
}

fn has_tool_call(output: &[OutputItem]) -> bool {
    output
        .iter()
        .any(|item| matches!(item, OutputItem::ToolCall(_)))
}

fn has_refusal(output: &[OutputItem]) -> bool {
    output.iter().any(|item| {
        matches!(
            item,
            OutputItem::Message(message)
                if message
                    .content
                    .iter()
                    .any(|part| matches!(part, OutputContent::Refusal { .. }))
        )
    })
}

/// 与 Responses 解码同一优先序:工具调用 > refusal > stop。
pub(super) fn finish_reason(
    reason: &wire::ChatFinishReason,
    tool_calls: bool,
    refusal: bool,
) -> FinishReason {
    match reason {
        wire::ChatFinishReason::Length => FinishReason::Length,
        wire::ChatFinishReason::ContentFilter => FinishReason::ContentFilter,
        wire::ChatFinishReason::ToolCalls | wire::ChatFinishReason::FunctionCall => {
            FinishReason::ToolCalls
        }
        _ if tool_calls => FinishReason::ToolCalls,
        _ if refusal => FinishReason::Refusal,
        _ => FinishReason::Stop,
    }
}

pub(super) fn model_string(model: &wire::OpenAiModelId) -> String {
    match serde_json::to_value(model) {
        Ok(Value::String(model)) => model,
        _ => String::new(),
    }
}

pub(super) fn usage(usage: &wire::CompletionUsage) -> Usage {
    Usage {
        input_tokens: u64::from(usage.prompt_tokens),
        output_tokens: u64::from(usage.completion_tokens),
        cached_input_tokens: usage
            .prompt_tokens_details
            .as_ref()
            .and_then(|details| details.cached_tokens)
            .map(u64::from)
            .unwrap_or(0),
        reasoning_tokens: usage
            .completion_tokens_details
            .as_ref()
            .and_then(|details| details.reasoning_tokens)
            .map(u64::from)
            .unwrap_or(0),
        total_tokens: u64::from(usage.total_tokens),
    }
}
