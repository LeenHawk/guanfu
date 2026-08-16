//! 媒体服务:执行生图 / 音频 / 视频操作,并把产物落成 Media Asset。
//!
//! 内联返回的字节直接进 AssetStore;只给 URL 的产物不伪造成 Asset
//! (没有字节就没有哈希),原样回给调用方由 UI 外链。

use std::sync::Arc;

use sea_orm::ConnectionTrait;
use serde::{Deserialize, Serialize};

use crate::assets::AssetStore;
use crate::llm::ir::audio::{
    AudioRequest, AudioResponse, SpeechRequest, Transcription, TranscriptionRequest,
};
use crate::llm::ir::images::{EditImageRequest, GenerateImageRequest, ImageRequest};
use crate::llm::ir::video::{
    CreateVideoRequest, DownloadVideoContentRequest, RetrieveVideoRequest, VideoJob, VideoRequest,
    VideoResponse,
};
use crate::llm::ir::{MediaSource, OperationRequest, OperationResponse};
use crate::services::assets::{AssetHeadDto, AssetService};
use crate::services::auth::Actor;
use crate::services::llm::{LlmService, SemanticLlmOutput};
use crate::CoreError;

/// 一次媒体操作的产物。
#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct MediaResult {
    /// 已落库的 Media Asset。
    pub assets: Vec<AssetHeadDto>,
    /// 上游只给了 URL 的产物;不落库,由 UI 外链或用户另存。
    pub urls: Vec<String>,
}

pub struct MediaService;

impl MediaService {
    #[tracing::instrument(skip_all, fields(channel_id = channel_id))]
    pub async fn generate_image(
        db: &impl ConnectionTrait,
        actor: Actor,
        llm: &LlmService,
        store: &Arc<dyn AssetStore>,
        channel_id: i32,
        name: &str,
        request: GenerateImageRequest,
    ) -> Result<MediaResult, CoreError> {
        let response = complete(
            db,
            llm,
            channel_id,
            OperationRequest::Images(ImageRequest::Generate(request)),
        )
        .await?;
        save_images(db, actor, store, name, response).await
    }

    #[tracing::instrument(skip_all, fields(channel_id = channel_id))]
    pub async fn edit_image(
        db: &impl ConnectionTrait,
        actor: Actor,
        llm: &LlmService,
        store: &Arc<dyn AssetStore>,
        channel_id: i32,
        name: &str,
        request: EditImageRequest,
    ) -> Result<MediaResult, CoreError> {
        let response = complete(
            db,
            llm,
            channel_id,
            OperationRequest::Images(ImageRequest::Edit(request)),
        )
        .await?;
        save_images(db, actor, store, name, response).await
    }

    #[tracing::instrument(skip_all, fields(channel_id = channel_id))]
    pub async fn speech(
        db: &impl ConnectionTrait,
        actor: Actor,
        llm: &LlmService,
        store: &Arc<dyn AssetStore>,
        channel_id: i32,
        name: &str,
        request: SpeechRequest,
    ) -> Result<AssetHeadDto, CoreError> {
        let response = complete(
            db,
            llm,
            channel_id,
            OperationRequest::Audio(AudioRequest::Speech(request)),
        )
        .await?;
        let OperationResponse::Audio(AudioResponse::Speech(artifact)) = response else {
            return Err(unexpected("speech"));
        };
        AssetService::create_media(
            db,
            actor,
            store.as_ref(),
            name,
            &artifact.media_type,
            None,
            &artifact.bytes,
        )
        .await
    }

    #[tracing::instrument(skip_all, fields(channel_id = channel_id))]
    pub async fn transcribe(
        db: &impl ConnectionTrait,
        llm: &LlmService,
        channel_id: i32,
        request: TranscriptionRequest,
    ) -> Result<Transcription, CoreError> {
        let response = complete(
            db,
            llm,
            channel_id,
            OperationRequest::Audio(AudioRequest::Transcribe(request)),
        )
        .await?;
        match response {
            OperationResponse::Audio(AudioResponse::Transcription(transcription)) => {
                Ok(transcription)
            }
            _ => Err(unexpected("transcription")),
        }
    }

    /// 创建视频任务;异步语义,返回排队中的任务。
    #[tracing::instrument(skip_all, fields(channel_id = channel_id))]
    pub async fn create_video(
        db: &impl ConnectionTrait,
        llm: &LlmService,
        channel_id: i32,
        request: CreateVideoRequest,
    ) -> Result<VideoJob, CoreError> {
        job(
            db,
            llm,
            channel_id,
            OperationRequest::Video(VideoRequest::Create(request)),
        )
        .await
    }

