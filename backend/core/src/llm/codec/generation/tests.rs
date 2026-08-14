use bytes::Bytes;
use futures_util::{stream, StreamExt};
use gproxy_protocol::{ContentGenerationKind, Operation, OperationKey};
use http::{HeaderMap, StatusCode};
use serde_json::{json, Value};

use super::{decode, encode};
use crate::llm::codec::{DecodedResponse, OperationEvent};
use crate::llm::ir::generation::*;
use crate::llm::ir::OperationResponse;
use crate::llm::wire::{
    parse_json_sse, JsonBody, JsonResponse, JsonSseResponse, RequestBody, ResponseMetadata,
    WireResponse,
};

fn request(mode: GenerateMode) -> GenerateRequest {
    GenerateRequest {
        model: crate::llm::ir::ModelId("claude-test".into()),
        input: vec![InputItem::Message {
            message: Message {
                role: MessageRole::User,
                content: vec![InputContent::Text { text: "hi".into() }],
            },
        }],
        instructions: Vec::new(),
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        output: OutputConstraint::Text,
        sampling: SamplingOptions::default(),
        reasoning: None,
        protocol_options: Vec::new(),
        limits: GenerationLimits {
            max_output_tokens: Some(32),
            max_tool_calls: None,
        },
        modalities: vec![OutputModality::Text],
        mode,
    }
}

fn json_patch(value: Value) -> JsonMergePatch {
    JsonMergePatch(serde_json::from_value(value).unwrap())
}

fn metadata() -> ResponseMetadata {
    ResponseMetadata {
        status: StatusCode::OK,
        headers: HeaderMap::new(),
    }
}

#[test]
fn encodes_semantic_request_as_claude_messages() {
    let target = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::ClaudeMessages,
    );
    let encoded = encode(&request(GenerateMode::Complete), target).unwrap();
    let RequestBody::Json(body) = encoded.body else {
        panic!("expected JSON body")
    };
    let value: Value = body.decode().unwrap();
    assert_eq!(value["model"], "claude-test");
    assert_eq!(value["messages"][0]["content"][0]["text"], "hi");
}

#[test]
fn encodes_sampling_and_historical_system_for_openai_chat() {
    let mut request = request(GenerateMode::Complete);
    request.input.push(InputItem::Message {
        message: Message {
            role: MessageRole::System,
            content: vec![InputContent::Text {
                text: "world info".into(),
            }],
        },
    });
    request.sampling = SamplingOptions {
        temperature: Some(0.7),
        top_p: Some(0.9),
        seed: Some(42),
        stop: vec!["STOP".into()],
        frequency_penalty: Some(0.2),
        presence_penalty: Some(0.3),
        ..Default::default()
    };
    let target = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::OpenAiChatCompletions,
    );
    let encoded = encode(&request, target).unwrap();
    let RequestBody::Json(body) = encoded.body else {
        panic!("expected JSON body")
    };
    let value: Value = body.decode().unwrap();
    assert_eq!(value["messages"][1]["role"], "system");
    assert_eq!(value["seed"], 42);
    assert_eq!(value["stop"], json!(["STOP"]));
    assert!((value["frequency_penalty"].as_f64().unwrap() - 0.2).abs() < 1e-6);
    assert!((value["presence_penalty"].as_f64().unwrap() - 0.3).abs() < 1e-6);
}

#[test]
fn applies_only_matching_protocol_overlay_after_sampling() {
    let mut request = request(GenerateMode::Complete);
    request.sampling.top_k = Some(12);
    request.protocol_options = vec![
        ProtocolOptions {
            kind: GenerationProtocol::OpenAiChatCompletions,
            patch: json_patch(json!({"service_tier":"priority"})),
        },
        ProtocolOptions {
            kind: GenerationProtocol::GeminiGenerateContent,
            patch: json_patch(json!({
                "model":"overridden-model",
                "generationConfig":{
                    "topK":99,
                    "maxOutputTokens":999,
                    "responseMimeType":"application/json",
                    "thinkingConfig":{"thinkingBudget":999}
                },
                "safetySettings":[{"category":"HARM_CATEGORY_HATE_SPEECH","threshold":"BLOCK_NONE"}]
            })),
        },
    ];
    let target = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::GeminiGenerateContent,
    );
    let encoded = encode(&request, target).unwrap();
    let RequestBody::Json(body) = encoded.body else {
        panic!("expected JSON body")
    };
    let value: Value = body.decode().unwrap();
    assert_eq!(value["model"], "claude-test");
    assert_eq!(value["generationConfig"]["topK"], 99);
    assert_eq!(value["generationConfig"]["maxOutputTokens"], 32);
    assert_eq!(value["generationConfig"]["responseMimeType"], "text/plain");
    assert!(value["generationConfig"].get("thinkingConfig").is_none());
    assert!(value.get("service_tier").is_none());
    assert_eq!(value["safetySettings"][0]["threshold"], "BLOCK_NONE");
}

