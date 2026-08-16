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
    request.tools = vec![
        ToolDefinition::WebSearch(WebSearchTool {
            max_uses: Some(3),
            blocked_domains: vec!["b.example".into()],
            ..Default::default()
        }),
        ToolDefinition::WebFetch(WebFetchTool {
            max_uses: Some(5),
            ..Default::default()
        }),
        ToolDefinition::TextEditor(TextEditorTool {
            max_characters: Some(10_000),
        }),
        ToolDefinition::Memory,
    ];
    let claude = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::ClaudeMessages,
    );
    let encoded = encode(&request, claude).unwrap();
    let RequestBody::Json(body) = encoded.body else {
        panic!("expected JSON body")
    };
    let value: Value = body.decode().unwrap();
    assert_eq!(value["model"], "claude-test");
    assert_eq!(value["messages"][0]["content"][0]["thinking"], "hidden");
    assert_eq!(value["messages"][0]["content"][0]["signature"], "signature");
    assert_eq!(value["messages"][1]["content"][0]["text"], "hi");
    let tools = value["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool["name"] == "web_search"
        && tool["max_uses"] == 3
        && tool["blocked_domains"] == json!(["b.example"])));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "web_fetch" && tool["max_uses"] == 5));
    assert!(tools
        .iter()
        .any(|tool| tool["type"] == "text_editor_20250728" && tool["max_characters"] == 10_000));
    assert!(tools.iter().any(|tool| tool["type"] == "memory_20250818"));

    let openai = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::OpenAiResponses,
    );
    let mut portable = request.clone();
    portable.input.remove(0);
    assert!(matches!(
        encode(&portable, openai),
        Err(crate::CoreError::IncompatibleRoute { fields, .. })
            if fields.contains(&"tools[].web_fetch".to_owned())
                && fields.contains(&"tools[].web_search.max_uses".to_owned())
                && fields.contains(&"tools[].text_editor.max_characters".to_owned())
                && fields.contains(&"tools[].memory".to_owned())
    ));

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
fn decodes_gemini_complete_response_into_semantic_output() {
    let target = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::GeminiGenerateContent,
    );
    let body = JsonBody::encode(&json!({
        "responseId":"gen_1","modelVersion":"gemini-test",
        "candidates":[{
            "index":0,
            "content":{"role":"model","parts":[
                {"text":"hidden","thought":true,"thoughtSignature":"signature"},
                {"text":"hello"}
            ]},
            "finishReason":"STOP"
        }],
        "usageMetadata":{
            "promptTokenCount":4,"candidatesTokenCount":2,"thoughtsTokenCount":3,
            "cachedContentTokenCount":1,"totalTokenCount":9
        }
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
                    continuation: Some(ReasoningContinuation::GeminiThoughtSignature { signature })
                } if text == "hidden" && signature == "signature"
            )
    ));
    assert!(matches!(
        &response.output[1],
        OutputItem::Message(message)
            if matches!(&message.content[0], OutputContent::Text { text, .. } if text == "hello")
    ));
    assert_eq!(response.finish, FinishReason::Stop);
    let usage = response.usage.unwrap();
    assert_eq!(usage.input_tokens, 4);
    assert_eq!(usage.output_tokens, 5);
    assert_eq!(usage.reasoning_tokens, 3);
    assert_eq!(usage.cached_input_tokens, 1);
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

    let hosted_body = JsonBody::encode(&json!({
        "id":"resp_2","model":"test","status":"completed",
        "output":[
            {"type":"web_search_call","id":"ws_1","status":"completed","action":{"type":"search","query":"q"}},
            {"type":"mcp_approval_request","id":"mcpr_1","server_label":"docs","name":"lookup","arguments":"{\"q\":1}"}
        ]
    }))
    .unwrap();
    let decoded = decode(
        &request(GenerateMode::Complete),
        responses,
        WireResponse::Json(JsonResponse {
            metadata: metadata(),
            body: hosted_body,
        }),
    )
    .unwrap();
    let DecodedResponse::Complete(OperationResponse::Generate(response)) = decoded else {
        panic!("expected complete generation response")
    };
    assert_eq!(response.finish, FinishReason::ToolCalls);
    assert!(response.output.iter().any(|item| matches!(
        item,
        OutputItem::ToolExecution(ToolExecution {
            state: ToolExecutionState::Completed,
            output: Some(_),
            ..
        })
    )));
    assert!(response.output.iter().any(|item| matches!(
        item,
        OutputItem::McpApprovalRequest(request)
            if request.server_label == "docs" && request.name == "lookup"
    )));
}

