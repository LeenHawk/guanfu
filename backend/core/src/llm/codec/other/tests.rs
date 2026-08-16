use bytes::Bytes;
use gproxy_protocol::{ContentGenerationKind, Operation, OperationKey, Provider};
use serde_json::json;

use super::{decode, encode};
use crate::llm::codec::{DecodedResponse, OperationEvent};
use crate::llm::ir::audio::{
    AudioChunking, AudioRequest, AudioUsage, DiarizationConfig, KnownSpeaker, TranscriptionMode,
    TranscriptionRequest,
};
use crate::llm::ir::images::{
    EditImageRequest, GenerateImageRequest, ImageBackground, ImageEvent, ImageMode,
    ImageModeration, ImageOptions, ImageProgress, ImageRequest,
};
use crate::llm::ir::models::{ModelRequest, ModelResponse};
use crate::llm::ir::video::{
    CreateVideoRequest, RetrieveVideoRequest, VideoJobStatus, VideoRequest, VideoResponse,
};
use crate::llm::ir::{MediaSource, MediaType, ModelId, OperationRequest, OperationResponse};
use crate::llm::wire::{
    JsonBody, JsonResponse, JsonSseData, JsonSseFrame, MultipartBody, RequestBody,
    ResponseMetadata, WireResponse,
};

fn wav() -> MediaSource {
    MediaSource::Data {
        media_type: MediaType("audio/wav".into()),
        bytes: Bytes::from_static(b"wav"),
    }
}

fn part_text(body: &MultipartBody, name: &str) -> Option<String> {
    body.parts
        .iter()
        .find(|part| part.name == name)
        .map(|part| match &part.value {
            crate::llm::wire::MultipartValue::Text(value) => value.clone(),
            _ => panic!("expected text part {name}"),
        })
}

#[test]
fn openai_image_edit_uses_replayable_multipart() {
    let request = OperationRequest::Images(ImageRequest::Edit(EditImageRequest {
        model: ModelId("gpt-image-1".into()),
        prompt: "edit".into(),
        images: vec![MediaSource::Data {
            media_type: MediaType("image/png".into()),
            bytes: Bytes::from_static(b"png"),
        }],
        mask: None,
        count: Some(1),
        input_fidelity: None,
        options: ImageOptions {
            background: Some(ImageBackground::Transparent),
            compression: Some(85),
            moderation: Some(ImageModeration::Low),
            partial_images: Some(2),
            ..Default::default()
        },
        mode: ImageMode::Stream,
    }));
    let target = OperationKey::provider(Operation::EditImage, Provider::OpenAi);
    let encoded = encode(&request, target).unwrap();
    let RequestBody::Multipart(body) = encoded.body else {
        panic!("expected multipart body")
    };
    assert!(body.parts.iter().any(|part| part.name == "image[]"));
    assert_eq!(
        part_text(&body, "background").as_deref(),
        Some("transparent")
    );
    assert_eq!(
        part_text(&body, "output_compression").as_deref(),
        Some("85")
    );
    assert_eq!(part_text(&body, "moderation").as_deref(), Some("low"));
    assert_eq!(part_text(&body, "partial_images").as_deref(), Some("2"));
    assert_eq!(part_text(&body, "stream").as_deref(), Some("true"));
}

#[test]
fn openai_transcription_encodes_diarization_and_decodes_usage() {
    let request = OperationRequest::Audio(AudioRequest::Transcribe(TranscriptionRequest {
        model: ModelId("gpt-4o-transcribe-diarize".into()),
        audio: wav(),
        language: None,
        prompt: None,
        temperature: None,
        timestamps: Vec::new(),
        diarization: Some(DiarizationConfig {
            known_speakers: vec![KnownSpeaker {
                name: "alice".into(),
                reference: wav(),
            }],
            chunking: Some(AudioChunking::Auto),
        }),
        mode: TranscriptionMode::Stream,
    }));
    let target = OperationKey::provider(Operation::CreateTranscription, Provider::OpenAi);
    let encoded = encode(&request, target).unwrap();
    let RequestBody::Multipart(body) = encoded.body else {
        panic!("expected multipart body")
    };
    assert_eq!(
        part_text(&body, "known_speaker_names[]").as_deref(),
        Some("alice")
    );
    assert!(part_text(&body, "known_speaker_references[]")
        .is_some_and(|value| value.starts_with("data:audio/wav;base64,")));
    assert_eq!(
        part_text(&body, "chunking_strategy").as_deref(),
        Some("auto")
    );
    assert_eq!(
        part_text(&body, "response_format").as_deref(),
        Some("diarized_json")
    );
    assert_eq!(part_text(&body, "stream").as_deref(), Some("true"));

    let decoded = super::response::decode_transcription(
        &json!({"text":"hi","usage":{"type":"duration","seconds":1.5}}),
        target,
    )
    .unwrap();
    assert_eq!(decoded.usage, Some(AudioUsage::Duration { seconds: 1.5 }));
    let decoded = super::response::decode_transcription(
        &json!({"text":"hi","usage":{"type":"tokens","input_tokens":3,"output_tokens":4,"total_tokens":7}}),
        target,
    )
    .unwrap();
    assert!(matches!(decoded.usage, Some(AudioUsage::Tokens(usage)) if usage.total_tokens == 7));
}

