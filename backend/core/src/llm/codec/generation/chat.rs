//! IR → OpenAI Chat Completions 原生编码(typed wire,直达,不经 canonical 两跳)。

use gproxy_protocol::openai as wire;
use gproxy_protocol::{ContentGenerationKind, OperationKey};
use serde_json::Value;

use super::claude::wire_build_error;
use super::options;
use crate::llm::ir::generation::*;
use crate::CoreError;

pub(super) fn encode_request(
    request: &GenerateRequest,
    target: OperationKey,
) -> Result<Value, CoreError> {
    validate_options(request, target)?;
    let sampling = &request.sampling;
    let tools = super::chat_tools::encode_tools(&request.tools, target)?;
    // 与 gproxy 不同:tool_choice 仅在存在 function/custom 工具时发出(live 兼容)。
    let tool_choice = (!tools.tools.is_empty())
        .then(|| super::chat_tools::encode_tool_choice(&request.tool_choice, &request.tools))
        .transpose()?;
    let body = wire::ChatCompletionRequest::builder()
        .messages(super::chat_content::encode_messages(request, target)?)
        .model(request.model.0.clone().into())
        .frequency_penalty(sampling.frequency_penalty.map(f64::from))
        .max_completion_tokens(request.limits.max_output_tokens.map(u64_to_u32))
        .modalities(encode_modalities(&request.modalities))
        .presence_penalty(sampling.presence_penalty.map(f64::from))
        .reasoning_effort(
            request
                .reasoning
                .as_ref()
                .and_then(|r| r.effort)
                .map(effort),
        )
        .response_format(encode_response_format(&request.output)?)
        .seed(sampling.seed)
        .stop((!sampling.stop.is_empty()).then(|| wire::StringOrList::List(sampling.stop.clone())))
        .stream((request.mode == GenerateMode::Stream).then_some(true))
        .stream_options(encode_stream_options(request.mode)?)
        .temperature(sampling.temperature.map(f64::from))
        .tool_choice(tool_choice)
        .tools((!tools.tools.is_empty()).then_some(tools.tools))
        .top_p(sampling.top_p.map(f64::from))
        .web_search_options(tools.web_search_options)
        .build()
        .map_err(wire_build_error)?;
    let mut value = serde_json::to_value(&body)?;
    let map = value
        .as_object_mut()
        .expect("chat request serializes as an object");
    options::apply_protocol_options(
        &request.protocol_options,
        ContentGenerationKind::OpenAiChatCompletions,
        map,
    );
    Ok(value)
}

/// budget/summary 沿 options.rs 原 Chat 分支判定;max_tool_calls 与 image
/// 模态在 Chat 无落点,显式 route incompatible 参与回退(旧两跳为静默丢弃)。
fn validate_options(request: &GenerateRequest, target: OperationKey) -> Result<(), CoreError> {
    let mut fields = Vec::new();
    if let Some(reasoning) = &request.reasoning {
        if reasoning.budget_tokens.is_some() {
            fields.push("reasoning.budget_tokens".to_owned());
        }
        if reasoning.summary.is_some() {
            fields.push("reasoning.summary".to_owned());
        }
    }
    if request.limits.max_tool_calls.is_some() {
        fields.push("limits.max_tool_calls".to_owned());
    }
    if request.modalities.contains(&OutputModality::Image) {
        fields.push("modalities.image".to_owned());
    }
    if fields.is_empty() {
        Ok(())
    } else {
        Err(CoreError::IncompatibleRoute { target, fields })
    }
}

/// 纯文本缺省不发;含 audio 时直写 modalities(voice/format 无 IR 落点,
/// 由 protocol_options patch 补 `audio` 参数)。
fn encode_modalities(modalities: &[OutputModality]) -> Option<Vec<wire::TextOrAudioModality>> {
    modalities.contains(&OutputModality::Audio).then(|| {
        modalities
            .iter()
            .filter_map(|modality| match modality {
                OutputModality::Text => Some(wire::TextOrAudioModality::Text),
                OutputModality::Audio => Some(wire::TextOrAudioModality::Audio),
                OutputModality::Image => None,
            })
            .collect()
    })
}

fn effort(effort: ReasoningEffort) -> wire::ReasoningEffort {
    match effort {
        ReasoningEffort::None => wire::ReasoningEffort::None,
        ReasoningEffort::Minimal => wire::ReasoningEffort::Minimal,
        ReasoningEffort::Low => wire::ReasoningEffort::Low,
        ReasoningEffort::Medium => wire::ReasoningEffort::Medium,
        ReasoningEffort::High => wire::ReasoningEffort::High,
        ReasoningEffort::XHigh => wire::ReasoningEffort::XHigh,
        ReasoningEffort::Max => wire::ReasoningEffort::Max,
    }
}

/// Text 约束缺省不发(与 gproxy 恒发 `{"type":"text"}` 不同,语义等价)。
fn encode_response_format(
    output: &OutputConstraint,
) -> Result<Option<wire::ChatResponseFormat>, CoreError> {
    Ok(match output {
        OutputConstraint::Text => None,
        OutputConstraint::JsonObject => Some(wire::ChatResponseFormat::JsonObject(
            wire::JsonObjectResponseFormat::builder()
                .type_(wire::JsonObjectResponseFormatType::JsonObject)
                .build()
                .map_err(wire_build_error)?,
        )),
        OutputConstraint::JsonSchema {
            name,
            schema,
            strict,
        } => Some(wire::ChatResponseFormat::ChatJsonSchema(
            wire::ChatJsonSchemaFormat::builder()
                .type_(wire::JsonSchemaResponseFormatType::JsonSchema)
                .json_schema(
                    wire::JsonSchemaFormat::builder()
                        .name(name.clone())
                        .schema(Some(
                            serde_json::from_value(schema.0.clone()).unwrap_or_default(),
                        ))
                        .strict(Some(*strict))
                        .build()
                        .map_err(wire_build_error)?,
                )
                .build()
                .map_err(wire_build_error)?,
        )),
    })
}

/// 流式请求显式要求 usage chunk(旧两跳不发 stream_options,usage 会缺失)。
fn encode_stream_options(mode: GenerateMode) -> Result<Option<wire::StreamOptions>, CoreError> {
    if mode != GenerateMode::Stream {
        return Ok(None);
    }
    Ok(Some(
        wire::StreamOptions::builder()
            .include_usage(Some(true))
            .build()
            .map_err(wire_build_error)?,
    ))
}

fn u64_to_u32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
