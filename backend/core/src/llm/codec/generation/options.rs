use gproxy_protocol::{ContentGenerationKind, OperationKey, OperationKind};
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::llm::ir::generation::{GenerateRequest, GenerationProtocol, ProtocolOptions};
use crate::CoreError;

pub(super) fn apply(
    request: &GenerateRequest,
    target: OperationKey,
    body: &mut Value,
) -> Result<(), CoreError> {
    let kind = content_kind(target)?;
    let body = body
        .as_object_mut()
        .ok_or_else(|| invalid_body(target, "generation request body is not an object"))?;
    apply_sampling(request, kind, body);
    apply_reasoning(request, kind, body);
    apply_protocol_options(&request.protocol_options, kind, body);
    Ok(())
}

fn apply_sampling(
    request: &GenerateRequest,
    kind: ContentGenerationKind,
    body: &mut Map<String, Value>,
) {
    let sampling = &request.sampling;
    match kind {
        ContentGenerationKind::OpenAiResponses
        | ContentGenerationKind::OpenAiResponsesWebSocket => {
            insert_option(body, "temperature", sampling.temperature);
            insert_option(body, "top_p", sampling.top_p);
        }
        ContentGenerationKind::OpenAiChatCompletions => {
            insert_option(body, "temperature", sampling.temperature);
            insert_option(body, "top_p", sampling.top_p);
            insert_option(body, "seed", sampling.seed);
            insert_option(body, "frequency_penalty", sampling.frequency_penalty);
            insert_option(body, "presence_penalty", sampling.presence_penalty);
            insert_stop(body, "stop", &sampling.stop);
        }
        ContentGenerationKind::ClaudeMessages => {
            insert_option(body, "temperature", sampling.temperature);
            insert_option(body, "top_p", sampling.top_p);
            insert_option(body, "top_k", sampling.top_k);
            insert_stop(body, "stop_sequences", &sampling.stop);
        }
        ContentGenerationKind::GeminiGenerateContent => {
            let config = object_field(body, "generationConfig");
            insert_option(config, "temperature", sampling.temperature);
            insert_option(config, "topP", sampling.top_p);
            insert_option(config, "topK", sampling.top_k);
            insert_option(config, "seed", sampling.seed);
            insert_option(config, "frequencyPenalty", sampling.frequency_penalty);
            insert_option(config, "presencePenalty", sampling.presence_penalty);
            insert_stop(config, "stopSequences", &sampling.stop);
        }
        _ => {}
    }
}

fn apply_reasoning(
    request: &GenerateRequest,
    kind: ContentGenerationKind,
    body: &mut Map<String, Value>,
) {
    let Some(reasoning) = &request.reasoning else {
        return;
    };
    match kind {
        ContentGenerationKind::ClaudeMessages => {
            if let Some(budget_tokens) = reasoning.budget_tokens {
                body.insert(
                    "thinking".into(),
                    json!({"type":"enabled","budget_tokens":budget_tokens}),
                );
            }
            if reasoning.summary.is_some() {
                let thinking = object_field(body, "thinking");
                thinking
                    .entry("type")
                    .or_insert_with(|| Value::String("adaptive".into()));
                thinking.insert("display".into(), Value::String("summarized".into()));
            }
        }
        ContentGenerationKind::GeminiGenerateContent
            if reasoning.budget_tokens.is_some() || reasoning.summary.is_some() =>
        {
            let config = object_field(body, "generationConfig");
            let thinking = object_field(config, "thinkingConfig");
            insert_option(thinking, "thinkingBudget", reasoning.budget_tokens);
            if reasoning.summary.is_some() {
                thinking.insert("includeThoughts".into(), Value::Bool(true));
            }
        }
        _ => {}
    }
}

fn apply_protocol_options(
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
            .filter(|(key, _)| !protected_field(key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Map<_, _>>();
        merge_object(body, &patch);
    }
}

fn protocol_matches(protocol: GenerationProtocol, kind: ContentGenerationKind) -> bool {
    matches!(
        (protocol, kind),
        (
            GenerationProtocol::OpenAiResponses,
            ContentGenerationKind::OpenAiResponses
        ) | (
            GenerationProtocol::OpenAiResponsesWebSocket,
            ContentGenerationKind::OpenAiResponsesWebSocket
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

fn merge_object(target: &mut Map<String, Value>, patch: &Map<String, Value>) {
    for (key, patch_value) in patch {
        if patch_value.is_null() {
            target.remove(key);
        } else if let Value::Object(patch_object) = patch_value {
            let target_object = object_field(target, key);
            merge_object(target_object, patch_object);
        } else {
            target.insert(key.clone(), patch_value.clone());
        }
    }
}

fn protected_field(key: &str) -> bool {
    matches!(
        key,
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
            | "stream"
    )
}

fn object_field<'a>(body: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    if !body.get(key).is_some_and(Value::is_object) {
        body.insert(key.into(), Value::Object(Map::new()));
    }
    body.get_mut(key)
        .and_then(Value::as_object_mut)
        .expect("object field was inserted")
}

fn insert_stop(body: &mut Map<String, Value>, key: &str, stop: &[String]) {
    if !stop.is_empty() {
        body.insert(
            key.into(),
            Value::Array(stop.iter().cloned().map(Value::String).collect()),
        );
    }
}

fn insert_option<T: Serialize>(body: &mut Map<String, Value>, key: &str, value: Option<T>) {
    if let Some(value) = value {
        body.insert(
            key.into(),
            serde_json::to_value(value).expect("generation option is serializable"),
        );
    }
}

fn content_kind(target: OperationKey) -> Result<ContentGenerationKind, CoreError> {
    match target.kind() {
        OperationKind::ContentGeneration(kind) => Ok(kind),
        _ => Err(invalid_body(
            target,
            "target is not a content generation kind",
        )),
    }
}

fn invalid_body(target: OperationKey, reason: &str) -> CoreError {
    CoreError::InvalidProviderPayload {
        target,
        reason: reason.into(),
    }
}
