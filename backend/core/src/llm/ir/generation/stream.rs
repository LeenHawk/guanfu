use serde::{Deserialize, Serialize};

use super::{Citation, FinishReason, OutputItem};
use crate::llm::ir::{ContentId, GenerationId, ModelId, OperationFailure, OutputId, Usage};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum GenerateEvent {
    Started(GenerationStarted),
    OutputStarted(OutputStarted),
    ContentStarted(ContentStarted),
    Delta(GenerateDelta),
    ContentFinished(ContentFinished),
    OutputFinished(OutputFinished),
    UsageUpdated(Usage),
    Finished(GenerationFinished),
    Failed(GenerationFailure),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct GenerationStarted {
    pub id: GenerationId,
    pub model: ModelId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct OutputStarted {
    pub output_index: u32,
    pub output_id: OutputId,
    pub kind: OutputKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Message,
    Reasoning,
    ToolCall,
    ToolExecution,
    Image,
    Audio,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ContentStarted {
    pub output_index: u32,
    pub content_index: u32,
    pub content_id: ContentId,
    pub kind: ContentKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Text,
    Refusal,
    ReasoningText,
    ToolInput,
    Audio,
    Transcript,
    Image,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum GenerateDelta {
    Text(ContentTextDelta),
    Refusal(ContentTextDelta),
    ReasoningSummary(ContentTextDelta),
    ReasoningText(ContentTextDelta),
    Compaction(CompactionDelta),
    FunctionArguments(JsonFragmentDelta),
    CustomToolInput(OutputTextDelta),
    Audio(BinaryDelta),
    Transcript(ContentTextDelta),
    Image(ImageDelta),
    Citation(CitationDelta),
    ToolExecution(ToolExecutionDelta),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ContentTextDelta {
    pub output_index: u32,
    pub content_index: u32,
    pub delta: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct OutputTextDelta {
    pub output_index: u32,
    pub delta: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct JsonFragmentDelta {
    pub output_index: u32,
    pub delta: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct CompactionDelta {
    pub output_index: u32,
    pub content: String,
    pub encrypted_content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct BinaryDelta {
    pub output_index: u32,
    pub content_index: u32,
    pub encoded: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ImageDelta {
    pub output_index: u32,
    pub content_index: u32,
    pub encoded: String,
    pub sequence: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct CitationDelta {
    pub output_index: u32,
    pub content_index: u32,
    pub citation: Citation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ToolExecutionDelta {
    pub output_index: u32,
    pub output_id: OutputId,
    pub state: super::ToolExecutionState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ContentFinished {
    pub output_index: u32,
    pub content_index: u32,
    pub content_id: ContentId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct OutputFinished {
    pub output_index: u32,
    pub item: OutputItem,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct GenerationFinished {
    pub finish: FinishReason,
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct GenerationFailure {
    pub error: OperationFailure,
    pub usage: Option<Usage>,
}