#[test]
fn model_decode_reads_token_limit_extensions() {
    let target = OperationKey::provider(Operation::ListModels, Provider::OpenAi);
    let request = OperationRequest::Models(ModelRequest::List(Default::default()));
    let body = JsonBody::encode(
        &json!({"data":[{"id":"claude-x","max_input_tokens":200000,"max_output_tokens":8192}]}),
    )
    .unwrap();
    let decoded = super::response::decode_json(&request, target, &body).unwrap();
    let OperationResponse::Models(ModelResponse::List(page)) = decoded else {
        panic!("expected model list")
    };
    assert_eq!(page.models[0].context_limit, Some(200_000));
    assert_eq!(page.models[0].output_limit, Some(8192));
}

#[tokio::test]
async fn image_stream_partial_image_yields_progress_and_preview() {
    use futures_util::StreamExt;

    let request = ImageRequest::Generate(GenerateImageRequest {
        model: ModelId("gpt-image-1".into()),
        prompt: "draw".into(),
        count: None,
        options: ImageOptions {
            partial_images: Some(2),
            ..Default::default()
        },
        mode: ImageMode::Stream,
    });
    let frame = JsonSseFrame {
        event: None,
        id: None,
        data: JsonSseData::Json(
            JsonBody::encode(&json!({
                "type": "image_generation.partial_image",
                "b64_json": "aW1n",
                "partial_image_index": 0,
            }))
            .unwrap(),
        ),
    };
    let target = OperationKey::provider(Operation::CreateImage, Provider::OpenAi);
    let stream: crate::llm::wire::JsonSseStream = Box::pin(futures_util::stream::iter([Ok(frame)]));
    let DecodedResponse::Stream(events) =
        super::stream::decode_image_stream(&request, target, stream).unwrap()
    else {
        panic!("expected stream")
    };
    let events: Vec<_> = events.collect().await;
    assert!(matches!(
        events[0].as_ref().unwrap(),
        OperationEvent::Image(ImageEvent::Progress(ImageProgress {
            completed: 1,
            total: Some(2),
        }))
    ));
    assert!(matches!(
        events[1].as_ref().unwrap(),
        OperationEvent::Image(ImageEvent::Preview(_))
    ));
}

/// 生图与视频经路由矩阵到 Gemini 渠道:端点合成 + gproxy 转换全链路。
#[test]
fn routes_images_and_videos_to_gemini() {
    let image = OperationRequest::Images(ImageRequest::Generate(GenerateImageRequest {
        model: ModelId("gemini-2.5-flash-image".into()),
        prompt: "远山".into(),
        count: None,
        options: ImageOptions::default(),
        mode: ImageMode::Complete,
    }));
    let target = OperationKey::content_generation(
        Operation::GenerateContent,
        ContentGenerationKind::GeminiGenerateContent,
    );
    let wire = encode(&image, target).unwrap();
    assert!(wire.path.ends_with(":generateContent"), "{}", wire.path);
    let RequestBody::Json(body) = wire.body else {
        panic!("expected JSON body")
    };
    let value: serde_json::Value = body.decode().unwrap();
    assert!(value.get("contents").is_some());

    let native = OperationRequest::Images(ImageRequest::Generate(GenerateImageRequest {
        model: ModelId("imagen-4.0-generate-001".into()),
        prompt: "远山".into(),
        count: Some(2),
        options: ImageOptions::default(),
        mode: ImageMode::Complete,
    }));
    let target = OperationKey::provider(Operation::CreateImage, Provider::Gemini);
    let wire = encode(&native, target).unwrap();
    assert!(wire.path.ends_with(":predict"), "{}", wire.path);
    let RequestBody::Json(body) = wire.body else {
        panic!("expected JSON body")
    };
    let value: serde_json::Value = body.decode().unwrap();
    assert_eq!(value["instances"][0]["prompt"], "远山");
    assert_eq!(value["parameters"]["sampleCount"], 2);

    let create = OperationRequest::Video(VideoRequest::Create(CreateVideoRequest {
        model: ModelId("veo-3.1-generate-preview".into()),
        prompt: "云海延时".into(),
        seconds: Some(8),
        size: Some("720x1280".into()),
        input_reference: None,
    }));
    let target = OperationKey::provider(Operation::CreateVideo, Provider::Gemini);
    let wire = encode(&create, target).unwrap();
    assert!(wire.path.ends_with(":predictLongRunning"), "{}", wire.path);
    let RequestBody::Json(body) = wire.body else {
        panic!("expected JSON body")
    };
    let value: serde_json::Value = body.decode().unwrap();
    assert_eq!(value["instances"][0]["prompt"], "云海延时");
    assert_eq!(value["parameters"]["aspectRatio"], "9:16");

    let retrieve = OperationRequest::Video(VideoRequest::Retrieve(RetrieveVideoRequest {
        id: "models/veo-3.1/operations/op1".into(),
    }));
    let target = OperationKey::provider(Operation::RetrieveVideo, Provider::Gemini);
    let decoded = decode(
        &retrieve,
        target,
        WireResponse::Json(JsonResponse {
            metadata: ResponseMetadata {
                status: http::StatusCode::OK,
                headers: http::HeaderMap::new(),
            },
            body: JsonBody::encode(&json!({
                "name": "models/veo-3.1/operations/op1",
                "done": true,
                "response": {"generateVideoResponse": {"generatedSamples": [
                    {"video": {"uri": "https://generativelanguage.googleapis.com/v1beta/files/f1:download?alt=media"}}
                ]}}
            }))
            .unwrap(),
        }),
    )
    .unwrap();
    let DecodedResponse::Complete(OperationResponse::Video(VideoResponse::Job(job))) = decoded
    else {
        panic!("expected video job")
    };
    assert!(matches!(job.status, VideoJobStatus::Completed));
    assert_eq!(job.content_ref.as_deref(), Some("f1"));
}
