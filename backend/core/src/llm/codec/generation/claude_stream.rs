//! Claude Messages SSE → GenerateEvent 状态机(message_start / content_block_*
//! / message_delta / message_stop 直达解码,不经 Responses 中间表示)。

use std::collections::BTreeMap;

use gproxy_protocol::claude as wire;
use gproxy_protocol::OperationKey;
use serde_json::Value;

use super::claude_response;
use crate::llm::codec::OperationEvent;
use crate::llm::ir::generation::*;
use crate::llm::ir::{ContentId, GenerationId, ModelId, OperationFailure, OutputId};
use crate::llm::wire::{JsonSseData, JsonSseFrame};
use crate::CoreError;

pub(super) struct StreamDecoder {
    target: OperationKey,
    message_id: String,
    blocks: BTreeMap<u64, Block>,
    usage: Option<wire::Usage>,
    stop_reason: Option<wire::StopReason>,
    saw_tool_call: bool,
}

enum Block {
    Message {
        text: String,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
    Redacted {
        data: String,
    },
    ToolCall {
        id: String,
        name: String,
        json: String,
    },
    Execution {
        value: Value,
        json: String,
    },
    Compaction {
        content: String,
        encrypted: String,
    },
    Ignored,
}

impl StreamDecoder {
    pub(super) fn new(target: OperationKey) -> Self {
        Self {
            target,
            message_id: String::new(),
            blocks: BTreeMap::new(),
            usage: None,
            stop_reason: None,
            saw_tool_call: false,
        }
    }

    pub(super) fn push(&mut self, frame: JsonSseFrame) -> Result<Vec<OperationEvent>, CoreError> {
        let JsonSseData::Json(body) = frame.data else {
            return Ok(Vec::new());
        };
        let events = match body.decode::<wire::StreamEvent>()? {
            wire::StreamEvent::Known(event) => self.known_event(*event)?,
            // Claude 协议约定客户端忽略未知事件类型。
            wire::StreamEvent::Unknown(_) => Vec::new(),
            _ => Vec::new(),
        };
        Ok(events.into_iter().map(OperationEvent::Generate).collect())
    }

    fn known_event(
        &mut self,
        event: wire::KnownStreamEvent,
    ) -> Result<Vec<GenerateEvent>, CoreError> {
        Ok(match event {
            wire::KnownStreamEvent::MessageStart { message, .. } => {
                self.message_id = message.id.clone();
                self.usage = Some(message.usage);
                vec![GenerateEvent::Started(GenerationStarted {
                    id: GenerationId(message.id),
                    model: ModelId(claude_response::model_string(&message.model)),
                })]
            }
            wire::KnownStreamEvent::ContentBlockStart {
                index,
                content_block,
                ..
            } => self.block_start(index, *content_block)?,
            wire::KnownStreamEvent::ContentBlockDelta { index, delta, .. } => {
                self.block_delta(index, *delta)
            }
            wire::KnownStreamEvent::ContentBlockStop { index, .. } => self.block_stop(index)?,
            wire::KnownStreamEvent::MessageDelta { delta, usage, .. } => {
                self.stop_reason = delta.stop_reason.or(self.stop_reason.take());
                if let Some(update) = usage {
                    self.merge_usage(*update);
                }
                Vec::new()
            }
            wire::KnownStreamEvent::MessageStop { .. } => {
                let stop_reason = self.stop_reason.take();
                let finish = stop_reason
                    .as_ref()
                    .map(|reason| claude_response::finish_reason(reason, self.saw_tool_call))
                    .unwrap_or(if self.saw_tool_call {
                        FinishReason::ToolCalls
                    } else {
                        FinishReason::Stop
                    });
                vec![GenerateEvent::Finished(GenerationFinished {
                    finish,
                    usage: self.usage.as_ref().map(claude_response::usage),
                })]
            }
            wire::KnownStreamEvent::Error { error, .. } => {
                vec![GenerateEvent::Failed(GenerationFailure {
                    error: OperationFailure {
                        retryable: matches!(
                            error.type_.as_str(),
                            "overloaded_error" | "api_error" | "rate_limit_error"
                        ),
                        code: error.type_,
                        message: error.message,
                        details: Default::default(),
                    },
                    usage: self.usage.as_ref().map(claude_response::usage),
                })]
            }
            wire::KnownStreamEvent::Ping { .. } => Vec::new(),
            other => {
                let value = serde_json::to_value(&other)?;
                return Err(CoreError::UnmodeledProviderEvent {
                    target: self.target,
                    event: value
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_owned(),
                });
            }
        })
    }

