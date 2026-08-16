use serde::{Deserialize, Serialize};

use crate::llm::ir::{GenerationId, MediaSource, ModelId, OutputId, ToolCallId, Usage};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct GenerateResponse {
    pub id: GenerationId,
    pub model: ModelId,
    pub output: Vec<OutputItem>,
    pub finish: FinishReason,
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", content = "item", rename_all = "snake_case")]
pub enum OutputItem {
    Message(OutputMessage),
    Reasoning(ReasoningOutput),
    Compaction(CompactionOutput),
    ToolCall(ToolCall),
    ToolExecution(ToolExecution),
    McpApprovalRequest(McpApprovalRequest),
    Image(ImageArtifact),
    Audio(AudioArtifact),
}

impl OutputItem {
    pub fn id(&self) -> &OutputId {
        match self {
            Self::Message(value) => &value.id,
            Self::Reasoning(value) => &value.id,
            Self::Compaction(value) => &value.id,
            Self::ToolCall(value) => value.output_id(),
            Self::ToolExecution(value) => &value.id,
            Self::McpApprovalRequest(value) => &value.id,
            Self::Image(value) => &value.id,
            Self::Audio(value) => &value.id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct CompactionOutput {
    pub id: OutputId,
    pub content: Option<String>,
    pub encrypted_content: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct OutputMessage {
    pub id: OutputId,
    pub content: Vec<OutputContent>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputContent {
    Text {
        text: String,
        citations: Vec<Citation>,
    },
    Refusal {
        text: String,
    },
    SummaryText {
        text: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Citation {
    pub start: u64,
    pub end: u64,
    pub source: CitationSource,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CitationSource {
    Url { url: String },
    File { file_id: String },
    Document { document_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ReasoningOutput {
    pub id: OutputId,
    pub parts: Vec<ReasoningPart>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReasoningPart {
    Summary {
        text: String,
    },
    Text {
        text: String,
        continuation: Option<ReasoningContinuation>,
    },
    Opaque {
        continuation: ReasoningContinuation,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReasoningContinuation {
    OpenAiEncrypted { content: String },
    ClaudeSignature { signature: String },
    ClaudeRedacted { data: String },
    GeminiThoughtSignature { signature: String },
}

impl ReasoningContinuation {
    pub fn opaque_value(&self) -> &str {
        match self {
            Self::OpenAiEncrypted { content } => content,
            Self::ClaudeSignature { signature } | Self::GeminiThoughtSignature { signature } => {
                signature
            }
            Self::ClaudeRedacted { data } => data,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolCall {
    Function(FunctionCall),
    Custom(CustomToolCall),
    ComputerUse(ComputerActionCall),
    Shell(ShellCall),
    TextEditor(TextEditorCall),
    ToolSearch(ToolSearchCall),
}

impl ToolCall {
    pub fn output_id(&self) -> &OutputId {
        match self {
            Self::Function(v) => &v.id,
            Self::Custom(v) => &v.id,
            Self::ComputerUse(v) => &v.id,
            Self::Shell(v) => &v.id,
            Self::TextEditor(v) => &v.id,
            Self::ToolSearch(v) => &v.id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct FunctionCall {
    pub id: OutputId,
    pub call_id: ToolCallId,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct CustomToolCall {
    pub id: OutputId,
    pub call_id: ToolCallId,
    pub name: String,
    pub input: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct ComputerActionCall {
    pub id: OutputId,
    pub call_id: ToolCallId,
    pub action: serde_json::Value,
}

macro_rules! json_call {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
        pub struct $name {
            pub id: OutputId,
            pub call_id: ToolCallId,
            pub input: serde_json::Value,
        }
    };
}

json_call!(ShellCall);
json_call!(TextEditorCall);
json_call!(ToolSearchCall);

/// 服务端执行的托管工具（web_search / file_search / code_interpreter /
/// image_generation / mcp 等）的调用与结果，调用方无需回传输出。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct ToolExecution {
    pub id: OutputId,
    pub call_id: ToolCallId,
    pub state: ToolExecutionState,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Responses 的 mcp_approval_request：模型请求批准调用某个 MCP 工具，
/// 调用方以 [`super::McpApprovalResponse`] 回复。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct McpApprovalRequest {
    pub id: OutputId,
    pub server_label: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionState {
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct ImageArtifact {
    pub id: OutputId,
    pub source: MediaSource,
    pub revised_prompt: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct AudioArtifact {
    pub id: OutputId,
    pub source: MediaSource,
    pub transcript: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Refusal,
    Incomplete,
}