#[test]
fn maps_reasoning_budget_and_summary_to_claude() {
    let mut request = request(GenerateMode::Complete);
    request.reasoning = Some(ReasoningOptions {
        effort: Some(ReasoningEffort::High),
        budget_tokens: Some(2048),
        summary: Some(ReasoningSummary::Auto),
    });
    let target = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::ClaudeMessages,
    );
    let encoded = encode(&request, target).unwrap();
    let RequestBody::Json(body) = encoded.body else {
        panic!("expected JSON body")
    };
    let value: Value = body.decode().unwrap();
    assert_eq!(value["thinking"]["type"], "enabled");
    assert_eq!(value["thinking"]["budget_tokens"], 2048);
    assert_eq!(value["thinking"]["display"], "summarized");
    assert_eq!(value["output_config"]["effort"], "high");

    request.reasoning.as_mut().unwrap().summary = Some(ReasoningSummary::Detailed);
    assert!(matches!(
        encode(&request, target),
        Err(crate::CoreError::IncompatibleRoute { fields, .. })
            if fields == ["reasoning.summary"]
    ));
}

#[test]
fn requests_and_replays_claude_reasoning_continuation() {
    let mut request = request(GenerateMode::Complete);
    request.input.insert(
        0,
        InputItem::Reasoning {
            reasoning: ReasoningInput {
                previous: ReasoningOutput {
                    id: crate::llm::ir::OutputId("reasoning_1".into()),
                    parts: vec![ReasoningPart::Text {
                        text: "hidden".into(),
                        continuation: Some(ReasoningContinuation::ClaudeSignature {
                            signature: "signature".into(),
                        }),
                    }],
                },
            },
        },
    );
    request.reasoning = Some(ReasoningOptions {
        effort: Some(ReasoningEffort::High),
        budget_tokens: Some(2048),
        summary: Some(ReasoningSummary::Auto),
    });
    let claude = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::ClaudeMessages,
    );
    let encoded = encode(&request, claude).unwrap();
    let RequestBody::Json(body) = encoded.body else {
        panic!("expected JSON body")
    };
    let value: Value = body.decode().unwrap();
    assert_eq!(value["messages"][0]["content"][0]["thinking"], "hidden");
    assert_eq!(value["messages"][0]["content"][0]["signature"], "signature");

    let gemini = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::GeminiGenerateContent,
    );
    assert!(matches!(
        encode(&request, gemini),
        Err(crate::CoreError::IncompatibleRoute { fields, .. })
            if fields == ["input.reasoning.continuation"]
    ));
}

#[test]
fn requests_openai_encrypted_reasoning_continuation() {
    let mut request = request(GenerateMode::Complete);
    request.reasoning = Some(ReasoningOptions {
        effort: Some(ReasoningEffort::High),
        budget_tokens: None,
        summary: Some(ReasoningSummary::Auto),
    });
    let target = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::OpenAiResponses,
    );
    let encoded = encode(&request, target).unwrap();
    let RequestBody::Json(body) = encoded.body else {
        panic!("expected JSON body")
    };
    let value: Value = body.decode().unwrap();
    assert_eq!(value["include"], json!(["reasoning.encrypted_content"]));
}

#[test]
fn decodes_claude_complete_response_into_semantic_output() {
    let target = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::ClaudeMessages,
    );
    let body = JsonBody::encode(&json!({
        "id":"msg_1","type":"message","role":"assistant","model":"claude-test",
        "content":[
            {"type":"thinking","thinking":"hidden","signature":"signature"},
            {"type":"text","text":"hello"}
        ],"stop_reason":"end_turn",
        "usage":{"input_tokens":2,"output_tokens":1}
    }))
    .unwrap();
    let decoded = decode(
        &request(GenerateMode::Complete),
        target,
        WireResponse::Json(JsonResponse {
            metadata: metadata(),
            body,
        }),
    )
    .unwrap();
    let DecodedResponse::Complete(OperationResponse::Generate(response)) = decoded else {
        panic!("expected complete generation response")
    };
    assert!(matches!(
        &response.output[0],
        OutputItem::Reasoning(ReasoningOutput { parts, .. })
            if matches!(
                &parts[0],
                ReasoningPart::Text {
                    text,
                    continuation: Some(ReasoningContinuation::ClaudeSignature { signature })
                } if text == "hidden" && signature == "signature"
            )
    ));
    assert!(response
        .output
        .iter()
        .any(|item| matches!(item, OutputItem::Message(_))));
}