    fn block_start(
        &mut self,
        index: u64,
        block: wire::ContentBlock,
    ) -> Result<Vec<GenerateEvent>, CoreError> {
        let output_index = to_u32(index);
        let mut events = Vec::new();
        let state = match block {
            wire::ContentBlock::Text(block) => {
                events.push(self.output_started(index, OutputKind::Message));
                events.push(GenerateEvent::ContentStarted(ContentStarted {
                    output_index,
                    content_index: 0,
                    content_id: ContentId(format!("{output_index}:0")),
                    kind: ContentKind::Text,
                }));
                if !block.text.is_empty() {
                    events.push(text_delta(output_index, &block.text, GenerateDelta::Text));
                }
                Block::Message { text: block.text }
            }
            wire::ContentBlock::Thinking(block) => {
                events.push(self.output_started(index, OutputKind::Reasoning));
                if !block.thinking.is_empty() {
                    events.push(text_delta(
                        output_index,
                        &block.thinking,
                        GenerateDelta::ReasoningText,
                    ));
                }
                Block::Thinking {
                    thinking: block.thinking,
                    signature: block.signature,
                }
            }
            wire::ContentBlock::RedactedThinking(block) => {
                events.push(self.output_started(index, OutputKind::Reasoning));
                Block::Redacted { data: block.data }
            }
            wire::ContentBlock::ToolUse(block) => {
                self.saw_tool_call = true;
                events.push(GenerateEvent::OutputStarted(OutputStarted {
                    output_index,
                    output_id: OutputId(block.id.clone()),
                    kind: OutputKind::ToolCall,
                }));
                Block::ToolCall {
                    id: block.id,
                    name: block.name,
                    json: initial_input(block.input),
                }
            }
            wire::ContentBlock::ServerToolUse(_) | wire::ContentBlock::McpToolUse(_) => {
                let value = serde_json::to_value(&block)?;
                events.push(GenerateEvent::OutputStarted(OutputStarted {
                    output_index,
                    output_id: OutputId(id_field(&value, "id")),
                    kind: OutputKind::ToolExecution,
                }));
                Block::Execution {
                    value,
                    json: String::new(),
                }
            }
            wire::ContentBlock::WebSearchToolResult(_)
            | wire::ContentBlock::WebFetchToolResult(_)
            | wire::ContentBlock::AdvisorToolResult(_)
            | wire::ContentBlock::CodeExecutionToolResult(_)
            | wire::ContentBlock::BashCodeExecutionToolResult(_)
            | wire::ContentBlock::TextEditorCodeExecutionToolResult(_)
            | wire::ContentBlock::ToolSearchToolResult(_)
            | wire::ContentBlock::McpToolResult(_) => {
                let value = serde_json::to_value(&block)?;
                events.push(GenerateEvent::OutputStarted(OutputStarted {
                    output_index,
                    output_id: OutputId(id_field(&value, "tool_use_id")),
                    kind: OutputKind::ToolExecution,
                }));
                Block::Execution {
                    value,
                    json: String::new(),
                }
            }
            wire::ContentBlock::Compaction(block) => {
                events.push(self.output_started(index, OutputKind::Compaction));
                Block::Compaction {
                    content: block.content.unwrap_or_default(),
                    encrypted: block.encrypted_content,
                }
            }
            // 未建模的块整体忽略(Claude 前向兼容约定)。
            _ => Block::Ignored,
        };
        self.blocks.insert(index, state);
        Ok(events)
    }

    fn block_delta(&mut self, index: u64, delta: wire::EventDelta) -> Vec<GenerateEvent> {
        let output_index = to_u32(index);
        let wire::EventDelta::Known(delta) = delta else {
            return Vec::new();
        };
        let Some(block) = self.blocks.get_mut(&index) else {
            return Vec::new();
        };
        match (*delta, block) {
            (wire::KnownEventDelta::Text { text, .. }, Block::Message { text: buffer }) => {
                buffer.push_str(&text);
                vec![text_delta(output_index, &text, GenerateDelta::Text)]
            }
            (
                wire::KnownEventDelta::Thinking { thinking, .. },
                Block::Thinking {
                    thinking: buffer, ..
                },
            ) => {
                buffer.push_str(&thinking);
                vec![text_delta(
                    output_index,
                    &thinking,
                    GenerateDelta::ReasoningText,
                )]
            }
            (
                wire::KnownEventDelta::Signature {
                    signature: delta, ..
                },
                Block::Thinking { signature, .. },
            ) => {
                signature.push_str(&delta);
                Vec::new()
            }
            (
                wire::KnownEventDelta::InputJson { partial_json, .. },
                Block::ToolCall { json, .. },
            ) => {
                json.push_str(&partial_json);
                vec![GenerateEvent::Delta(GenerateDelta::FunctionArguments(
                    JsonFragmentDelta {
                        output_index,
                        delta: partial_json,
                    },
                ))]
            }
            (
                wire::KnownEventDelta::InputJson { partial_json, .. },
                Block::Execution { json, .. },
            ) => {
                json.push_str(&partial_json);
                Vec::new()
            }
            (
                wire::KnownEventDelta::Compaction {
                    content,
                    encrypted_content,
                    ..
                },
                Block::Compaction {
                    content: buffer,
                    encrypted,
                },
            ) => {
                buffer.push_str(&content);
                encrypted.push_str(&encrypted_content);
                Vec::new()
            }
            // citations 及其余组合无对应 IR 语义,忽略。
            _ => Vec::new(),
        }
    }

