use serde_json::Value;

use super::*;
use crate::llm::ir::generation::*;

pub(super) fn decode_complete(body: &JsonBody) -> Result<GenerateResponse, CoreError> {
    let value: Value = body.decode()?;
    let id = required_str(&value, "id")?;
    let model = required_str(&value, "model")?;
    let output = value
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_payload("Responses output is missing"))?
        .iter()
        .map(decode_output_item)
        .collect::<Result<Vec<_>, _>>()?;
    let finish = decode_finish(&value)?;
    let usage = value.get("usage").map(decode_usage).transpose()?;
    Ok(GenerateResponse {
        id: crate::llm::ir::GenerationId(id.into()),
        model: crate::llm::ir::ModelId(model.into()),
        output,
        finish,
        usage,
    })
}

pub(in crate::llm::codec) fn decode_output_item(value: &Value) -> Result<OutputItem, CoreError> {
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

fn decode_finish(value: &Value) -> Result<FinishReason, CoreError> {
    match required_str(value, "status")? {
        "completed" => Ok(FinishReason::Stop),
        "incomplete" => Ok(FinishReason::Incomplete),
        "failed" => Ok(FinishReason::ContentFilter),
        status => Err(invalid_payload(&format!(
            "unsupported response status {status}"
        ))),
    }
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