    /// 轮询视频任务状态。
    #[tracing::instrument(skip_all, fields(channel_id = channel_id))]
    pub async fn poll_video(
        db: &impl ConnectionTrait,
        llm: &LlmService,
        channel_id: i32,
        id: String,
    ) -> Result<VideoJob, CoreError> {
        job(
            db,
            llm,
            channel_id,
            OperationRequest::Video(VideoRequest::Retrieve(RetrieveVideoRequest { id })),
        )
        .await
    }

    /// 下载已完成任务的视频内容并落成 Media Asset。
    #[tracing::instrument(skip_all, fields(channel_id = channel_id))]
    pub async fn download_video(
        db: &impl ConnectionTrait,
        actor: Actor,
        llm: &LlmService,
        store: &Arc<dyn AssetStore>,
        channel_id: i32,
        name: &str,
        content_ref: String,
    ) -> Result<AssetHeadDto, CoreError> {
        let response = complete(
            db,
            llm,
            channel_id,
            OperationRequest::Video(VideoRequest::DownloadContent(DownloadVideoContentRequest {
                id: content_ref,
            })),
        )
        .await?;
        let OperationResponse::Video(VideoResponse::Content(content)) = response else {
            return Err(unexpected("video content"));
        };
        AssetService::create_media(
            db,
            actor,
            store.as_ref(),
            name,
            content.media_type.as_deref().unwrap_or("video/mp4"),
            None,
            &content.bytes,
        )
        .await
    }
}

async fn complete(
    db: &impl ConnectionTrait,
    llm: &LlmService,
    channel_id: i32,
    request: OperationRequest,
) -> Result<OperationResponse, CoreError> {
    match llm.execute(db, channel_id, request).await? {
        SemanticLlmOutput::Complete(response) => Ok(response),
        // 媒体产物需要完整字节才能落库,流式分支由专门的进度通道承载。
        _ => Err(CoreError::UnsupportedRouteImplementation {
            implementation: "streaming media persistence",
        }),
    }
}

async fn job(
    db: &impl ConnectionTrait,
    llm: &LlmService,
    channel_id: i32,
    request: OperationRequest,
) -> Result<VideoJob, CoreError> {
    match complete(db, llm, channel_id, request).await? {
        OperationResponse::Video(VideoResponse::Job(job)) => Ok(job),
        _ => Err(unexpected("video job")),
    }
}

async fn save_images(
    db: &impl ConnectionTrait,
    actor: Actor,
    store: &Arc<dyn AssetStore>,
    name: &str,
    response: OperationResponse,
) -> Result<MediaResult, CoreError> {
    let OperationResponse::Images(response) = response else {
        return Err(unexpected("image"));
    };
    let mut assets = Vec::new();
    let mut urls = Vec::new();
    for (index, artifact) in response.images.iter().enumerate() {
        match &artifact.source {
            MediaSource::Data { media_type, bytes } => {
                let label = if index == 0 {
                    name.to_owned()
                } else {
                    format!("{name} {}", index + 1)
                };
                assets.push(
                    AssetService::create_media(
                        db,
                        actor,
                        store.as_ref(),
                        &label,
                        &media_type.0,
                        None,
                        bytes,
                    )
                    .await?,
                );
            }
            MediaSource::Url { url } => urls.push(url.clone()),
            // File 引用属于上游文件体系,没有可落库的字节。
            MediaSource::File { id } => urls.push(id.0.clone()),
        }
    }
    Ok(MediaResult { assets, urls })
}

fn unexpected(what: &'static str) -> CoreError {
    CoreError::InvalidExchangePayload {
        reason: format!("route returned a response that is not {what}"),
    }
}

/// 适配层入参:渠道与产物命名 + 语义请求本体。
#[derive(Clone, Debug, Deserialize, Serialize, ts_rs::TS)]
pub struct MediaInput<T> {
    pub channel_id: i32,
    /// 产物 Asset 名;转写等无产物的操作忽略。
    #[serde(default)]
    pub name: String,
    pub request: T,
}

/// 视频轮询 / 下载的入参。
#[derive(Clone, Debug, Deserialize, Serialize, ts_rs::TS)]
pub struct VideoJobInput {
    pub channel_id: i32,
    pub id: String,
    #[serde(default)]
    pub name: String,
}
