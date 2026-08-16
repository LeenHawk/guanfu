//! Chat Completions SSE → GenerateEvent 状态机(chat chunk 直达解码,
//! 不经 Responses 中间表示)。usage chunk 在 finish_reason 之后到达,
//! Finished 事件推迟到 [DONE] 收尾。

use std::collections::BTreeMap;

use gproxy_protocol::openai as wire;
use gproxy_protocol::OperationKey;
use serde_json::Value;

use super::chat_response;
use crate::llm::codec::OperationEvent;
use crate::llm::ir::generation::*;
use crate::llm::ir::{ContentId, GenerationId, ModelId, OutputId, ToolCallId};
use crate::llm::wire::{JsonSseData, JsonSseFrame};
use crate::CoreError;

pub(super) struct StreamDecoder {
    target: OperationKey,
    response_id: String,
    started: bool,
    finished: bool,
    next_index: u32,
    block: Option<Block>,
    tools: BTreeMap<u32, ToolState>,
    usage: Option<wire::CompletionUsage>,
    finish: Option<FinishReason>,
    saw_tool_call: bool,
    saw_refusal: bool,
}

enum Block {
    Message {
        index: u32,
        text: String,
        refusal: String,
        next_content: u32,
        text_content: Option<u32>,
        refusal_content: Option<u32>,
    },
    Reasoning {
        index: u32,
        text: String,
    },
}

/// 以 chat delta.tool_calls[].index 为键聚合的工具调用状态。
struct ToolState {
    output_index: u32,
    id: String,
    name: String,
    arguments: String,
    custom: bool,
}

impl StreamDecoder {
    pub(super) fn new(target: OperationKey) -> Self {
        Self {
            target,
            response_id: String::new(),
            started: false,
            finished: false,
            next_index: 0,
            block: None,
            tools: BTreeMap::new(),
            usage: None,
            finish: None,
            saw_tool_call: false,
            saw_refusal: false,
        }
    }

    pub(super) fn push(&mut self, frame: JsonSseFrame) -> Result<Vec<OperationEvent>, CoreError> {
        let events = match frame.data {
            JsonSseData::Done => self.done_events()?,
            JsonSseData::Json(body) => match body.decode::<wire::ChatCompletionChunk>() {
                Ok(chunk) => self.chunk_events(chunk)?,
                // 流内错误帧 {"error":{...}} 不是 chunk 形状,单独识别。
                Err(error) => {
                    let value: Value = body.decode()?;
                    if value.get("error").is_none() {
                        return Err(error);
                    }
                    self.finished = true;
                    vec![GenerateEvent::Failed(GenerationFailure {
                        error: super::response::decode_failure(&value),
                        usage: self.usage.as_ref().map(chat_response::usage),
                    })]
                }
            },
        };
        Ok(events.into_iter().map(OperationEvent::Generate).collect())
    }

    fn chunk_events(
        &mut self,
        chunk: wire::ChatCompletionChunk,
    ) -> Result<Vec<GenerateEvent>, CoreError> {
        let mut events = Vec::new();
        if !self.started {
            self.started = true;
            self.response_id = chunk.id.clone();
            events.push(GenerateEvent::Started(GenerationStarted {
                id: GenerationId(chunk.id),
                model: ModelId(chat_response::model_string(&chunk.model)),
            }));
        }
        // usage 为累计值,后到覆盖先到(finish 后的专属 usage chunk 也走这里)。
        if chunk.usage.is_some() {
            self.usage = chunk.usage;
        }
        for choice in chunk.choices {
            // 单 choice 通路,与完整解码一致只消费 index 0。
            if choice.index != 0 {
                continue;
            }
            self.delta_events(choice.delta, &mut events)?;
            if let Some(reason) = choice.finish_reason {
                self.close_block(&mut events);
                self.close_tools(&mut events)?;
                self.finish = Some(chat_response::finish_reason(
                    &reason,
                    self.saw_tool_call,
                    self.saw_refusal,
                ));
            }
        }
        Ok(events)
    }

