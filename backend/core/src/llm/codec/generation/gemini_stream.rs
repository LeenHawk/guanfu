//! Gemini streamGenerateContent SSE → GenerateEvent 状态机(增量
//! GenerateContentResponse chunk 直达解码,不经 Responses 中间表示)。

use gproxy_protocol::gemini as wire;

use super::gemini_response;
use crate::llm::codec::OperationEvent;
use crate::llm::ir::generation::*;
use crate::llm::ir::{ContentId, GenerationId, ModelId, OutputId};
use crate::llm::wire::{JsonSseData, JsonSseFrame};
use crate::CoreError;

#[derive(Default)]
pub(super) struct StreamDecoder {
    response_id: String,
    started: bool,
    finished: bool,
    next_index: u32,
    block: Option<Block>,
    usage: Option<wire::UsageMetadata>,
    saw_tool_call: bool,
}

enum Block {
    Message {
        index: u32,
        text: String,
    },
    Thinking {
        index: u32,
        text: String,
        signature: Option<String>,
    },
}

impl StreamDecoder {
    pub(super) fn push(&mut self, frame: JsonSseFrame) -> Result<Vec<OperationEvent>, CoreError> {
        let JsonSseData::Json(body) = frame.data else {
            return Ok(Vec::new());
        };
        let chunk: wire::GenerateContentResponse = body.decode()?;
        Ok(self
            .chunk_events(chunk)
            .into_iter()
            .map(OperationEvent::Generate)
            .collect())
    }

    fn chunk_events(&mut self, chunk: wire::GenerateContentResponse) -> Vec<GenerateEvent> {
        let mut events = Vec::new();
        if !self.started {
            self.started = true;
            self.response_id = chunk.response_id.clone().unwrap_or_default();
            events.push(GenerateEvent::Started(GenerationStarted {
                id: GenerationId(self.response_id.clone()),
                model: ModelId(chunk.model_version.clone().unwrap_or_default()),
            }));
        }
        // usageMetadata 为累计值,后到覆盖先到,收尾时取最终值。
        if chunk.usage_metadata.is_some() {
            self.usage = chunk.usage_metadata;
        }
        let blocked = chunk
            .prompt_feedback
            .as_ref()
            .is_some_and(|feedback| feedback.block_reason.is_some());
        // 只消费 candidates[0](与完整解码一致)。
        let Some(candidate) = chunk.candidates.into_iter().next() else {
            if blocked && !self.finished {
                self.close_block(&mut events);
                events.push(self.finished_event(FinishReason::ContentFilter));
            }
            return events;
        };
        if let Some(content) = candidate.content {
            for part in content.parts {
                self.part_events(part, &mut events);
            }
        }
        // finishReason 位于末 chunk,收到即收尾。
        if let Some(reason) = candidate.finish_reason {
            if !self.finished {
                self.close_block(&mut events);
                let finish = gemini_response::finish_reason(Some(&reason), self.saw_tool_call);
                events.push(self.finished_event(finish));
            }
        }
        events
    }

