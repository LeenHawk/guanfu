//! HashEdit:修订的锚定原语(计划 §5.2)。
//!
//! 可编辑单元就是 chunk,`target_hash` 即 chunk 哈希——编辑原语、存储寻址
//! 与并发校验共用同一个哈希体系。一批指令原子地作用于某个 revision 的
//! manifest,提交为下一个 revision;与头指针 CAS 正交组合。
//!
//! 锚的是内容而不是下标:目标缺失或不唯一都显式报错,不静默错位。

use serde::{Deserialize, Serialize};

use super::{canonical_bytes, chunk_hash, ChunkHash, ChunkPayload, Manifest};
use crate::CoreError;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct HashEdit {
    pub target_hash: ChunkHash,
    pub op: HashEditOp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HashEditOp {
    /// 用新内容替换目标单元。
    Replace { new_content: serde_json::Value },
    /// 删除目标单元。
    Delete,
    /// 在目标单元之后插入新内容。
    InsertAfter { new_content: serde_json::Value },
}

/// 把一批指令应用到 manifest 的某个 chunk 列表。
///
/// 全部指令按目标在原列表中的位置解析后一次性生效,指令之间互不影响
/// 位置——避免"前一条插入导致后一条锚点漂移"。
pub fn apply_hash_edits(
    manifest: &Manifest,
    list: &str,
    edits: &[HashEdit],
) -> Result<(Manifest, Vec<ChunkPayload>), CoreError> {
    if edits.is_empty() {
        return Ok((manifest.clone(), Vec::new()));
    }
    // 列表缺失等价于没有可锚定单元:每条指令都会得到 stale 错误。
    let current = manifest
        .chunk_lists
        .get(list)
        .map_or(&[][..], Vec::as_slice);

    let mut planned: Vec<Option<Plan>> = vec![None; current.len()];
    let mut payloads = Vec::new();
    for edit in edits {
        let index = locate(current, &edit.target_hash)?;
        if planned[index].is_some() {
            return Err(CoreError::HashEditAmbiguous {
                hash: edit.target_hash.0.clone(),
                reason: "multiple edits target the same unit".to_owned(),
            });
        }
        planned[index] = Some(match &edit.op {
            HashEditOp::Delete => Plan::Delete,
            HashEditOp::Replace { new_content } => {
                let payload = encode(new_content)?;
                let hash = payload.hash.clone();
                payloads.push(payload);
                Plan::Replace(hash)
            }
            HashEditOp::InsertAfter { new_content } => {
                let payload = encode(new_content)?;
                let hash = payload.hash.clone();
                payloads.push(payload);
                Plan::InsertAfter(hash)
            }
        });
    }

    let mut next = Vec::with_capacity(current.len() + edits.len());
    for (index, hash) in current.iter().enumerate() {
        match planned[index].take() {
            None => next.push(hash.clone()),
            Some(Plan::Delete) => {}
            Some(Plan::Replace(new_hash)) => next.push(new_hash),
            Some(Plan::InsertAfter(new_hash)) => {
                next.push(hash.clone());
                next.push(new_hash);
            }
        }
    }

    let mut manifest = manifest.clone();
    manifest.chunk_lists.insert(list.to_owned(), next);
    Ok((manifest, payloads))
}

#[derive(Clone)]
enum Plan {
    Delete,
    Replace(ChunkHash),
    InsertAfter(ChunkHash),
}

fn locate(list: &[ChunkHash], target: &ChunkHash) -> Result<usize, CoreError> {
    let mut found = None;
    for (index, hash) in list.iter().enumerate() {
        if hash == target {
            if found.is_some() {
                return Err(CoreError::HashEditAmbiguous {
                    hash: target.0.clone(),
                    reason: "the anchored content appears more than once".to_owned(),
                });
            }
            found = Some(index);
        }
    }
    found.ok_or_else(|| CoreError::HashEditStale {
        hash: target.0.clone(),
        reason: "no unit with this hash in the revision".to_owned(),
    })
}

fn encode(content: &serde_json::Value) -> Result<ChunkPayload, CoreError> {
    let bytes = canonical_bytes(content)?;
    Ok(ChunkPayload {
        hash: chunk_hash(&bytes),
        bytes,
    })
}