#[test]
fn decodes_tool_call_and_failed_complete_lifecycle() {
    let claude = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::ClaudeMessages,
    );
    let tool_body = JsonBody::encode(&json!({
        "id":"msg_1","type":"message","role":"assistant","model":"claude-test",
        "content":[{"type":"tool_use","id":"toolu_1","name":"lookup","input":{}}],
        "stop_reason":"tool_use","usage":{"input_tokens":2,"output_tokens":1}
    }))
    .unwrap();
    let decoded = decode(
        &request(GenerateMode::Complete),
        claude,
        WireResponse::Json(JsonResponse {
            metadata: metadata(),
            body: tool_body,
        }),
    )
    .unwrap();
    let DecodedResponse::Complete(OperationResponse::Generate(response)) = decoded else {
        panic!("expected complete generation response")
    };
    assert_eq!(response.finish, FinishReason::ToolCalls);

    let responses = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::OpenAiResponses,
    );
    let failed_body = JsonBody::encode(&json!({
        "id":"resp_1","model":"test","status":"failed",
        "error":{"code":"server_error","message":"failed"}
    }))
    .unwrap();
    assert!(matches!(
        decode(
            &request(GenerateMode::Complete),
            responses,
            WireResponse::Json(JsonResponse {
                metadata: metadata(),
                body: failed_body,
            }),
        ),
        Err(crate::CoreError::OperationFailed(failure))
            if failure.code == "server_error" && failure.retryable
    ));
}

#[tokio::test]
async fn decodes_claude_sse_into_semantic_text_delta() {
    let target = OperationKey::content_generation(
        Operation::StreamGenerateContent,
        ContentGenerationKind::ClaudeMessages,
    );
    let bytes = Bytes::from_static(
        b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-test\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );
    let decoded = decode(
        &request(GenerateMode::Stream),
        target,
        WireResponse::JsonSse(JsonSseResponse {
            metadata: metadata(),
            stream: parse_json_sse(Box::pin(stream::iter([Ok(bytes)]))),
        }),
    )
    .unwrap();
    let DecodedResponse::Stream(stream) = decoded else {
        panic!("expected stream")
    };
    let events = stream.collect::<Vec<_>>().await;
    assert!(events.iter().any(|event| matches!(
        event,
        Ok(OperationEvent::Generate(GenerateEvent::Delta(GenerateDelta::Text(
            ContentTextDelta { delta, .. }
        )))) if delta == "hello"
    )));
}

#[tokio::test]
async fn decodes_claude_stream_reasoning_continuation() {
    let target = OperationKey::content_generation(
        Operation::StreamGenerateContent,
        ContentGenerationKind::ClaudeMessages,
    );
    let bytes = Bytes::from_static(
        b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-test\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hidden\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"signature\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );
    let decoded = decode(
        &request(GenerateMode::Stream),
        target,
        WireResponse::JsonSse(JsonSseResponse {
            metadata: metadata(),
            stream: parse_json_sse(Box::pin(stream::iter([Ok(bytes)]))),
        }),
    )
    .unwrap();
    let DecodedResponse::Stream(stream) = decoded else {
        panic!("expected stream")
    };
    let events = stream.collect::<Vec<_>>().await;
    assert!(events.iter().any(|event| matches!(
        event,
        Ok(OperationEvent::Generate(GenerateEvent::OutputFinished(OutputFinished {
            item: OutputItem::Reasoning(ReasoningOutput { parts, .. }),
            ..
        }))) if matches!(
            &parts[0],
            ReasoningPart::Text {
                text,
                continuation: Some(ReasoningContinuation::ClaudeSignature { signature })
            } if text == "hidden" && signature == "signature"
        )
    )));
}

#[tokio::test]
async fn preserves_stream_tool_call_finish_reason() {
    let target = OperationKey::content_generation(
        Operation::StreamGenerateContent,
        ContentGenerationKind::OpenAiResponses,
    );
    let bytes = Bytes::from_static(
        b"data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"lookup\",\"arguments\":\"\",\"status\":\"in_progress\"}}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"model\":\"test\",\"status\":\"completed\",\"output\":[]}}\n\n",
    );
    let decoded = decode(
        &request(GenerateMode::Stream),
        target,
        WireResponse::JsonSse(JsonSseResponse {
            metadata: metadata(),
            stream: parse_json_sse(Box::pin(stream::iter([Ok(bytes)]))),
        }),
    )
    .unwrap();
    let DecodedResponse::Stream(stream) = decoded else {
        panic!("expected stream")
    };
    let events = stream.collect::<Vec<_>>().await;
    assert!(events.iter().any(|event| matches!(
        event,
        Ok(OperationEvent::Generate(GenerateEvent::Finished(
            GenerationFinished {
                finish: FinishReason::ToolCalls,
                ..
            }
        )))
    )));
}