    fn block_stop(&mut self, index: u64) -> Result<Vec<GenerateEvent>, CoreError> {
        let output_index = to_u32(index);
        let Some(block) = self.blocks.remove(&index) else {
            return Ok(Vec::new());
        };
        let item = match block {
            Block::Message { text } => {
                let item = OutputItem::Message(OutputMessage {
                    id: self.item_id(index),
                    content: vec![OutputContent::Text {
                        text,
                        citations: Vec::new(),
                    }],
                });
                return Ok(vec![
                    GenerateEvent::ContentFinished(ContentFinished {
                        output_index,
                        content_index: 0,
                        content_id: ContentId(format!("{output_index}:0")),
                    }),
                    GenerateEvent::OutputFinished(OutputFinished { output_index, item }),
                ]);
            }
            Block::Thinking {
                thinking,
                signature,
            } => OutputItem::Reasoning(claude_response::thinking_item(
                self.item_id(index),
                thinking,
                signature,
            )),
            Block::Redacted { data } => {
                OutputItem::Reasoning(claude_response::redacted_item(self.item_id(index), data))
            }
            Block::ToolCall { id, name, json } => OutputItem::ToolCall(
                claude_response::client_tool_call(id, name, parse_input(&json, self.target)?),
            ),
            Block::Execution { mut value, json } => {
                if !json.is_empty() {
                    value["input"] = parse_input(&json, self.target)?;
                }
                let id = id_field(&value, "id");
                OutputItem::ToolExecution(ToolExecution {
                    id: OutputId(id.clone()),
                    call_id: crate::llm::ir::ToolCallId(id),
                    state: ToolExecutionState::Completed,
                    output: Some(value),
                    error: None,
                })
            }
            Block::Compaction { content, encrypted } => OutputItem::Compaction(CompactionOutput {
                id: self.item_id(index),
                content: (!content.is_empty()).then_some(content),
                encrypted_content: encrypted,
            }),
            Block::Ignored => return Ok(Vec::new()),
        };
        Ok(vec![GenerateEvent::OutputFinished(OutputFinished {
            output_index,
            item,
        })])
    }

    fn output_started(&self, index: u64, kind: OutputKind) -> GenerateEvent {
        GenerateEvent::OutputStarted(OutputStarted {
            output_index: to_u32(index),
            output_id: self.item_id(index),
            kind,
        })
    }

    fn item_id(&self, index: u64) -> OutputId {
        OutputId(format!("{}:{index}", self.message_id))
    }

    /// message_delta 的 usage 为累计值,Some 字段覆盖既有值。
    fn merge_usage(&mut self, update: wire::Usage) {
        let Some(usage) = self.usage.as_mut() else {
            self.usage = Some(update);
            return;
        };
        macro_rules! overlay {
            ($($field:ident),+) => {
                $(if update.$field.is_some() { usage.$field = update.$field; })+
            };
        }
        overlay!(
            input_tokens,
            output_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens,
            cache_creation,
            output_tokens_details
        );
    }
}

fn text_delta(
    output_index: u32,
    delta: &str,
    wrap: fn(ContentTextDelta) -> GenerateDelta,
) -> GenerateEvent {
    GenerateEvent::Delta(wrap(ContentTextDelta {
        output_index,
        content_index: 0,
        delta: delta.to_owned(),
    }))
}

/// content_block_start 携带的初始 input:空对象视为待流式累积。
fn initial_input(input: wire::JsonObject) -> String {
    if input.is_empty() {
        String::new()
    } else {
        serde_json::to_string(&input).unwrap_or_default()
    }
}

fn parse_input(json: &str, target: OperationKey) -> Result<Value, CoreError> {
    if json.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_str(json).map_err(|error| CoreError::InvalidProviderPayload {
        target,
        reason: format!("invalid streamed tool input JSON: {error}"),
    })
}

fn id_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn to_u32(index: u64) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}
