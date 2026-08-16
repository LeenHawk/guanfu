use serde::{Deserialize, Serialize};

use super::{ReasoningOutput, ToolOutcome};
use crate::llm::ir::{FileId, MediaSource, ToolCallId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputItem {
    Message { message: Message },
    ToolResult { result: ToolResult },
    McpApproval { approval: McpApprovalResponse },
    Reasoning { reasoning: ReasoningInput },
}

/// 对 [`super::McpApprovalRequest`] 的答复。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct McpApprovalResponse {
    pub approval_request_id: String,
    pub approve: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct Instruction {
    pub role: InstructionRole,
    pub content: Vec<InputContent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum InstructionRole {
    System,
    Developer,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct Message {
    pub role: MessageRole,
    pub content: Vec<InputContent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputContent {
    Text {
        text: String,
    },
    Image {
        source: MediaSource,
        detail: ImageDetail,
    },
    Audio {
        source: MediaSource,
    },
    File {
        source: FileSource,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum ImageDetail {
    Low,
    High,
    #[default]
    Auto,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileSource {
    Media {
        source: MediaSource,
    },
    Id {
        id: FileId,
    },
    Text {
        filename: Option<String>,
        text: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct ToolResult {
    pub call_id: ToolCallId,
    pub kind: ToolResultKind,
    pub outcome: ToolOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultKind {
    Function,
    Custom,
    ComputerUse,
    CodeExecution,
    Shell,
    TextEditor,
    Memory,
    ToolSearch,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct ReasoningInput {
    pub previous: ReasoningOutput,
}
