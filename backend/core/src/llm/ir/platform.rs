use serde::{Deserialize, Serialize};

use super::generation::{GenerateRequest, InputItem, Instruction, OutputItem};
use super::{ModelId, Usage};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlatformRequest {
    Compact(CompactRequest),
    CreateConversation(CreateConversationRequest),
    CreateRealtimeCall(CreateRealtimeCallRequest),
    ConnectRealtime(ConnectRealtimeRequest),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompactRequest {
    pub model: ModelId,
    pub input: Vec<InputItem>,
    pub instructions: Vec<Instruction>,
    pub max_output_tokens: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateConversationRequest {
    pub items: Vec<InputItem>,
    pub metadata: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateRealtimeCallRequest {
    pub session: RealtimeSession,
    pub offer_sdp: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnectRealtimeRequest {
    pub model: ModelId,
    pub session: RealtimeSession,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RealtimeSession {
    pub model: ModelId,
    pub instructions: Vec<Instruction>,
    pub modalities: Vec<RealtimeModality>,
    pub voice: Option<String>,
    pub input_audio_format: Option<String>,
    pub output_audio_format: Option<String>,
    pub turn_detection: Option<TurnDetection>,
    pub generation: Option<Box<GenerateRequest>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeModality {
    Text,
    Audio,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnDetection {
    ServerVad {
        threshold: Option<f32>,
        prefix_padding_ms: Option<u32>,
        silence_duration_ms: Option<u32>,
    },
    SemanticVad {
        eagerness: Option<SemanticVadEagerness>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticVadEagerness {
    Low,
    Medium,
    High,
    Auto,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlatformResponse {
    Compact(CompactResponse),
    Conversation(Conversation),
    RealtimeCall(RealtimeCall),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompactResponse {
    pub output: Vec<OutputItem>,
    pub encrypted_content: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub items: Vec<OutputItem>,
    pub metadata: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeCall {
    pub id: String,
    pub answer_sdp: String,
}
