use serde::{Deserialize, Serialize};

use super::generation::{InputItem, Instruction, OutputItem};
use super::realtime::RealtimeSession;
use super::{ModelId, Usage};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlatformRequest {
    Compact(CompactRequest),
    CreateConversation(CreateConversationRequest),
    CreateRealtimeCall(CreateRealtimeCallRequest),
    ConnectRealtime(ConnectRealtimeRequest),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct CompactRequest {
    pub model: ModelId,
    pub input: Vec<InputItem>,
    pub instructions: Vec<Instruction>,
    pub max_output_tokens: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct CreateConversationRequest {
    pub items: Vec<InputItem>,
    pub metadata: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct CreateRealtimeCallRequest {
    pub session: RealtimeSession,
    pub offer_sdp: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct ConnectRealtimeRequest {
    pub session: RealtimeSession,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlatformResponse {
    Compact(CompactResponse),
    Conversation(Conversation),
    RealtimeCall(RealtimeCall),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct CompactResponse {
    pub output: Vec<OutputItem>,
    pub encrypted_content: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct Conversation {
    pub id: String,
    pub metadata: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct RealtimeCall {
    /// 来自 `Location` 响应头的 call id;上游未返回时为 None。
    pub id: Option<String>,
    pub answer_sdp: String,
}
