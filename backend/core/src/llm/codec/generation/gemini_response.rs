//! Gemini generateContent 完整响应 → IR 解码,以及与流式状态机共享的映射。

use gproxy_protocol::gemini as wire;
use serde_json::Value;

use crate::llm::ir::generation::*;
use crate::llm::ir::{GenerationId, ModelId, OutputId, ToolCallId, Usage};
use crate::llm::wire::JsonBody;
use crate::CoreError;

pub(super) fn decode_complete(body: &JsonBody) -> Result<GenerateResponse, CoreError> {
    let response: wire::GenerateContentResponse = body.decode()?;
    let id = response.response_id.unwrap_or_default();
    let model = response.model_version.unwrap_or_default();
    let blocked = response
        .prompt_feedback
        .as_ref()
        .is_some_and(|feedback| feedback.block_reason.is_some());
    // 只解码 candidates[0](Gemini 生成通路固定单 candidate)。
    let (output, finish) = match response.candidates.into_iter().next() {
        Some(candidate) => {
            let output = candidate
                .content
                .map(|content| decode_parts(&id, content.parts))
                .unwrap_or_default();
            let finish = finish_reason(candidate.finish_reason.as_ref(), has_tool_call(&output));
            (output, finish)
        }
        None if blocked => (Vec::new(), FinishReason::ContentFilter),
        None => (Vec::new(), FinishReason::Stop),
    };
    Ok(GenerateResponse {
        id: GenerationId(id),
        model: ModelId(model),
        output,
        finish,
        usage: response.usage_metadata.as_ref().map(usage),
    })
}

/// parts 逐个映射;连续非 thought text 合并为一个 Message,保持 part 序。
/// executableCode / codeExecutionResult / inlineData 等无对应 IR 语义,
/// 忽略(与 gproxy 现行为一致)。
fn decode_parts(response_id: &str, parts: Vec<wire::Part>) -> Vec<OutputItem> {
    let mut output = Vec::new();
    let mut texts: Vec<OutputContent> = Vec::new();
    let mut message_count = 0usize;
    for (index, part) in parts.into_iter().enumerate() {
        let signature = part.thought_signature;
        let thought = part.thought == Some(true);
        match part.data {
            Some(wire::PartData::Text { text }) if thought => {
                flush_message(response_id, &mut texts, &mut message_count, &mut output);
                if let Some(item) = thought_item(item_id(response_id, index), text, signature) {
                    output.push(OutputItem::Reasoning(item));
                }
            }
            Some(wire::PartData::Text { text }) => {
                // 可见文本上的签名独立为 Opaque 段(与 gproxy 一致)。
                if let Some(signature) = signature {
                    flush_message(response_id, &mut texts, &mut message_count, &mut output);
                    output.push(OutputItem::Reasoning(opaque_item(
                        item_id(response_id, index),
                        signature,
                    )));
                }
                if !text.is_empty() {
                    texts.push(OutputContent::Text {
                        text,
                        citations: Vec::new(),
                    });
                }
            }
            Some(wire::PartData::FunctionCall { function_call }) => {
                flush_message(response_id, &mut texts, &mut message_count, &mut output);
                // functionCall 上的 thoughtSignature 在 IR 无落点,静默丢弃。
                output.push(OutputItem::ToolCall(function_tool_call(
                    item_id(response_id, index),
                    function_call,
                )));
            }
            None => {
                if let Some(signature) = signature {
                    flush_message(response_id, &mut texts, &mut message_count, &mut output);
                    output.push(OutputItem::Reasoning(opaque_item(
                        item_id(response_id, index),
                        signature,
                    )));
                }
            }
            Some(_) => {}
        }
    }
    flush_message(response_id, &mut texts, &mut message_count, &mut output);
    output
}

