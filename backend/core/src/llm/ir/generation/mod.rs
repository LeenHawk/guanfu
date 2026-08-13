mod input;
mod output;
mod stream;
mod tools;

pub use input::*;
pub use output::*;
pub use stream::*;
pub use tools::*;

use serde::{Deserialize, Serialize};

use super::ModelId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerateMode {
    Complete,
    Stream,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub model: ModelId,
    pub input: Vec<InputItem>,
    pub instructions: Vec<Instruction>,
    pub tools: Vec<ToolDefinition>,
    pub tool_choice: ToolChoice,
    pub output: OutputConstraint,
    pub sampling: SamplingOptions,
    pub limits: GenerationLimits,
    pub modalities: Vec<OutputModality>,
    pub mode: GenerateMode,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SamplingOptions {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub seed: Option<i64>,
    pub stop: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationLimits {
    pub max_output_tokens: Option<u64>,
    pub max_tool_calls: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputModality {
    Text,
    Audio,
    Image,
}
