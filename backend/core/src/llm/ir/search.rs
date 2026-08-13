use serde::{Deserialize, Serialize};

use super::{ModelId, Usage};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchRequest {
    Web(WebSearchRequest),
    Rerank(RerankRequest),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebSearchRequest {
    pub model: Option<ModelId>,
    pub query: String,
    pub max_results: Option<u32>,
    pub allowed_domains: Vec<String>,
    pub blocked_domains: Vec<String>,
    pub location: Option<SearchLocation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchLocation {
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub timezone: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RerankRequest {
    pub model: ModelId,
    pub query: String,
    pub documents: Vec<RerankDocument>,
    pub top_n: Option<u32>,
    pub return_documents: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RerankDocument {
    pub id: Option<String>,
    pub text: String,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchResponse {
    Web(WebSearchResponse),
    Rerank(RerankResponse),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebSearchResponse {
    pub results: Vec<WebSearchResult>,
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebSearchResult {
    pub url: String,
    pub title: String,
    pub snippet: Option<String>,
    pub published_at: Option<String>,
    pub score: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RerankResponse {
    pub results: Vec<RerankResult>,
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RerankResult {
    pub index: u32,
    pub relevance_score: f32,
    pub document: Option<RerankDocument>,
}
