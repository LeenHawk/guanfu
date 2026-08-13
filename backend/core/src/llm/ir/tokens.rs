use serde::{Deserialize, Serialize};

use super::generation::{InputItem, Instruction, ToolDefinition};
use super::ModelId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CountTokensRequest {
    pub model: ModelId,
    pub input: TokenCountInput,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TokenCountInput {
    Text { values: Vec<String> },
    Generation(GenerationTokenInput),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerationTokenInput {
    pub input: Vec<InputItem>,
    pub instructions: Vec<Instruction>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountTokensResponse {
    pub input_tokens: u64,
}
