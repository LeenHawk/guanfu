use serde::{Deserialize, Serialize};

use super::audio::{AudioRequest, AudioResponse};
use super::embeddings::{EmbeddingRequest, EmbeddingResponse};
use super::generation::{GenerateRequest, GenerateResponse};
use super::images::{ImageRequest, ImageResponse};
use super::models::{ModelRequest, ModelResponse};
use super::platform::{PlatformRequest, PlatformResponse};
use super::search::{SearchRequest, SearchResponse};
use super::tokens::{CountTokensRequest, CountTokensResponse};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OperationRequest {
    Models(ModelRequest),
    CountTokens(CountTokensRequest),
    Generate(GenerateRequest),
    Embeddings(EmbeddingRequest),
    Images(ImageRequest),
    Audio(AudioRequest),
    Search(SearchRequest),
    Platform(PlatformRequest),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OperationResponse {
    Models(ModelResponse),
    CountTokens(CountTokensResponse),
    Generate(GenerateResponse),
    Embeddings(EmbeddingResponse),
    Images(ImageResponse),
    Audio(AudioResponse),
    Search(SearchResponse),
    Platform(PlatformResponse),
}
