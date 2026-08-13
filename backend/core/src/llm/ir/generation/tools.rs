use serde::{Deserialize, Serialize};

use crate::llm::ir::{JsonSchema, MediaSource};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolDefinition {
    Function(FunctionTool),
    Custom(CustomTool),
    WebSearch(WebSearchTool),
    WebFetch(WebFetchTool),
    FileSearch(FileSearchTool),
    ComputerUse(ComputerUseTool),
    CodeExecution(CodeExecutionTool),
    Shell(ShellTool),
    TextEditor(TextEditorTool),
    ImageGeneration(ImageGenerationTool),
    Mcp(McpTool),
    Memory(MemoryTool),
    ToolSearch(ToolSearchTool),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct FunctionTool {
    pub name: String,
    pub description: Option<String>,
    pub parameters: JsonSchema,
    pub strict: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct CustomTool {
    pub name: String,
    pub description: Option<String>,
    pub input_format: CustomToolInputFormat,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CustomToolInputFormat {
    Text,
    Grammar {
        syntax: GrammarSyntax,
        definition: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum GrammarSyntax {
    Lark,
    Regex,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct WebSearchTool {
    pub search_context_size: Option<SearchContextSize>,
    pub user_location: Option<UserLocation>,
    pub allowed_domains: Vec<String>,
    pub blocked_domains: Vec<String>,
    pub max_uses: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum SearchContextSize {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct UserLocation {
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub timezone: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct WebFetchTool {
    pub allowed_domains: Vec<String>,
    pub blocked_domains: Vec<String>,
    pub max_uses: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct FileSearchTool {
    pub vector_store_ids: Vec<String>,
    pub max_results: Option<u32>,
    pub ranking: Option<FileSearchRanking>,
    pub filters: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct FileSearchRanking {
    pub ranker: Option<String>,
    pub score_threshold: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ComputerUseTool {
    pub environment: ComputerEnvironment,
    pub display_width: u32,
    pub display_height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum ComputerEnvironment {
    Browser,
    Mac,
    Windows,
    Linux,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct CodeExecutionTool {
    pub container: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ShellTool {
    pub environment: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct TextEditorTool {
    pub max_characters: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ImageGenerationTool {
    pub size: Option<String>,
    pub quality: Option<String>,
    pub background: Option<String>,
    pub output_format: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct McpTool {
    pub server_label: String,
    pub server_url: String,
    pub allowed_tools: Vec<String>,
    pub approval: McpApproval,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum McpApproval {
    Always,
    Never,
    PerTool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct MemoryTool {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ToolSearchTool {
    pub deferred_tools: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolChoice {
    Auto,
    None,
    Required,
    Tool {
        name: String,
    },
    Allowed {
        names: Vec<String>,
        mode: AllowedToolMode,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum AllowedToolMode {
    Auto,
    Required,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputConstraint {
    Text,
    JsonObject,
    JsonSchema {
        name: String,
        schema: JsonSchema,
        strict: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolResultContent {
    Text { text: String },
    Image { source: MediaSource },
    Json { value: serde_json::Value },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolOutcome {
    Success {
        content: Vec<ToolResultContent>,
    },
    Error {
        code: Option<String>,
        message: String,
    },
}
