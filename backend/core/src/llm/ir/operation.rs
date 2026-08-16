use serde::{Deserialize, Serialize};

use super::audio::{AudioRequest, AudioResponse};
use super::embeddings::{EmbeddingRequest, EmbeddingResponse};
use super::generation::{GenerateRequest, GenerateResponse};
use super::images::{ImageRequest, ImageResponse};
use super::models::{ModelRequest, ModelResponse};
use super::platform::{PlatformRequest, PlatformResponse};
use super::search::{SearchRequest, SearchResponse};
use super::tokens::{CountTokensRequest, CountTokensResponse};
use super::video::{VideoRequest, VideoResponse};
use gproxy_protocol::Operation;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "operation", content = "request", rename_all = "snake_case")]
pub enum OperationRequest {
    Models(ModelRequest),
    CountTokens(CountTokensRequest),
    Generate(GenerateRequest),
    Embeddings(EmbeddingRequest),
    Images(ImageRequest),
    Audio(AudioRequest),
    Search(SearchRequest),
    Video(VideoRequest),
    Platform(PlatformRequest),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "operation", content = "response", rename_all = "snake_case")]
pub enum OperationResponse {
    Models(ModelResponse),
    CountTokens(CountTokensResponse),
    Generate(GenerateResponse),
    Embeddings(EmbeddingResponse),
    Images(ImageResponse),
    Audio(AudioResponse),
    Search(SearchResponse),
    Video(VideoResponse),
    Platform(PlatformResponse),
}

impl OperationRequest {
    pub const fn operation(&self) -> Operation {
        match self {
            Self::Models(ModelRequest::List(_)) => Operation::ListModels,
            Self::Models(ModelRequest::Get(_)) => Operation::GetModel,
            Self::CountTokens(_) => Operation::CountTokens,
            Self::Generate(request) => match request.mode {
                super::generation::GenerateMode::Complete => Operation::GenerateContent,
                super::generation::GenerateMode::Stream => Operation::StreamGenerateContent,
            },
            Self::Embeddings(_) => Operation::CreateEmbedding,
            Self::Images(ImageRequest::Generate(_)) => Operation::CreateImage,
            Self::Images(ImageRequest::Edit(_)) => Operation::EditImage,
            Self::Audio(AudioRequest::Speech(_)) => Operation::CreateSpeech,
            Self::Audio(AudioRequest::Transcribe(_)) => Operation::CreateTranscription,
            Self::Audio(AudioRequest::Translate(_)) => Operation::CreateTranslation,
            Self::Search(SearchRequest::Web(_)) => Operation::WebSearch,
            Self::Search(SearchRequest::Rerank(_)) => Operation::Rerank,
            Self::Video(VideoRequest::Create(_)) => Operation::CreateVideo,
            Self::Video(VideoRequest::Retrieve(_)) => Operation::RetrieveVideo,
            Self::Video(VideoRequest::List(_)) => Operation::ListVideos,
            Self::Video(VideoRequest::Delete(_)) => Operation::DeleteVideo,
            Self::Video(VideoRequest::DownloadContent(_)) => Operation::DownloadVideoContent,
            Self::Platform(PlatformRequest::Compact(_)) => Operation::CompactContent,
            Self::Platform(PlatformRequest::CreateConversation(_)) => Operation::CreateConversation,
            Self::Platform(PlatformRequest::CreateRealtimeCall(_)) => Operation::CreateRealtimeCall,
            Self::Platform(PlatformRequest::ConnectRealtime(_)) => Operation::ConnectRealtime,
        }
    }

    pub fn model_id(&self) -> Option<&str> {
        match self {
            Self::Models(ModelRequest::Get(request)) => Some(&request.id.0),
            Self::Models(ModelRequest::List(_)) => None,
            Self::CountTokens(request) => Some(&request.model.0),
            Self::Generate(request) => Some(&request.model.0),
            Self::Embeddings(request) => Some(&request.model.0),
            Self::Images(ImageRequest::Generate(request)) => Some(&request.model.0),
            Self::Images(ImageRequest::Edit(request)) => Some(&request.model.0),
            Self::Audio(AudioRequest::Speech(request)) => Some(&request.model.0),
            Self::Audio(AudioRequest::Transcribe(request)) => Some(&request.model.0),
            Self::Audio(AudioRequest::Translate(request)) => Some(&request.model.0),
            Self::Search(SearchRequest::Web(request)) => {
                request.model.as_ref().map(|model| model.0.as_str())
            }
            Self::Search(SearchRequest::Rerank(request)) => Some(&request.model.0),
            // 视频资源 id 沿用端点合成的 model 参数槽约定。
            Self::Video(VideoRequest::Create(request)) => Some(&request.model.0),
            Self::Video(VideoRequest::Retrieve(request)) => Some(&request.id),
            Self::Video(VideoRequest::List(_)) => None,
            Self::Video(VideoRequest::Delete(request)) => Some(&request.id),
            Self::Video(VideoRequest::DownloadContent(request)) => Some(&request.id),
            Self::Platform(PlatformRequest::Compact(request)) => Some(&request.model.0),
            Self::Platform(PlatformRequest::CreateConversation(_)) => None,
            Self::Platform(PlatformRequest::CreateRealtimeCall(request)) => {
                Some(&request.session.model.0)
            }
            Self::Platform(PlatformRequest::ConnectRealtime(request)) => {
                Some(&request.session.model.0)
            }
        }
    }
}
