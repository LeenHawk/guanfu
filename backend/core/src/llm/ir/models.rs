use serde::{Deserialize, Serialize};

use super::ModelId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelRequest {
    List(ListModelsRequest),
    Get(GetModelRequest),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ListModelsRequest {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct GetModelRequest {
    pub id: ModelId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelResponse {
    List(ModelPage),
    One(Model),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ModelPage {
    pub models: Vec<Model>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Model {
    pub id: ModelId,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub created_at: Option<i64>,
    pub capabilities: Option<Vec<ModelCapability>>,
    pub context_limit: Option<u64>,
    pub output_limit: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapability {
    GenerateText,
    GenerateImage,
    ImageInput,
    AudioInput,
    AudioOutput,
    Embeddings,
    Tools,
    Reasoning,
}
