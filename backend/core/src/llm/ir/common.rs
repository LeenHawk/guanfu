use std::collections::BTreeMap;

use bytes::Bytes;
use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);
    };
}

string_id!(ModelId);
string_id!(GenerationId);
string_id!(OutputId);
string_id!(ContentId);
string_id!(ToolCallId);
string_id!(FileId);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaType(pub String);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MediaSource {
    Url { url: String },
    Data { media_type: MediaType, bytes: Bytes },
    File { id: FileId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JsonSchema(pub serde_json::Value);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    TextGeneration,
    ImageInput,
    AudioInput,
    FileInput,
    Reasoning,
    StructuredOutput,
    FunctionTool,
    CustomTool,
    WebSearchTool,
    WebFetchTool,
    FileSearchTool,
    ComputerUseTool,
    CodeExecutionTool,
    ShellTool,
    TextEditorTool,
    ImageGenerationTool,
    McpTool,
    MemoryTool,
    ToolSearchTool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub details: BTreeMap<String, serde_json::Value>,
}
