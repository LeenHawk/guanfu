use gproxy_protocol::{ContentGenerationKind, Operation, OperationKey, Provider};
use serde::{Deserialize, Serialize};

/// 应用层能力集合。
///
/// `Audio`：gproxy-protocol 2.5 尚无音频操作（TTS/转写），此处占位，
/// 待上游支持后接入（见 AGENTS.md 的 gproxy issue 约定）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Models,
    GenerateContent,
    CountTokens,
    ImageGeneration,
    ImageEdit,
    Audio,
    Compact,
}

/// 渠道 provider 字符串 → gproxy Provider。
pub fn parse_provider(s: &str) -> Option<Provider> {
    match s {
        "openai" => Some(Provider::OpenAi),
        "claude" => Some(Provider::Claude),
        "gemini" => Some(Provider::Gemini),
        _ => None,
    }
}

/// provider 家族的原生内容生成线格式。
pub fn native_generation_kind(provider: Provider) -> ContentGenerationKind {
    match provider {
        Provider::Claude => ContentGenerationKind::ClaudeMessages,
        Provider::Gemini => ContentGenerationKind::GeminiGenerateContent,
        _ => ContentGenerationKind::OpenAiChatCompletions,
    }
}

/// 能力 + provider → gproxy OperationKey；协议尚不支持的能力返回 None。
pub fn operation_key(cap: Capability, provider: Provider, stream: bool) -> Option<OperationKey> {
    let key = match cap {
        Capability::Models => OperationKey::provider(Operation::ListModels, provider),
        Capability::GenerateContent => {
            let op = if stream {
                Operation::StreamGenerateContent
            } else {
                Operation::GenerateContent
            };
            OperationKey::content_generation(op, native_generation_kind(provider))
        }
        Capability::CountTokens => OperationKey::provider(Operation::CountTokens, provider),
        Capability::ImageGeneration => OperationKey::provider(Operation::CreateImage, provider),
        Capability::ImageEdit => OperationKey::provider(Operation::EditImage, provider),
        Capability::Compact => OperationKey::provider(Operation::CompactContent, provider),
        Capability::Audio => return None,
    };
    Some(key)
}
