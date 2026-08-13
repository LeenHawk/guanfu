use serde::{Deserialize, Serialize};

use super::{ModelId, Usage};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct EmbeddingRequest {
    pub model: ModelId,
    pub input: EmbeddingInput,
    pub dimensions: Option<u32>,
    pub task: Option<EmbeddingTask>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EmbeddingInput {
    Text { value: String },
    TextBatch { values: Vec<String> },
    Tokens { value: Vec<u32> },
    TokenBatch { values: Vec<Vec<u32>> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingTask {
    RetrievalQuery,
    RetrievalDocument,
    SemanticSimilarity,
    Classification,
    Clustering,
    QuestionAnswering,
    FactVerification,
    CodeRetrievalQuery,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct EmbeddingResponse {
    pub model: Option<ModelId>,
    pub vectors: Vec<EmbeddingVector>,
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct EmbeddingVector {
    pub index: u32,
    pub values: Vec<f32>,
}