    fn part_events(&mut self, part: wire::Part, events: &mut Vec<GenerateEvent>) {
        let signature = part.thought_signature;
        let thought = part.thought == Some(true);
        match part.data {
            Some(wire::PartData::Text { text }) if thought => {
                let index = self.thinking_index(events);
                if !text.is_empty() {
                    events.push(text_delta(index, &text, GenerateDelta::ReasoningText));
                }
                if let Some(Block::Thinking {
                    text: buffer,
                    signature: slot,
                    ..
                }) = self.block.as_mut()
                {
                    buffer.push_str(&text);
                    if signature.is_some() {
                        *slot = signature;
                    }
                }
            }
            Some(wire::PartData::Text { text }) => {
                // 可见文本上的签名独立为 Opaque 段(与完整解码一致)。
                if let Some(signature) = signature {
                    self.standalone_signature(signature, events);
                }
                if text.is_empty() {
                    return;
                }
                let index = self.message_index(events);
                events.push(text_delta(index, &text, GenerateDelta::Text));
                if let Some(Block::Message { text: buffer, .. }) = self.block.as_mut() {
                    buffer.push_str(&text);
                }
            }
            // functionCall 整体到达:一次给出参数 delta 与完成 item。
            Some(wire::PartData::FunctionCall { function_call }) => {
                self.close_block(events);
                self.saw_tool_call = true;
                let index = self.allocate_index();
                let mut function_call = function_call;
                if function_call.id.is_none() {
                    // 无 id 时以输出序号合成 call id(与 gproxy 流式现行为一致)。
                    function_call.id = Some(format!("call_{index}"));
                }
                let item_id = self.item_id(index);
                events.push(GenerateEvent::OutputStarted(OutputStarted {
                    output_index: index,
                    output_id: item_id.clone(),
                    kind: OutputKind::ToolCall,
                }));
                let arguments =
                    serde_json::to_string(&function_call.args.clone().unwrap_or_default())
                        .unwrap_or_else(|_| "{}".into());
                events.push(GenerateEvent::Delta(GenerateDelta::FunctionArguments(
                    JsonFragmentDelta {
                        output_index: index,
                        delta: arguments,
                    },
                )));
                events.push(GenerateEvent::OutputFinished(OutputFinished {
                    output_index: index,
                    item: OutputItem::ToolCall(gemini_response::function_tool_call(
                        item_id,
                        function_call,
                    )),
                }));
            }
            None => {
                if let Some(signature) = signature {
                    // 优先挂到进行中的 thought 块,否则独立 Opaque 段。
                    if let Some(Block::Thinking {
                        signature: slot, ..
                    }) = self.block.as_mut()
                    {
                        *slot = Some(signature);
                    } else {
                        self.standalone_signature(signature, events);
                    }
                }
            }
            // 未建模 part(inlineData/executableCode 等)忽略,与 gproxy 一致。
            Some(_) => {}
        }
    }

    fn thinking_index(&mut self, events: &mut Vec<GenerateEvent>) -> u32 {
        if let Some(Block::Thinking { index, .. }) = self.block.as_ref() {
            return *index;
        }
        self.close_block(events);
        let index = self.allocate_index();
        events.push(GenerateEvent::OutputStarted(OutputStarted {
            output_index: index,
            output_id: self.item_id(index),
            kind: OutputKind::Reasoning,
        }));
        self.block = Some(Block::Thinking {
            index,
            text: String::new(),
            signature: None,
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
        events.push(GenerateEvent::ContentStarted(ContentStarted {
            output_index: index,
            content_index: 0,
            content_id: ContentId(format!("{index}:0")),
            kind: ContentKind::Text,
        }));
        self.block = Some(Block::Message {
            index,
            text: String::new(),
        });
        index
    }

    fn standalone_signature(&mut self, signature: String, events: &mut Vec<GenerateEvent>) {
        self.close_block(events);
        let index = self.allocate_index();
        let id = self.item_id(index);
        events.push(GenerateEvent::OutputStarted(OutputStarted {
            output_index: index,
            output_id: id.clone(),
            kind: OutputKind::Reasoning,
        }));
        events.push(GenerateEvent::OutputFinished(OutputFinished {
            output_index: index,
            item: OutputItem::Reasoning(gemini_response::opaque_item(id, signature)),
        }));
    }

    fn close_block(&mut self, events: &mut Vec<GenerateEvent>) {
        match self.block.take() {
            None => {}
            Some(Block::Message { index, text }) => {
                events.push(GenerateEvent::ContentFinished(ContentFinished {
                    output_index: index,
                    content_index: 0,
                    content_id: ContentId(format!("{index}:0")),
                }));
                events.push(GenerateEvent::OutputFinished(OutputFinished {
                    output_index: index,
                    item: OutputItem::Message(OutputMessage {
                        id: self.item_id(index),
                        content: vec![OutputContent::Text {
                            text,
                            citations: Vec::new(),
                        }],
                    }),
                }));
            }
            Some(Block::Thinking {
                index,
                text,
                signature,
            }) => {
                if let Some(item) =
                    gemini_response::thought_item(self.item_id(index), text, signature)
                {
                    events.push(GenerateEvent::OutputFinished(OutputFinished {
                        output_index: index,
                        item: OutputItem::Reasoning(item),
                    }));
                }
            }
        }
    }

    fn finished_event(&mut self, finish: FinishReason) -> GenerateEvent {
        self.finished = true;
        GenerateEvent::Finished(GenerationFinished {
            finish,
            usage: self.usage.as_ref().map(gemini_response::usage),
        })
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