    fn delta_events(
        &mut self,
        delta: wire::ChatDelta,
        events: &mut Vec<GenerateEvent>,
    ) -> Result<(), CoreError> {
        if let Some(text) = delta.reasoning_content.filter(|text| !text.is_empty()) {
            let index = self.reasoning_index(events);
            events.push(GenerateEvent::Delta(GenerateDelta::ReasoningText(
                ContentTextDelta {
                    output_index: index,
                    content_index: 0,
                    delta: text.clone(),
                },
            )));
            if let Some(Block::Reasoning { text: buffer, .. }) = self.block.as_mut() {
                buffer.push_str(&text);
            }
        }
        if let Some(text) = delta.content.filter(|text| !text.is_empty()) {
            self.message_delta(text, false, events);
        }
        if let Some(text) = delta.refusal.filter(|text| !text.is_empty()) {
            self.saw_refusal = true;
            self.message_delta(text, true, events);
        }
        for call in delta.tool_calls.unwrap_or_default() {
            self.tool_delta(call, events);
        }
        Ok(())
    }

    fn message_delta(&mut self, delta: String, is_refusal: bool, events: &mut Vec<GenerateEvent>) {
        let index = self.message_index(events);
        let Some(Block::Message {
            text,
            refusal,
            next_content,
            text_content,
            refusal_content,
            ..
        }) = self.block.as_mut()
        else {
            return;
        };
        let (slot, buffer, kind) = if is_refusal {
            (refusal_content, refusal, ContentKind::Refusal)
        } else {
            (text_content, text, ContentKind::Text)
        };
        let content_index = *slot.get_or_insert_with(|| {
            let content_index = *next_content;
            *next_content += 1;
            events.push(GenerateEvent::ContentStarted(ContentStarted {
                output_index: index,
                content_index,
                content_id: ContentId(format!("{index}:{content_index}")),
                kind,
            }));
            content_index
        });
        buffer.push_str(&delta);
        let event = ContentTextDelta {
            output_index: index,
            content_index,
            delta,
        };
        events.push(GenerateEvent::Delta(if is_refusal {
            GenerateDelta::Refusal(event)
        } else {
            GenerateDelta::Text(event)
        }));
    }