#[test]
fn decodes_chat_complete_tool_call_response() {
    let target = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::OpenAiChatCompletions,
    );
    let body = JsonBody::encode(&json!({
        "id":"chatcmpl_1","object":"chat.completion","created":1,"model":"gpt-test",
        "choices":[{
            "index":0,"finish_reason":"tool_calls",
            "message":{
                "role":"assistant","content":"hello",
                "tool_calls":[{"type":"function","id":"call_1",
                    "function":{"name":"lookup","arguments":"{\"q\":1}"}}]
            }
        }],
        "usage":{
            "prompt_tokens":5,"completion_tokens":3,"total_tokens":8,
            "prompt_tokens_details":{"cached_tokens":2},
            "completion_tokens_details":{"reasoning_tokens":1}
        }
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
    assert_eq!(response.finish, FinishReason::ToolCalls);
    assert!(matches!(
        &response.output[0],
        OutputItem::Message(message)
            if matches!(&message.content[0], OutputContent::Text { text, .. } if text == "hello")
    ));
    assert!(matches!(
        &response.output[1],
        OutputItem::ToolCall(ToolCall::Function(call))
            if call.call_id.0 == "call_1" && call.name == "lookup"
                && call.arguments == json!({"q":1})
    ));
    let usage = response.usage.unwrap();
    assert_eq!(usage.input_tokens, 5);
    assert_eq!(usage.output_tokens, 3);
    assert_eq!(usage.cached_input_tokens, 2);
    assert_eq!(usage.reasoning_tokens, 1);
    assert_eq!(usage.total_tokens, 8);
}

async fn collect_sse_events(
    target: OperationKey,
    bytes: &'static [u8],
) -> Vec<Result<OperationEvent, crate::CoreError>> {
    let decoded = decode(
        &request(GenerateMode::Stream),
        target,
        WireResponse::JsonSse(JsonSseResponse {
            metadata: metadata(),
            stream: parse_json_sse(Box::pin(stream::iter([Ok(Bytes::from_static(bytes))]))),
        }),
    )
    .unwrap();
    let DecodedResponse::Stream(stream) = decoded else {
        panic!("expected stream")
    };
    stream.collect::<Vec<_>>().await
}

#[tokio::test]
async fn decodes_claude_sse_text_delta_and_reasoning_continuation() {
    let target = OperationKey::content_generation(
        Operation::StreamGenerateContent,
        ContentGenerationKind::ClaudeMessages,
    );
    let events = collect_sse_events(
        target,
        b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-test\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hidden\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"signature\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    )
    .await;
    assert!(events.iter().any(|event| matches!(
        event,
        Ok(OperationEvent::Generate(GenerateEvent::Delta(GenerateDelta::Text(
            ContentTextDelta { delta, .. }
        )))) if delta == "hello"
    )));
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
async fn decodes_gemini_sse_thought_and_text_chunks() {
    let target = OperationKey::content_generation(
        Operation::StreamGenerateContent,
        ContentGenerationKind::GeminiGenerateContent,
    );
    let events = collect_sse_events(
        target,
        b"data: {\"responseId\":\"gen_1\",\"modelVersion\":\"gemini-test\",\"candidates\":[{\"index\":0,\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"hidden\",\"thought\":true,\"thoughtSignature\":\"signature\"}]}}]}\n\ndata: {\"responseId\":\"gen_1\",\"candidates\":[{\"index\":0,\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"hello\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":2,\"thoughtsTokenCount\":3,\"totalTokenCount\":9}}\n\n",
    )
    .await;
    assert!(events.iter().any(|event| matches!(
        event,
        Ok(OperationEvent::Generate(GenerateEvent::Delta(GenerateDelta::Text(
            ContentTextDelta { delta, .. }
        )))) if delta == "hello"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Ok(OperationEvent::Generate(GenerateEvent::OutputFinished(OutputFinished {
            item: OutputItem::Reasoning(ReasoningOutput { parts, .. }),
            ..
        }))) if matches!(
            &parts[0],
            ReasoningPart::Text {
                text,
                continuation: Some(ReasoningContinuation::GeminiThoughtSignature { signature })
            } if text == "hidden" && signature == "signature"
        )
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Ok(OperationEvent::Generate(GenerateEvent::Finished(GenerationFinished {
            finish: FinishReason::Stop,
            usage: Some(usage),
        }))) if usage.output_tokens == 5
    )));
}

#[tokio::test]
async fn decodes_chat_sse_text_and_tool_argument_chunks() {
    let target = OperationKey::content_generation(
        Operation::StreamGenerateContent,
        ContentGenerationKind::OpenAiChatCompletions,
    );
    let events = collect_sse_events(
        target,
        b"data: {\"id\":\"chatcmpl_1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"gpt-test\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hello\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"chatcmpl_1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"gpt-test\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{\\\"q\\\":1}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8}}\n\ndata: [DONE]\n\n",
    )
    .await;
    assert!(events.iter().any(|event| matches!(
        event,
        Ok(OperationEvent::Generate(GenerateEvent::Delta(GenerateDelta::Text(
            ContentTextDelta { delta, .. }
        )))) if delta == "hello"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Ok(OperationEvent::Generate(GenerateEvent::Delta(GenerateDelta::FunctionArguments(
            JsonFragmentDelta { delta, .. }
        )))) if delta == "{\"q\":1}"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Ok(OperationEvent::Generate(GenerateEvent::OutputFinished(OutputFinished {
            item: OutputItem::ToolCall(ToolCall::Function(call)),
            ..
        }))) if call.call_id.0 == "call_1" && call.name == "lookup"
            && call.arguments == json!({"q":1})
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        Ok(OperationEvent::Generate(GenerateEvent::Finished(GenerationFinished {
            finish: FinishReason::ToolCalls,
            usage: Some(usage),
        }))) if usage.input_tokens == 5 && usage.output_tokens == 3
    )));
}

#[tokio::test]
async fn preserves_stream_tool_call_finish_reason() {
    let target = OperationKey::content_generation(
        Operation::StreamGenerateContent,
        ContentGenerationKind::OpenAiResponses,
    );
    let events = collect_sse_events(
        target,
        b"data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"lookup\",\"arguments\":\"\",\"status\":\"in_progress\"}}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"model\":\"test\",\"status\":\"completed\",\"output\":[]}}\n\n",
    )
    .await;
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

/// Responses 的 input 里,助手消息的文本必须是 `output_text`;
/// 发 `input_text` 会被上游 400 拒掉。多轮对话每一轮都会走到这条,
/// 单轮请求永远碰不到,所以它值得单独钉住。
#[test]
fn replayed_assistant_turns_use_output_text() {
    let mut request = request(GenerateMode::Complete);
    request.input.push(InputItem::Message {
        message: Message {
            role: MessageRole::Assistant,
            content: vec![InputContent::Text {
                text: "earlier reply".into(),
            }],
        },
    });
    request.input.push(InputItem::Message {
        message: Message {
            role: MessageRole::User,
            content: vec![InputContent::Text {
                text: "follow up".into(),
            }],
        },
    });

    let target = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::OpenAiResponses,
    );
    let RequestBody::Json(body) = encode(&request, target).unwrap().body else {
        panic!("responses encodes a json body");
    };
    let value: Value = body.decode().unwrap();
    let input = value["input"].as_array().unwrap();
    let kinds: Vec<(&str, &str)> = input
        .iter()
        .map(|item| {
            (
                item["role"].as_str().unwrap(),
                item["content"][0]["type"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("user", "input_text"),
            ("assistant", "output_text"),
            ("user", "input_text"),
        ]
    );

    // 助手消息带非文本内容:Responses 的助手侧只有 output_text / refusal,
    // IR 却容得下这种组合。显式报不兼容让路由回退,而不是发出去等 400。
    let mut illegal = super::tests::request(GenerateMode::Complete);
    illegal.input.push(InputItem::Message {
        message: Message {
            role: MessageRole::Assistant,
            content: vec![InputContent::Image {
                source: crate::llm::ir::MediaSource::Url {
                    url: "https://example.invalid/a.png".into(),
                },
                detail: ImageDetail::Auto,
            }],
        },
    });
    assert!(matches!(
        encode(&illegal, target),
        Err(crate::CoreError::IncompatibleRoute { .. })
    ));
}
