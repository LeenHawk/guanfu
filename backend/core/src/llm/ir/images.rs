use serde::{Deserialize, Serialize};

use super::{MediaSource, ModelId, OperationFailure, OutputId, Usage};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageRequest {
    Generate(GenerateImageRequest),
    Edit(EditImageRequest),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMode {
    Complete,
    Stream,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerateImageRequest {
    pub model: ModelId,
    pub prompt: String,
    pub count: Option<u32>,
    pub options: ImageOptions,
    pub mode: ImageMode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditImageRequest {
    pub model: ModelId,
    pub prompt: String,
    pub images: Vec<MediaSource>,
    pub mask: Option<MediaSource>,
    pub count: Option<u32>,
    pub input_fidelity: Option<ImageInputFidelity>,
    pub options: ImageOptions,
    pub mode: ImageMode,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageOptions {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub quality: Option<ImageQuality>,
    pub background: Option<ImageBackground>,
    pub output_format: Option<ImageFormat>,
    pub compression: Option<u8>,
    pub moderation: Option<ImageModeration>,
    pub partial_images: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageQuality {
    Low,
    Medium,
    High,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageBackground {
    Transparent,
    Opaque,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageModeration {
    Low,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageInputFidelity {
    Low,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageResponse {
    pub images: Vec<ImageArtifact>,
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageArtifact {
    pub id: OutputId,
    pub source: MediaSource,
    pub revised_prompt: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageEvent {
    Started,
    Preview(ImagePreview),
    Progress(ImageProgress),
    Finished(ImageResponse),
    Failed(OperationFailure),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImagePreview {
    pub index: u32,
    pub sequence: u32,
    pub image: ImageArtifact,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageProgress {
    pub completed: u32,
    pub total: Option<u32>,
}