    fn tool_delta(&mut self, call: wire::ChatToolCallDelta, events: &mut Vec<GenerateEvent>) {
        let chat_index = call.index;
        if !self.tools.contains_key(&chat_index) {
            // 工具阶段开始,收尾进行中的文本/思考块。
            self.close_block(events);
            self.saw_tool_call = true;
            let output_index = self.allocate_index();
            let custom =
                call.custom.is_some() || matches!(call.type_, Some(wire::ChatToolCallType::Custom));
            let id = call
                .id
                .clone()
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| format!("call_{output_index}"));
            events.push(GenerateEvent::OutputStarted(OutputStarted {
                output_index,
                output_id: OutputId(id.clone()),
                kind: OutputKind::ToolCall,
            }));
            self.tools.insert(
                chat_index,
                ToolState {
                    output_index,
                    id,
                    name: String::new(),
                    arguments: String::new(),
                    custom,
                },
            );
        }
        let state = self
            .tools
            .get_mut(&chat_index)
            .expect("tool state inserted");
        if let Some(function) = call.function {
            if let Some(name) = function.name {
                state.name.push_str(&name);
            }
            if let Some(arguments) = function.arguments.filter(|value| !value.is_empty()) {
                state.arguments.push_str(&arguments);
                events.push(GenerateEvent::Delta(GenerateDelta::FunctionArguments(
                    JsonFragmentDelta {
                        output_index: state.output_index,
                        delta: arguments,
                    },
                )));
            }
        }
        if let Some(custom) = call.custom {
            state.custom = true;
            if let Some(name) = custom.name {
                state.name.push_str(&name);
            }
            if let Some(input) = custom.input.filter(|value| !value.is_empty()) {
                state.arguments.push_str(&input);
                events.push(GenerateEvent::Delta(GenerateDelta::CustomToolInput(
                    OutputTextDelta {
                        output_index: state.output_index,
                        delta: input,
                    },
                )));
            }
        }
    }

    fn reasoning_index(&mut self, events: &mut Vec<GenerateEvent>) -> u32 {
        if let Some(Block::Reasoning { index, .. }) = self.block.as_ref() {
            return *index;
        }
        self.close_block(events);
        let index = self.allocate_index();
        events.push(GenerateEvent::OutputStarted(OutputStarted {
            output_index: index,
            output_id: self.item_id(index),
            kind: OutputKind::Reasoning,
        }));
        self.block = Some(Block::Reasoning {
            index,
            text: String::new(),
        });
        index
    }

    fn message_index(&mut self, events: &mut Vec<GenerateEvent>) -> u32 {
        if let Some(Block::Message { index, .. }) = self.block.as_ref() {
            return *index;
        }
        self.close_block(events);
        let index = self.allocate_index();
        events.push(GenerateEvent::OutputStarted(OutputStarted {
            output_index: index,
            output_id: self.item_id(index),
            kind: OutputKind::Message,
        }));
        self.block = Some(Block::Message {
            index,
            text: String::new(),
            refusal: String::new(),
            next_content: 0,
            text_content: None,
            refusal_content: None,
        });
        index
    }

    fn close_block(&mut self, events: &mut Vec<GenerateEvent>) {
        match self.block.take() {
            None => {}
            Some(Block::Message {
                index,
                text,
                refusal,
                text_content,
                refusal_content,
                ..
            }) => {
                let mut parts = vec![
                    (
                        text_content,
                        OutputContent::Text {
                            text,
                            citations: Vec::new(),
                        },
                    ),
                    (refusal_content, OutputContent::Refusal { text: refusal }),
                ];
                parts.retain(|(slot, _)| slot.is_some());
                parts.sort_by_key(|(slot, _)| *slot);
                for (slot, _) in &parts {
                    let content_index = slot.expect("retained contents are assigned");
                    events.push(GenerateEvent::ContentFinished(ContentFinished {
                        output_index: index,
                        content_index,
                        content_id: ContentId(format!("{index}:{content_index}")),
                    }));
                }
                events.push(GenerateEvent::OutputFinished(OutputFinished {
                    output_index: index,
                    item: OutputItem::Message(OutputMessage {
                        id: self.item_id(index),
                        content: parts.into_iter().map(|(_, content)| content).collect(),
                    }),
                }));
            }
            Some(Block::Reasoning { index, text }) => {
                if text.is_empty() {
                    return;
                }
                events.push(GenerateEvent::OutputFinished(OutputFinished {
                    output_index: index,
                    item: OutputItem::Reasoning(ReasoningOutput {
                        id: self.item_id(index),
                        parts: vec![ReasoningPart::Text {
                            text,
                            continuation: None,
                        }],
                    }),
                }));
            }
        }
    }

    fn close_tools(&mut self, events: &mut Vec<GenerateEvent>) -> Result<(), CoreError> {
        for (_, state) in std::mem::take(&mut self.tools) {
            let item = if state.custom {
                ToolCall::Custom(CustomToolCall {
                    id: OutputId(state.id.clone()),
                    call_id: ToolCallId(state.id),
                    name: state.name,
                    input: state.arguments,
                })
            } else {
                ToolCall::Function(FunctionCall {
                    id: OutputId(state.id.clone()),
                    call_id: ToolCallId(state.id),
                    name: state.name,
                    arguments: chat_response::parse_arguments(&state.arguments, self.target)?,
                })
            };
            events.push(GenerateEvent::OutputFinished(OutputFinished {
                output_index: state.output_index,
                item: OutputItem::ToolCall(item),
            }));
        }
        Ok(())
    }

    fn done_events(&mut self) -> Result<Vec<GenerateEvent>, CoreError> {
        if !self.started || self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        let mut events = Vec::new();
        // 上游未发 finish_reason 时兜底收尾。
        self.close_block(&mut events);
        self.close_tools(&mut events)?;
        let finish = self.finish.take().unwrap_or(if self.saw_tool_call {
            FinishReason::ToolCalls
        } else if self.saw_refusal {
            FinishReason::Refusal
        } else {
            FinishReason::Stop
        });
        events.push(GenerateEvent::Finished(GenerationFinished {
            finish,
            usage: self.usage.as_ref().map(chat_response::usage),
        }));
        Ok(events)
    }

    fn allocate_index(&mut self) -> u32 {
        let index = self.next_index;
        self.next_index = self.next_index.saturating_add(1);
        index
    }

    fn item_id(&self, index: u32) -> OutputId {
        OutputId(format!("{}:{index}", self.response_id))
    }
}
