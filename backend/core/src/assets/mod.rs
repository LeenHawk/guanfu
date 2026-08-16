//! Asset 存储机制:头指针 + 不可变修订 + 内容寻址 chunk。
//!
//! definition(typed serde enum)与存储形态之间的边界:
//! - [`Manifest`]:修订骨架——标量字段内联在 `fields`,可编辑单元存为
//!   `chunk_lists` 中的哈希数组,粒度按 kind 选;
//! - [`AssetDefinition`]:各 kind 的拆分/重组实现,manifest 不以裸
//!   `serde_json::Value` 穿过服务边界;
//! - [`chunk_hash`]:canonical 内容字节的 sha256,同一哈希体系同时服务
//!   存储寻址、HashEdit 锚定与并发校验。

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::entities::asset::AssetKind;
use crate::CoreError;

/// 内容寻址哈希(sha256 hex)。
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, ts_rs::TS)]
#[ts(as = "String")]
pub struct ChunkHash(pub String);

pub fn chunk_hash(bytes: &[u8]) -> ChunkHash {
    ChunkHash(hex::encode(Sha256::digest(bytes)))
}

/// 修订清单:标量骨架 + 命名 chunk 列表。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub fields: serde_json::Value,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub chunk_lists: BTreeMap<String, Vec<ChunkHash>>,
}

/// 待写入的 chunk 内容(canonical 字节)。
#[derive(Clone, Debug)]
pub struct ChunkPayload {
    pub hash: ChunkHash,
    pub bytes: Vec<u8>,
}

/// definition 拆分结果。
#[derive(Clone, Debug)]
pub struct SplitManifest {
    pub manifest: Manifest,
    pub chunks: Vec<ChunkPayload>,
}

/// 按哈希取 chunk 内容;由服务层从 db 装载。
pub type ChunkContents = BTreeMap<ChunkHash, Vec<u8>>;

/// definition ↔ manifest 编解码边界。
pub trait AssetDefinition: Sized {
    const KIND: AssetKind;

    fn split(&self) -> Result<SplitManifest, CoreError>;
    fn join(manifest: &Manifest, chunks: &ChunkContents) -> Result<Self, CoreError>;
}

/// 体量小的 kind 全内联:manifest.fields 即完整 definition,零 chunk。
pub fn split_inline<T: Serialize>(value: &T) -> Result<SplitManifest, CoreError> {
    Ok(SplitManifest {
        manifest: Manifest {
            fields: serde_json::to_value(value)?,
            chunk_lists: BTreeMap::new(),
        },
        chunks: Vec::new(),
    })
}

pub fn join_inline<T: DeserializeOwned>(manifest: &Manifest) -> Result<T, CoreError> {
    Ok(serde_json::from_value(manifest.fields.clone())?)
}

/// 把一组可编辑单元序列化为逐项 chunk,返回哈希列表与待写内容。
pub fn split_items<T: Serialize>(
    items: &[T],
) -> Result<(Vec<ChunkHash>, Vec<ChunkPayload>), CoreError> {
    let mut hashes = Vec::with_capacity(items.len());
    let mut payloads = Vec::new();
    for item in items {
        let bytes = serde_json::to_vec(item)?;
        let hash = chunk_hash(&bytes);
        hashes.push(hash.clone());
        payloads.push(ChunkPayload { hash, bytes });
    }
    Ok((hashes, payloads))
}

/// 按 manifest 中的哈希列表重组可编辑单元。
pub fn join_items<T: DeserializeOwned>(
    manifest: &Manifest,
    chunks: &ChunkContents,
    list: &str,
) -> Result<Vec<T>, CoreError> {
    let hashes = manifest
        .chunk_lists
        .get(list)
        .map_or(&[][..], Vec::as_slice);
    hashes
        .iter()
        .map(|hash| {
            let bytes = chunks
                .get(hash)
                .ok_or_else(|| CoreError::ChunkMissing(hash.0.clone()))?;
            Ok(serde_json::from_slice(bytes)?)
        })
        .collect()
}

impl Manifest {
    /// manifest 引用的全部 chunk 哈希(装载与 GC 的依据)。
    pub fn referenced_chunks(&self) -> impl Iterator<Item = &ChunkHash> {
        self.chunk_lists.values().flatten()
    }
}
