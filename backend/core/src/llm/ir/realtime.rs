//! Realtime(实时语音)语义模型。
//!
//! OpenAI 专属能力:路由矩阵以 provider kind 直通,不做跨协议转换。
//! wire 编解码见 `llm::codec::realtime`,WebSocket 传输见 `llm::realtime`。

use serde::{Deserialize, Serialize};

use super::generation::{FunctionCall, InputItem, Instruction, ToolChoice, ToolDefinition};
use super::{ModelId, OperationFailure, Usage};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct RealtimeSession {
    pub model: ModelId,
    pub instructions: Vec<Instruction>,
    pub modalities: Vec<RealtimeModality>,
    pub voice: Option<String>,
    pub speed: Option<f64>,
    pub input_audio_format: Option<RealtimeAudioFormat>,
    pub output_audio_format: Option<RealtimeAudioFormat>,
    pub input_transcription: Option<RealtimeTranscription>,
    pub noise_reduction: Option<NoiseReductionMode>,
    pub turn_detection: Option<TurnDetection>,
    pub tools: Vec<ToolDefinition>,
    pub tool_choice: ToolChoice,
    pub max_output_tokens: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeModality {
    Text,
    Audio,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RealtimeAudioFormat {
    Pcm16 { rate: Option<u32> },
    G711Ulaw,
    G711Alaw,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct RealtimeTranscription {
    pub model: Option<String>,
    pub language: Option<String>,
    pub prompt: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum NoiseReductionMode {
    NearField,
    FarField,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnDetection {
    /// 显式关闭服务端断句(wire 上是 `turn_detection: null`)。
    Off,
    ServerVad {
        threshold: Option<f64>,
        prefix_padding_ms: Option<u32>,
        silence_duration_ms: Option<u32>,
        idle_timeout_ms: Option<u32>,
        create_response: Option<bool>,
        interrupt_response: Option<bool>,
    },
    SemanticVad {
        eagerness: Option<SemanticVadEagerness>,
        create_response: Option<bool>,
        interrupt_response: Option<bool>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum SemanticVadEagerness {
    Low,
    Medium,
    High,
    Auto,
}

/// 调用方 → 会话 的语义事件。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RealtimeClientEvent {
    UpdateSession {
        session: Box<RealtimeSession>,
    },
    /// base64 音频,采用会话输入格式。
    AppendAudio {
        audio: String,
    },
    CommitAudio,
    ClearAudio,
    CreateItem {
        item: InputItem,
    },
    /// 打断播放后截断助手音频 item,对齐会话对上下文的记忆。
    TruncatePlayback {
        item_id: String,
        audio_end_ms: u64,
    },
    CreateResponse,
    CancelResponse,
}

/// 会话 → 调用方 的语义事件。
///
/// 未建模的服务端事件(item 生命周期细节、限流通报等)被有意忽略,
/// 会话不因协议演进中断;这是 realtime 与 HTTP 流式解码的显式差异。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RealtimeServerEvent {
    SessionCreated,
    SessionUpdated,
    InputSpeechStarted {
        item_id: String,
    },
    InputSpeechStopped {
        item_id: String,
    },
    InputAudioCommitted {
        item_id: String,
    },
    InputTranscriptDelta {
        item_id: String,
        delta: String,
    },
    InputTranscriptCompleted {
        item_id: String,
        transcript: String,
    },
    InputTranscriptFailed {
        item_id: String,
        error: OperationFailure,
    },
    ResponseStarted {
        response_id: String,
    },
    /// base64 音频,采用会话输出格式。
    AudioDelta {
        item_id: String,
        delta: String,
    },
    AudioDone {
        item_id: String,
    },
    OutputTranscriptDelta {
        item_id: String,
        delta: String,
    },
    OutputTranscriptDone {
        item_id: String,
        transcript: String,
    },
    TextDelta {
        item_id: String,
        delta: String,
    },
    TextDone {
        item_id: String,
        text: String,
    },
    ToolCall {
        call: FunctionCall,
    },
    ResponseFinished {
        response_id: String,
        status: RealtimeFinish,
        usage: Option<Usage>,
    },
    Error {
        error: OperationFailure,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeFinish {
    Completed,
    Cancelled,
    Incomplete,
    Failed,
}
