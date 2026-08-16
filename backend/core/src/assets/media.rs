//! Media definition:只描述 AssetStore 中的对象,不含二进制。
//!
//! 字节是 `location = store` 的 chunk,与 db 内文本 chunk 共用同一
//! sha256 命名空间(计划 §2.2)。

use serde::{Deserialize, Serialize};

use super::refs::Extra;
use super::{
    join_inline, split_inline, AssetDefinition, ChunkContents, ChunkHash, Manifest, SplitManifest,
};
use crate::entities::asset::AssetKind;
use crate::CoreError;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "version")]
pub enum MediaDefinition {
    V1(MediaV1),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct MediaV1 {
    /// 内容哈希,同时是 AssetStore 的 storage key。
    pub hash: ChunkHash,
    pub mime_type: String,
    pub size: u64,
    /// 原始文件名(导入保真用)。
    pub filename: Option<String>,
    #[serde(default)]
    pub extra: Extra,
}

impl AssetDefinition for MediaDefinition {
    const KIND: AssetKind = AssetKind::Media;

    fn split(&self) -> Result<SplitManifest, CoreError> {
        split_inline(self)
    }

    fn join(manifest: &Manifest, _: &ChunkContents) -> Result<Self, CoreError> {
        join_inline(manifest)
    }
}