/// 连续 text part 合并为一个 Message item(首个沿用响应 id)。
fn flush_message(
    response_id: &str,
    texts: &mut Vec<OutputContent>,
    message_count: &mut usize,
    output: &mut Vec<OutputItem>,
) {
    if texts.is_empty() {
        return;
    }
    let id = if *message_count == 0 {
        response_id.to_owned()
    } else {
        format!("{response_id}:text:{message_count}")
    };
    *message_count += 1;
    output.push(OutputItem::Message(OutputMessage {
        id: OutputId(id),
        content: std::mem::take(texts),
    }));
}

pub(super) fn thought_item(
    id: OutputId,
    text: String,
    signature: Option<String>,
) -> Option<ReasoningOutput> {
    let continuation =
        signature.map(|signature| ReasoningContinuation::GeminiThoughtSignature { signature });
    let parts = match (text.is_empty(), continuation) {
        (false, continuation) => vec![ReasoningPart::Text { text, continuation }],
        (true, Some(continuation)) => vec![ReasoningPart::Opaque { continuation }],
        (true, None) => return None,
    };
    Some(ReasoningOutput { id, parts })
}

pub(super) fn opaque_item(id: OutputId, signature: String) -> ReasoningOutput {
    ReasoningOutput {
        id,
        parts: vec![ReasoningPart::Opaque {
            continuation: ReasoningContinuation::GeminiThoughtSignature { signature },
        }],
    }
}

/// call_id 合成对照 gproxy:优先 functionCall.id,缺省 `call_{name}`。
pub(super) fn function_tool_call(id: OutputId, call: wire::FunctionCall) -> ToolCall {
    let call_id = call.id.unwrap_or_else(|| format!("call_{}", call.name));
    ToolCall::Function(FunctionCall {
        id,
        call_id: ToolCallId(call_id),
        name: call.name,
        arguments: Value::Object(call.args.unwrap_or_default().into_iter().collect()),
    })
}

fn item_id(response_id: &str, index: usize) -> OutputId {
    OutputId(format!("{response_id}:{index}"))
}

fn has_tool_call(output: &[OutputItem]) -> bool {
    output
        .iter()
        .any(|item| matches!(item, OutputItem::ToolCall(_)))
}

/// MAX_TOKENS → Length,安全类 → ContentFilter,其余按是否有工具调用收敛。
pub(super) fn finish_reason(reason: Option<&wire::FinishReason>, tool_calls: bool) -> FinishReason {
    use wire::FinishReasonKnown as Known;
    match reason {
        Some(wire::FinishReason::Known(Known::MaxTokens)) => FinishReason::Length,
        Some(wire::FinishReason::Known(
            Known::Safety
            | Known::Recitation
            | Known::Blocklist
            | Known::ProhibitedContent
            | Known::Spii
            | Known::ImageSafety
            | Known::ImageProhibitedContent,
        )) => FinishReason::ContentFilter,
        _ if tool_calls => FinishReason::ToolCalls,
        _ => FinishReason::Stop,
    }
}

/// 与 transform 相同口径:output 计入 thoughtsTokenCount,cached 取
/// cachedContentTokenCount,total 缺省为两侧之和。
pub(super) fn usage(usage: &wire::UsageMetadata) -> Usage {
    let input_tokens = count(usage.prompt_token_count);
    let cached = count(usage.cached_content_token_count);
    let reasoning = count(usage.thoughts_token_count);
    let output_tokens = count(usage.candidates_token_count).saturating_add(reasoning);
    Usage {
        input_tokens,
        output_tokens,
        cached_input_tokens: cached,
        reasoning_tokens: reasoning,
        total_tokens: usage
            .total_token_count
            .map(count_value)
            .unwrap_or_else(|| input_tokens.saturating_add(output_tokens)),
    }
}

fn count(value: Option<i32>) -> u64 {
    value.map(count_value).unwrap_or_default()
}

fn count_value(value: i32) -> u64 {
    u64::try_from(value).unwrap_or_default()
}
