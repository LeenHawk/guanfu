use serde::{Deserialize, Serialize};

use super::{MediaSource, ModelId, Usage};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AudioRequest {
    Speech(SpeechRequest),
    Transcribe(TranscriptionRequest),
    Translate(TranslationRequest),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct SpeechRequest {
    pub model: ModelId,
    pub input: String,
    pub voice: Voice,
    pub instructions: Option<String>,
    pub format: AudioFormat,
    pub speed: Option<f32>,
    pub mode: SpeechMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Voice(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum AudioFormat {
    Mp3,
    Opus,
    Aac,
    Flac,
    Wav,
    Pcm,
}

impl AudioFormat {
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::Opus => "audio/opus",
            Self::Aac => "audio/aac",
            Self::Flac => "audio/flac",
            Self::Wav => "audio/wav",
            Self::Pcm => "audio/pcm",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum SpeechMode {
    Complete,
    Stream,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct TranscriptionRequest {
    pub model: ModelId,
    pub audio: MediaSource,
    pub language: Option<String>,
    pub prompt: Option<String>,
    pub temperature: Option<f32>,
    pub timestamps: Vec<TimestampGranularity>,
    pub diarization: Option<DiarizationConfig>,
    pub mode: TranscriptionMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum TimestampGranularity {
    Word,
    Segment,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct DiarizationConfig {
    pub known_speakers: Vec<KnownSpeaker>,
    pub chunking: Option<AudioChunking>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct KnownSpeaker {
    pub name: String,
    pub reference: MediaSource,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AudioChunking {
    Auto,
    ServerVad {
        threshold: Option<f32>,
        prefix_padding_ms: Option<u32>,
        silence_duration_ms: Option<u32>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionMode {
    Complete,
    Stream,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct TranslationRequest {
    pub model: ModelId,
    pub audio: MediaSource,
    pub prompt: Option<String>,
    pub temperature: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AudioResponse {
    Speech(SpeechArtifact),
    Transcription(Transcription),
    Translation(Translation),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct SpeechArtifact {
    pub media_type: String,
    #[ts(type = "number[]")]
    pub bytes: bytes::Bytes,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct Transcription {
    pub text: String,
    pub language: Option<String>,
    pub duration_seconds: Option<f64>,
    pub words: Vec<TranscriptWord>,
    pub segments: Vec<TranscriptSegment>,
    pub usage: Option<AudioUsage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct Translation {
    pub text: String,
    pub source_language: Option<String>,
    pub duration_seconds: Option<f64>,
    pub segments: Vec<TranscriptSegment>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct TranscriptWord {
    pub text: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub speaker: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct TranscriptSegment {
    pub id: String,
    pub text: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub speaker: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AudioUsage {
    Tokens(Usage),
    Duration { seconds: f64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SpeechEvent {
    Started {
        media_type: String,
    },
    AudioDelta {
        #[ts(type = "number[]")]
        bytes: bytes::Bytes,
    },
    Finished,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TranscriptionEvent {
    TextDelta { text: String },
    Segment(TranscriptSegment),
    Finished(Transcription),
}
