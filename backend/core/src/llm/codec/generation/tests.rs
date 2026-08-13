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
        limits: GenerationLimits {
            max_output_tokens: Some(32),
            max_tool_calls: None,
        },
        modalities: vec![OutputModality::Text],
        mode,
    }
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
fn decodes_claude_complete_response_into_semantic_output() {
    let target = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::ClaudeMessages,
    );
    let body = JsonBody::encode(&json!({
        "id":"msg_1","type":"message","role":"assistant","model":"claude-test",
        "content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn",
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
    assert!(matches!(response.output[0], OutputItem::Message(_)));
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
