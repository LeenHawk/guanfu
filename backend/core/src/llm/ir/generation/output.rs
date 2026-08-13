use serde::{Deserialize, Serialize};

use crate::llm::ir::{GenerationId, MediaSource, ModelId, OutputId, ToolCallId, Usage};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerateResponse {
    pub id: GenerationId,
    pub model: ModelId,
    pub output: Vec<OutputItem>,
    pub finish: FinishReason,
    pub usage: Usage,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputItem {
    Message(OutputMessage),
    Reasoning(ReasoningOutput),
    ToolCall(ToolCall),
    ToolExecution(ToolExecution),
    Image(ImageArtifact),
    Audio(AudioArtifact),
}

impl OutputItem {
    pub fn id(&self) -> &OutputId {
        match self {
            Self::Message(value) => &value.id,
            Self::Reasoning(value) => &value.id,
            Self::ToolCall(value) => value.output_id(),
            Self::ToolExecution(value) => &value.id,
            Self::Image(value) => &value.id,
            Self::Audio(value) => &value.id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputMessage {
    pub id: OutputId,
    pub content: Vec<OutputContent>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputContent {
    Text {
        text: String,
        citations: Vec<Citation>,
    },
    Refusal {
        text: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    pub start: u64,
    pub end: u64,
    pub source: CitationSource,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CitationSource {
    Url { url: String },
    File { file_id: String },
    Document { document_id: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReasoningOutput {
    pub id: OutputId,
    pub summary: Vec<String>,
    pub encrypted_content: Option<String>,
    pub signature: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolCall {
    Function(FunctionCall),
    Custom(CustomToolCall),
    WebSearch(HostedToolCall),
    WebFetch(HostedToolCall),
    FileSearch(HostedToolCall),
    ComputerUse(ComputerActionCall),
    CodeExecution(CodeExecutionCall),
    Shell(ShellCall),
    TextEditor(TextEditorCall),
    ImageGeneration(ImageGenerationCall),
    Mcp(McpCall),
    Memory(MemoryCall),
    ToolSearch(ToolSearchCall),
}

impl ToolCall {
    pub fn output_id(&self) -> &OutputId {
        match self {
            Self::Function(v) => &v.id,
            Self::Custom(v) => &v.id,
            Self::WebSearch(v) | Self::WebFetch(v) | Self::FileSearch(v) => &v.id,
            Self::ComputerUse(v) => &v.id,
            Self::CodeExecution(v) => &v.id,
            Self::Shell(v) => &v.id,
            Self::TextEditor(v) => &v.id,
            Self::ImageGeneration(v) => &v.id,
            Self::Mcp(v) => &v.id,
            Self::Memory(v) => &v.id,
            Self::ToolSearch(v) => &v.id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub id: OutputId,
    pub call_id: ToolCallId,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomToolCall {
    pub id: OutputId,
    pub call_id: ToolCallId,
    pub name: String,
    pub input: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostedToolCall {
    pub id: OutputId,
    pub call_id: ToolCallId,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComputerActionCall {
    pub id: OutputId,
    pub call_id: ToolCallId,
    pub action: serde_json::Value,
}

macro_rules! json_call {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        pub struct $name {
            pub id: OutputId,
            pub call_id: ToolCallId,
            pub input: serde_json::Value,
        }
    };
}

json_call!(CodeExecutionCall);
json_call!(ShellCall);
json_call!(TextEditorCall);
json_call!(ImageGenerationCall);
json_call!(MemoryCall);
json_call!(ToolSearchCall);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpCall {
    pub id: OutputId,
    pub call_id: ToolCallId,
    pub server_label: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolExecution {
    pub id: OutputId,
    pub call_id: ToolCallId,
    pub state: ToolExecutionState,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionState {
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageArtifact {
    pub id: OutputId,
    pub source: MediaSource,
    pub revised_prompt: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioArtifact {
    pub id: OutputId,
    pub source: MediaSource,
    pub transcript: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Refusal,
    Incomplete,
}
