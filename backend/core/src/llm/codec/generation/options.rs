use gproxy_protocol::{ContentGenerationKind, OperationKey};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::llm::ir::generation::{
    GenerateRequest, GenerationProtocol, InputItem, ProtocolOptions, ReasoningPart,
};
use crate::CoreError;

/// OpenAI Responses 专用:采样直写 + reasoning 校验 + merge patch +
/// encrypted_content include。其余协议在各自 codec 内直写参数。
pub(super) fn apply(
    request: &GenerateRequest,
    target: OperationKey,
    body: &mut Value,
) -> Result<(), CoreError> {
    let body = body
        .as_object_mut()
        .ok_or_else(|| CoreError::InvalidProviderPayload {
            target,
            reason: "generation request body is not an object".into(),
        })?;
    insert_option(body, "temperature", request.sampling.temperature);
    insert_option(body, "top_p", request.sampling.top_p);
    if request
        .reasoning
        .as_ref()
        .is_some_and(|reasoning| reasoning.budget_tokens.is_some())
    {
        return Err(CoreError::IncompatibleRoute {
            target,
            fields: vec!["reasoning.budget_tokens".into()],
        });
    }
    apply_protocol_options(
        &request.protocol_options,
        ContentGenerationKind::OpenAiResponses,
        body,
    );
    ensure_reasoning_continuation(request, body);
    Ok(())
}

/// 请求 reasoning 或回放延续时补 include=["reasoning.encrypted_content"]。
fn ensure_reasoning_continuation(request: &GenerateRequest, body: &mut Map<String, Value>) {
    let has_continuation = request.input.iter().any(|item| {
        let InputItem::Reasoning { reasoning } = item else {
            return false;
        };
        reasoning.previous.parts.iter().any(|part| {
            matches!(
                part,
                ReasoningPart::Text {
                    continuation: Some(_),
                    ..
                } | ReasoningPart::Opaque { .. }
            )
        })
    });
    if request.reasoning.is_none() && !has_continuation {
        return;
    }
    if !body.get("include").is_some_and(Value::is_array) {
        body.insert("include".into(), Value::Array(Vec::new()));
    }
    let include = body
        .get_mut("include")
        .and_then(Value::as_array_mut)
        .expect("include was inserted as an array");
    if !include
        .iter()
        .any(|value| value.as_str() == Some("reasoning.encrypted_content"))
    {
        include.push(Value::String("reasoning.encrypted_content".into()));
    }
}

/// 各协议 merge patch(含保护路径),被四个原生 codec 复用。
pub(super) fn apply_protocol_options(
    options: &[ProtocolOptions],
    kind: ContentGenerationKind,
    body: &mut Map<String, Value>,
) {
    for option in options
        .iter()
        .filter(|option| protocol_matches(option.kind, kind))
    {
        let patch = option
            .patch
            .0
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Map<_, _>>();
        merge_object(body, &patch, kind, &mut Vec::new());
    }
}

fn protocol_matches(protocol: GenerationProtocol, kind: ContentGenerationKind) -> bool {
    matches!(
        (protocol, kind),
        (
            GenerationProtocol::OpenAiResponses,
            ContentGenerationKind::OpenAiResponses
        ) | (
            GenerationProtocol::OpenAiChatCompletions,
            ContentGenerationKind::OpenAiChatCompletions
        ) | (
            GenerationProtocol::ClaudeMessages,
            ContentGenerationKind::ClaudeMessages
        ) | (
            GenerationProtocol::GeminiGenerateContent,
            ContentGenerationKind::GeminiGenerateContent
        )
    )
}

fn merge_object(
    target: &mut Map<String, Value>,
    patch: &Map<String, Value>,
    kind: ContentGenerationKind,
    path: &mut Vec<String>,
) {
    for (key, patch_value) in patch {
        path.push(key.clone());
        if protected_path(kind, path) {
            path.pop();
            continue;
        }
        if has_protected_descendant(kind, path) && !patch_value.is_object() {
            path.pop();
            continue;
        }
        if patch_value.is_null() {
            target.remove(key);
        } else if let Value::Object(patch_object) = patch_value {
            let target_object = object_field(target, key);
            merge_object(target_object, patch_object, kind, path);
        } else {
            target.insert(key.clone(), patch_value.clone());
        }
        path.pop();
    }
}

fn protected_path(kind: ContentGenerationKind, path: &[String]) -> bool {
    if path.len() == 1
        && matches!(
            path[0].as_str(),
            "model"
                | "input"
                | "messages"
                | "contents"
                | "instructions"
                | "system"
                | "systemInstruction"
                | "tools"
                | "tool_choice"
                | "toolChoice"
                | "toolConfig"
                | "response_format"
                | "reasoning_effort"
                | "thinking"
                | "modalities"
                | "max_output_tokens"
                | "max_completion_tokens"
                | "max_tokens"
                | "max_tool_calls"
                | "stream"
        )
    {
        return true;
    }
    if path.len() != 2 {
        return false;
    }
    match kind {
        ContentGenerationKind::OpenAiResponses => matches!(
            (path[0].as_str(), path[1].as_str()),
            ("text", "format") | ("reasoning", "effort" | "summary")
        ),
        ContentGenerationKind::ClaudeMessages => matches!(
            (path[0].as_str(), path[1].as_str()),
            ("output_config", "effort" | "format")
        ),
        ContentGenerationKind::GeminiGenerateContent if path[0] == "generationConfig" => {
            matches!(
                path[1].as_str(),
                "maxOutputTokens"
                    | "responseMimeType"
                    | "responseSchema"
                    | "responseJsonSchema"
                    | "responseModalities"
                    | "thinkingConfig"
            )
        }
        _ => false,
    }
}

fn has_protected_descendant(kind: ContentGenerationKind, path: &[String]) -> bool {
    if path.len() != 1 {
        return false;
    }
    match kind {
        ContentGenerationKind::OpenAiResponses => {
            matches!(path[0].as_str(), "text" | "reasoning")
        }
        ContentGenerationKind::ClaudeMessages => path[0] == "output_config",
        ContentGenerationKind::GeminiGenerateContent => path[0] == "generationConfig",
        _ => false,
    }
}

fn object_field<'a>(body: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    if !body.get(key).is_some_and(Value::is_object) {
        body.insert(key.into(), Value::Object(Map::new()));
    }
    body.get_mut(key)
        .and_then(Value::as_object_mut)
        .expect("object field was inserted")
}

fn insert_option<T: Serialize>(body: &mut Map<String, Value>, key: &str, value: Option<T>) {
    if let Some(value) = value {
        body.insert(
            key.into(),
            serde_json::to_value(value).expect("generation option is serializable"),
        );
    }
}
