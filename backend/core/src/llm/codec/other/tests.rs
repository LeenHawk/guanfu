use bytes::Bytes;
use gproxy_protocol::{Operation, OperationKey, Provider};

use super::encode;
use crate::llm::ir::images::{EditImageRequest, ImageMode, ImageOptions, ImageRequest};
use crate::llm::ir::{MediaSource, ModelId, OperationRequest};
use crate::llm::wire::RequestBody;

#[test]
fn openai_image_edit_uses_replayable_multipart() {
    let request = OperationRequest::Images(ImageRequest::Edit(EditImageRequest {
        model: ModelId("gpt-image-1".into()),
        prompt: "edit".into(),
        images: vec![MediaSource::Data {
            media_type: crate::llm::ir::MediaType("image/png".into()),
            bytes: Bytes::from_static(b"png"),
        }],
        mask: None,
        count: Some(1),
        input_fidelity: None,
        options: ImageOptions::default(),
        mode: ImageMode::Complete,
    }));
    let target = OperationKey::provider(Operation::EditImage, Provider::OpenAi);
    let encoded = encode(&request, target).unwrap();
    let RequestBody::Multipart(body) = encoded.body else {
        panic!("expected multipart body")
    };
    assert!(body.parts.iter().any(|part| part.name == "image[]"));
}
