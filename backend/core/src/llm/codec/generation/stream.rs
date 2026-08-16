use serde_json::Value;

use super::*;
use crate::llm::ir::generation::*;

/// Responses SSE 事件 → GenerateEvent(迁移后仅服务 OpenAI Responses 目标,
/// 事件即目标 wire 本身,无转换层)。
pub(super) struct StreamDecoder {
    target: OperationKey,
    finish_hint: Option<FinishReason>,
}

impl StreamDecoder {
    pub(super) fn new(target: OperationKey) -> Self {
        Self {
            target,
            finish_hint: None,
        }
    }

    pub(super) fn push(
        &mut self,
        frame: crate::llm::wire::JsonSseFrame,
    ) -> Result<Vec<OperationEvent>, CoreError> {
        let JsonSseData::Json(body) = frame.data else {
            return Ok(Vec::new());
        };
        let value: Value = body.decode()?;
        match decode_stream_event(&value, self.target, &mut self.finish_hint) {
            Ok(Some(event)) => Ok(vec![OperationEvent::Generate(event)]),
            Ok(None) => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }
}

fn decode_stream_event(
    value: &Value,
    target: OperationKey,
    finish_hint: &mut Option<FinishReason>,
) -> Result<Option<GenerateEvent>, CoreError> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_payload("stream event type is missing"))?;
    Ok(Some(match kind {
        "response.created" => GenerateEvent::Started(GenerationStarted {
            id: crate::llm::ir::GenerationId(pointer_str(value, "/response/id")?.into()),
            model: crate::llm::ir::ModelId(pointer_str(value, "/response/model")?.into()),
        }),
        "response.output_item.added" => {
            let item_kind = pointer_str(value, "/item/type")?;
            if super::response::requires_client_action(item_kind) {
                *finish_hint = Some(FinishReason::ToolCalls);
            }
            GenerateEvent::OutputStarted(OutputStarted {
                output_index: u32_field(value, "output_index")?,
                output_id: crate::llm::ir::OutputId(pointer_str(value, "/item/id")?.into()),
                kind: output_kind(item_kind)?,
            })
        }
        "response.content_part.added" => {
            let part_kind = pointer_str(value, "/part/type")?;
            if part_kind == "refusal" && *finish_hint != Some(FinishReason::ToolCalls) {
                *finish_hint = Some(FinishReason::Refusal);
            }
            GenerateEvent::ContentStarted(ContentStarted {
                output_index: u32_field(value, "output_index")?,
                content_index: u32_field(value, "content_index")?,
                content_id: crate::llm::ir::ContentId(format!(
                    "{}:{}",
                    u32_field(value, "output_index")?,
                    u32_field(value, "content_index")?
                )),
                kind: content_kind(part_kind)?,
            })
        }
        "response.output_text.delta" => {
            GenerateEvent::Delta(GenerateDelta::Text(content_delta(value)?))
        }
        "response.refusal.delta" => {
            if *finish_hint != Some(FinishReason::ToolCalls) {
                *finish_hint = Some(FinishReason::Refusal);
            }
            GenerateEvent::Delta(GenerateDelta::Refusal(content_delta(value)?))
        }
        "response.reasoning_summary_text.delta" => {
            GenerateEvent::Delta(GenerateDelta::ReasoningSummary(content_delta(value)?))
        }
        "response.reasoning_text.delta" => {
            GenerateEvent::Delta(GenerateDelta::ReasoningText(content_delta(value)?))
        }
        "response.function_call_arguments.delta" => {
            GenerateEvent::Delta(GenerateDelta::FunctionArguments(JsonFragmentDelta {
                output_index: u32_field(value, "output_index")?,
                delta: string_field(value, "delta")?,
            }))
        }
        "response.custom_tool_call_input.delta" => {
            GenerateEvent::Delta(GenerateDelta::CustomToolInput(OutputTextDelta {
                output_index: u32_field(value, "output_index")?,
                delta: string_field(value, "delta")?,
            }))
        }
        "response.audio.delta" => GenerateEvent::Delta(GenerateDelta::Audio(BinaryDelta {
            output_index: 0,
            content_index: 0,
            encoded: string_field(value, "delta")?,
        })),
        "response.audio.transcript.delta" => {
            GenerateEvent::Delta(GenerateDelta::Transcript(ContentTextDelta {
                output_index: 0,
                content_index: 0,
                delta: string_field(value, "delta")?,
            }))
        }
        "response.image_generation_call.partial_image" => {
            GenerateEvent::Delta(GenerateDelta::Image(ImageDelta {
                output_index: u32_field(value, "output_index")?,
                content_index: 0,
                encoded: string_field(value, "partial_image_b64")?,
                sequence: u32_field(value, "partial_image_index")?,
            }))
        }
        "response.output_text.annotation.added" => {
            GenerateEvent::Delta(GenerateDelta::Citation(CitationDelta {
                output_index: u32_field(value, "output_index")?,
                content_index: u32_field(value, "content_index")?,
                citation: decode_citation(
                    value
                        .get("annotation")
                        .ok_or_else(|| invalid_payload("annotation is missing"))?,
                )?,
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
            tool_execution_delta(value, ToolExecutionState::Running)?
        }
        "response.file_search_call.completed"
        | "response.web_search_call.completed"
        | "response.code_interpreter_call.completed"
        | "response.mcp_call.completed"
        | "response.mcp_list_tools.completed"
        | "response.image_generation_call.completed" => {
            tool_execution_delta(value, ToolExecutionState::Completed)?
        }
        "response.mcp_call.failed" | "response.mcp_list_tools.failed" => {
            tool_execution_delta(value, ToolExecutionState::Failed)?
        }
        "response.code_interpreter_call_code.delta" => {
            GenerateEvent::Delta(GenerateDelta::CustomToolInput(OutputTextDelta {
                output_index: u32_field(value, "output_index")?,
                delta: string_field(value, "delta")?,
            }))
        }
        "response.mcp_call_arguments.delta" => {
            GenerateEvent::Delta(GenerateDelta::FunctionArguments(JsonFragmentDelta {
                output_index: u32_field(value, "output_index")?,
                delta: string_field(value, "delta")?,
            }))
        }
        "response.content_part.done" => GenerateEvent::ContentFinished(ContentFinished {
            output_index: u32_field(value, "output_index")?,
            content_index: u32_field(value, "content_index")?,
            content_id: crate::llm::ir::ContentId(format!(
                "{}:{}",
                u32_field(value, "output_index")?,
                u32_field(value, "content_index")?
            )),
        }),
        "response.output_item.done" => {
            let item = value
                .get("item")
                .ok_or_else(|| invalid_payload("output item is missing"))?;
            if item
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(super::response::requires_client_action)
            {
                *finish_hint = Some(FinishReason::ToolCalls);
            }
            GenerateEvent::OutputFinished(OutputFinished {
                output_index: u32_field(value, "output_index")?,
                item: super::response::decode_output_item(item, target)?,
            })
        }
        "response.completed" => GenerateEvent::Finished(GenerationFinished {
            finish: super::response::decode_finish(
                value
                    .get("response")
                    .ok_or_else(|| invalid_payload("completed response is missing"))?,
                finish_hint.take(),
            )?,
            usage: value
                .pointer("/response/usage")
                .map(super::response::decode_usage)
                .transpose()?,
        }),
        "response.incomplete" => GenerateEvent::Finished(GenerationFinished {
            finish: super::response::decode_finish(
                value
                    .get("response")
                    .ok_or_else(|| invalid_payload("incomplete response is missing"))?,
                finish_hint.take(),
            )?,
            usage: value
                .pointer("/response/usage")
                .map(super::response::decode_usage)
                .transpose()?,
        }),
        "response.failed" | "error" => GenerateEvent::Failed(GenerationFailure {
            error: super::response::decode_failure(value),
            usage: value
                .pointer("/response/usage")
                .map(super::response::decode_usage)
                .transpose()?,
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

fn tool_execution_delta(
    value: &Value,
    state: ToolExecutionState,
) -> Result<GenerateEvent, CoreError> {
    Ok(GenerateEvent::Delta(GenerateDelta::ToolExecution(
        ToolExecutionDelta {
            output_index: u32_field(value, "output_index")?,
            output_id: crate::llm::ir::OutputId(required_str(value, "item_id")?.to_owned()),
            state,
        },
    )))
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
                .ok_or_else(|| invalid_payload("citation document id is missing"))?
                .to_owned(),
        },
        other => return Err(unmodeled(kind_key(other), Operation::StreamGenerateContent)),
    };
    Ok(Citation {
        start: u64_field(value, "start_index")?,
        end: u64_field(value, "end_index")?,
        source,
        title: value
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn output_kind(kind: &str) -> Result<OutputKind, CoreError> {
    Ok(match kind {
        "message" => OutputKind::Message,
        "reasoning" => OutputKind::Reasoning,
        "compaction" => OutputKind::Compaction,
        "mcp_approval_request" => OutputKind::McpApprovalRequest,
        "audio" => OutputKind::Audio,
        "web_search_call"
        | "file_search_call"
        | "code_interpreter_call"
        | "image_generation_call"
        | "mcp_call"
        | "mcp_list_tools" => OutputKind::ToolExecution,
        value if value.ends_with("call") => OutputKind::ToolCall,
        other => return Err(unmodeled(other, Operation::StreamGenerateContent)),
    })
}
fn content_kind(kind: &str) -> Result<ContentKind, CoreError> {
    Ok(match kind {
        "output_text" => ContentKind::Text,
        "refusal" => ContentKind::Refusal,
        "reasoning_text" => ContentKind::ReasoningText,
        "audio" => ContentKind::Audio,
        "transcript" => ContentKind::Transcript,
        "image" => ContentKind::Image,
        other => return Err(unmodeled(other, Operation::StreamGenerateContent)),
    })
}
fn content_delta(value: &Value) -> Result<ContentTextDelta, CoreError> {
    Ok(ContentTextDelta {
        output_index: u32_field(value, "output_index")?,
        content_index: u32_field(value, "content_index")?,
        delta: string_field(value, "delta")?,
    })
}
