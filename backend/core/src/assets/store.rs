//! AssetStore:二进制内容的存放。
//!
//! 字节以 `location = store` 的 chunk 形式与 db 内文本 chunk 共用同一
//! sha256 命名空间(计划 §2.2)。外部存储写入不进数据库事务:先写对象、
//! 再提交元数据,失败产生的孤儿对象由后续显式维护操作清理。

use std::path::PathBuf;

use crate::assets::ChunkHash;
use crate::CoreError;

/// 按内容哈希寻址的对象存储。
///
/// 接口是异步的:本地目录用不上,但对象存储是网络调用,让它同步会把
/// 运行时线程堵住。
#[async_trait::async_trait]
pub trait AssetStore: Send + Sync + std::fmt::Debug {
    /// 幂等写入:同一哈希重复写视为已存在。
    async fn put(&self, hash: &ChunkHash, bytes: &[u8]) -> Result<(), CoreError>;
    async fn get(&self, hash: &ChunkHash) -> Result<Vec<u8>, CoreError>;
    async fn exists(&self, hash: &ChunkHash) -> bool;
    async fn delete(&self, hash: &ChunkHash) -> Result<(), CoreError>;
}

/// 本地目录实现;按哈希前两位分桶,避免单目录条目过多。
#[derive(Clone, Debug)]
pub struct LocalAssetStore {
    root: PathBuf,
}

impl LocalAssetStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path_of(&self, hash: &ChunkHash) -> PathBuf {
        let (bucket, rest) = hash.0.split_at(2.min(hash.0.len()));
        self.root.join(bucket).join(rest)
    }
}

#[async_trait::async_trait]
impl AssetStore for LocalAssetStore {
    async fn put(&self, hash: &ChunkHash, bytes: &[u8]) -> Result<(), CoreError> {
        let path = self.path_of(hash);
        if path.exists() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(store_error)?;
        }
        // 先写临时文件再改名:半截文件不会被当成完整内容读到。
        let temporary = path.with_extension("partial");
        std::fs::write(&temporary, bytes).map_err(store_error)?;
        std::fs::rename(&temporary, &path).map_err(store_error)
    }

    async fn get(&self, hash: &ChunkHash) -> Result<Vec<u8>, CoreError> {
        std::fs::read(self.path_of(hash)).map_err(|_| CoreError::ChunkMissing(hash.0.clone()))
    }

    async fn exists(&self, hash: &ChunkHash) -> bool {
        self.path_of(hash).exists()
    }

    async fn delete(&self, hash: &ChunkHash) -> Result<(), CoreError> {
        match std::fs::remove_file(self.path_of(hash)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(store_error(error)),
        }
    }
}

pub(crate) fn store_error(error: impl std::fmt::Display) -> CoreError {
    CoreError::AssetStore {
        reason: error.to_string(),
    }
}
