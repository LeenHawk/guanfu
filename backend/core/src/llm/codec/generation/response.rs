use serde_json::Value;

use super::*;
use crate::llm::ir::generation::*;

pub(super) fn decode_complete(
    body: &JsonBody,
    source: OperationKey,
) -> Result<GenerateResponse, CoreError> {
    let value: Value = body.decode()?;
    if value.get("status").and_then(Value::as_str) == Some("failed") {
        return Err(CoreError::OperationFailed(decode_failure(&value)));
    }
    let id = required_str(&value, "id")?;
    let model = required_str(&value, "model")?;
    let output = value
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_payload("Responses output is missing"))?
        .iter()
        .map(|item| decode_output_item(item, source))
        .collect::<Result<Vec<_>, _>>()?;
    let finish = decode_finish(&value, None)?;
    let usage = value.get("usage").map(decode_usage).transpose()?;
    Ok(GenerateResponse {
        id: crate::llm::ir::GenerationId(id.into()),
        model: crate::llm::ir::ModelId(model.into()),
        output,
        finish,
        usage,
    })
}

pub(in crate::llm::codec) fn decode_output_item(
    value: &Value,
    source: OperationKey,
) -> Result<OutputItem, CoreError> {
    let kind = required_str(value, "type")?;
    let id = required_str(value, "id")?.to_owned();
    let output_id = crate::llm::ir::OutputId(id.clone());
    Ok(match kind {
        "message" => OutputItem::Message(OutputMessage {
            id: output_id,
            content: value
                .get("content")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid_payload("message content must be an array"))?
                .iter()
                .map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("output_text") => Ok(OutputContent::Text {
                        text: part
                            .get("text")
                            .and_then(Value::as_str)
                            .ok_or_else(|| invalid_payload("output_text text is missing"))?
                            .into(),
                        citations: Vec::new(),
                    }),
                    Some("refusal") => Ok(OutputContent::Refusal {
                        text: part
                            .get("refusal")
                            .and_then(Value::as_str)
                            .ok_or_else(|| invalid_payload("refusal text is missing"))?
                            .into(),
                    }),
                    Some("summary_text") => Ok(OutputContent::SummaryText {
                        text: required_str(part, "text")?.into(),
                    }),
                    other => Err(unmodeled(kind_key(other), Operation::GenerateContent)),
                })
                .collect::<Result<_, _>>()?,
        }),
        "reasoning" => OutputItem::Reasoning(decode_reasoning(value, output_id, source)?),
        "compaction" => OutputItem::Compaction(CompactionOutput {
            id: output_id,
            content: value
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_owned),
            encrypted_content: required_str(value, "encrypted_content")?.into(),
        }),
        "function_call" => OutputItem::ToolCall(ToolCall::Function(FunctionCall {
            id: output_id,
            call_id: crate::llm::ir::ToolCallId(required_str(value, "call_id")?.into()),
            name: required_str(value, "name")?.into(),
            arguments: serde_json::from_str(required_str(value, "arguments")?)?,
        })),
        "custom_tool_call" => OutputItem::ToolCall(ToolCall::Custom(CustomToolCall {
            id: output_id,
            call_id: crate::llm::ir::ToolCallId(required_str(value, "call_id")?.into()),
            name: required_str(value, "name")?.into(),
            input: required_str(value, "input")?.into(),
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
            action: value
                .get("action")
                .cloned()
                .ok_or_else(|| invalid_payload("computer action is missing"))?,
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
            arguments: serde_json::from_str(required_str(value, "arguments")?)?,
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

fn decode_reasoning(
    value: &Value,
    id: crate::llm::ir::OutputId,
    source: OperationKey,
) -> Result<ReasoningOutput, CoreError> {
    let mut parts = value
        .get("summary")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|part| {
            Ok(ReasoningPart::Summary {
                text: required_str(part, "text")?.into(),
            })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    let texts = value
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|part| Ok(required_str(part, "text")?.to_owned()))
        .collect::<Result<Vec<_>, CoreError>>()?;
    let mut continuation = value
        .get("encrypted_content")
        .and_then(Value::as_str)
        .map(|value| decode_continuation(source, value, !texts.is_empty()))
        .transpose()?;
    let final_text = texts.len().checked_sub(1);
    parts.extend(texts.into_iter().enumerate().map(|(index, text)| {
        ReasoningPart::Text {
            text,
            continuation: (Some(index) == final_text)
                .then(|| continuation.take())
                .flatten(),
        }
    }));
    if let Some(continuation) = continuation {
        parts.push(ReasoningPart::Opaque { continuation });
    }
    Ok(ReasoningOutput { id, parts })
}

fn decode_continuation(
    source: OperationKey,
    value: &str,
    has_text: bool,
) -> Result<ReasoningContinuation, CoreError> {
    Ok(match source.kind() {
        OperationKind::Provider(gproxy_protocol::Provider::Claude) if has_text => {
            ReasoningContinuation::ClaudeSignature {
                signature: value.into(),
            }
        }
        OperationKind::Provider(gproxy_protocol::Provider::Claude) => {
            ReasoningContinuation::ClaudeRedacted { data: value.into() }
        }
        OperationKind::Provider(gproxy_protocol::Provider::Gemini) => {
            ReasoningContinuation::GeminiThoughtSignature {
                signature: value.into(),
            }
        }
        OperationKind::Provider(_) => ReasoningContinuation::OpenAiEncrypted {
            content: value.into(),
        },
        OperationKind::ContentGeneration(ContentGenerationKind::ClaudeMessages) if has_text => {
            ReasoningContinuation::ClaudeSignature {
                signature: value.into(),
            }
        }
        OperationKind::ContentGeneration(ContentGenerationKind::ClaudeMessages) => {
            ReasoningContinuation::ClaudeRedacted { data: value.into() }
        }
        OperationKind::ContentGeneration(ContentGenerationKind::GeminiGenerateContent) => {
            ReasoningContinuation::GeminiThoughtSignature {
                signature: value.into(),
            }
        }
        OperationKind::ContentGeneration(
            ContentGenerationKind::OpenAiResponses
            | ContentGenerationKind::OpenAiResponsesWebSocket
            | ContentGenerationKind::OpenAiChatCompletions,
        ) => ReasoningContinuation::OpenAiEncrypted {
            content: value.into(),
        },
        _ => {
            return Err(CoreError::InvalidProviderPayload {
                target: source,
                reason: "reasoning continuation came from an unsupported generation protocol"
                    .into(),
            })
        }
    })
}

pub(super) fn decode_finish(
    value: &Value,
    stream_hint: Option<FinishReason>,
) -> Result<FinishReason, CoreError> {
    match required_str(value, "status")? {
        "completed" => Ok(stream_hint
            .or_else(|| finish_from_output(value))
            .unwrap_or(FinishReason::Stop)),
        "incomplete" => Ok(
            match value
                .pointer("/incomplete_details/reason")
                .and_then(Value::as_str)
            {
                Some("max_output_tokens") => FinishReason::Length,
                Some("content_filter") => FinishReason::ContentFilter,
                _ => FinishReason::Incomplete,
            },
        ),
        status => Err(invalid_payload(&format!(
            "unsupported response status {status}"
        ))),
    }
}

pub(super) fn decode_failure(value: &Value) -> crate::llm::ir::OperationFailure {
    let error = value
        .get("error")
        .or_else(|| value.pointer("/response/error"))
        .unwrap_or(value);
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("generation_failed")
        .to_owned();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("generation failed")
        .to_owned();
    crate::llm::ir::OperationFailure {
        retryable: matches!(code.as_str(), "server_error" | "rate_limit_exceeded"),
        code,
        message,
        details: Default::default(),
    }
}

fn finish_from_output(value: &Value) -> Option<FinishReason> {
    let output = value.get("output")?.as_array()?;
    if output.iter().any(|item| {
        item.get("type")
            .and_then(Value::as_str)
            .is_some_and(requires_client_action)
    }) {
        Some(FinishReason::ToolCalls)
    } else if output.iter().any(output_contains_refusal) {
        Some(FinishReason::Refusal)
    } else {
        None
    }
}

pub(super) fn requires_client_action(kind: &str) -> bool {
    matches!(
        kind,
        "function_call"
            | "custom_tool_call"
            | "computer_call"
            | "shell_call"
            | "local_shell_call"
            | "apply_patch_call"
            | "mcp_approval_request"
            | "tool_search_call"
    )
}

fn output_contains_refusal(item: &Value) -> bool {
    item.get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|part| part.get("type").and_then(Value::as_str) == Some("refusal"))
}

pub(super) fn decode_usage(value: &Value) -> Result<crate::llm::ir::Usage, CoreError> {
    let input_tokens = u64_field(value, "input_tokens")?;
    let output_tokens = u64_field(value, "output_tokens")?;
    Ok(crate::llm::ir::Usage {
        input_tokens,
        output_tokens,
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
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| invalid_payload("total_tokens must be an integer"))
            })
            .transpose()?
            .unwrap_or(input_tokens + output_tokens),
    })
}
