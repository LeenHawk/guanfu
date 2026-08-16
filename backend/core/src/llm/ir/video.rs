//! 视频生成语义模型(异步任务形态)。
//!
//! canonical 走 OpenAI 视频任务;Gemini 渠道经 gproxy 转换为 Veo 长时操作。
//! 任务完成后用 [`VideoJob::content_ref`] 调 DownloadContent 取媒体字节。

use bytes::Bytes;
use serde::{Deserialize, Serialize};

use super::{MediaSource, ModelId, OperationFailure};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VideoRequest {
    Create(CreateVideoRequest),
    Retrieve(RetrieveVideoRequest),
    List(ListVideosRequest),
    Delete(DeleteVideoRequest),
    DownloadContent(DownloadVideoContentRequest),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct CreateVideoRequest {
    pub model: ModelId,
    pub prompt: String,
    /// 时长(秒)。
    pub seconds: Option<u32>,
    /// `宽x高`,canonical OpenAI 形态;Gemini 渠道映射为宽高比 + 分辨率。
    pub size: Option<String>,
    /// 参考图;Gemini 渠道要求内联数据。
    pub input_reference: Option<MediaSource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct RetrieveVideoRequest {
    /// 任务 id(OpenAI:video id;Gemini:操作资源名)。
    pub id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ListVideosRequest {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DeleteVideoRequest {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DownloadVideoContentRequest {
    /// 取自 [`VideoJob::content_ref`]。
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VideoResponse {
    Job(VideoJob),
    Jobs(VideoJobList),
    Deleted(VideoDeleted),
    Content(VideoContent),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct VideoJob {
    pub id: String,
    pub status: VideoJobStatus,
    pub progress: Option<u32>,
    pub model: Option<String>,
    pub error: Option<OperationFailure>,
    /// 完成后用于 DownloadContent 的标识
    /// (OpenAI:video id;Gemini:Veo 文件 id)。未完成或不可下载时为 None。
    pub content_ref: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum VideoJobStatus {
    Queued,
    InProgress,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct VideoJobList {
    pub jobs: Vec<VideoJob>,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct VideoDeleted {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct VideoContent {
    pub media_type: Option<String>,
    #[ts(type = "number[]")]
    pub bytes: Bytes,
}
