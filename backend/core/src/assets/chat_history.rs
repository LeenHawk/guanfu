//! ChatHistory definition:聊天历史是一等 Asset,逐消息成 chunk。
//!
//! 消息内容直接复用生成 IR 的 typed 形态,不传裸协议 JSON:assistant 轮
//! 保存完整有序的 `OutputItem`(含 reasoning parts 与 continuation),
//! 因而同协议回放无需再拼接或重签名(计划 §4.8 / §5.1)。

use serde::{Deserialize, Serialize};

use super::refs::{CharacterRef, ChatHistoryRef, Extra, PersonaRef, PipelineRef, PresetRef};
use super::{join_items, split_items, AssetDefinition, ChunkContents, Manifest, SplitManifest};
use crate::entities::asset::AssetKind;
use crate::llm::ir::generation::{FinishReason, InputContent, OutputItem, ToolResult};
use crate::llm::ir::{ModelId, Usage};
use crate::CoreError;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "version")]
pub enum ChatHistoryDefinition {
    V1(ChatHistoryV1),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct ChatHistoryV1 {
    pub title: String,
    pub messages: Vec<ChatMessage>,
    /// 会话默认绑定;可被单次 run 覆盖。
    pub bindings: SessionBindings,
    /// 分支来源:fork 自哪段历史的第几条消息之后。
    pub forked_from: Option<ForkOrigin>,
    #[serde(default)]
    pub extra: Extra,
}

/// 单条消息——内容寻址的可编辑单元(HashEdit 的锚点)。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum ChatMessage {
    User {
        content: Vec<InputContent>,
        created_at_ms: i64,
    },
    /// 模型输出原样保存:有序 reasoning / tool call / 文本。
    Assistant {
        output: Vec<OutputItem>,
        model: ModelId,
        finish: FinishReason,
        usage: Option<Usage>,
        created_at_ms: i64,
    },
    Tool {
        result: ToolResult,
        created_at_ms: i64,
    },
    /// 会话内的 system 旁白(与 preset 的系统提示词不同)。
    System { text: String, created_at_ms: i64 },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct SessionBindings {
    pub character: Option<CharacterRef>,
    pub persona: Option<PersonaRef>,
    pub preset: Option<PresetRef>,
    pub pipeline: Option<PipelineRef>,
    pub channel_id: Option<i32>,
    pub model: Option<ModelId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct ForkOrigin {
    pub source: ChatHistoryRef,
    pub source_revision: i32,
    /// 保留到源历史的第几条消息(独占上界)。
    pub message_count: u32,
}

impl AssetDefinition for ChatHistoryDefinition {
    const KIND: AssetKind = AssetKind::ChatHistory;

    fn split(&self) -> Result<SplitManifest, CoreError> {
        let Self::V1(history) = self;
        let (hashes, chunks) = split_items(&history.messages)?;
        let mut manifest = Manifest {
            fields: serde_json::to_value(HistoryHead {
                version: "V1",
                title: &history.title,
                bindings: &history.bindings,
                forked_from: &history.forked_from,
                extra: &history.extra,
            })?,
            chunk_lists: Default::default(),
        };
        manifest.chunk_lists.insert(MESSAGES.to_owned(), hashes);
        Ok(SplitManifest { manifest, chunks })
    }

    fn join(manifest: &Manifest, chunks: &ChunkContents) -> Result<Self, CoreError> {
        let head: OwnedHistoryHead = serde_json::from_value(manifest.fields.clone())?;
        Ok(Self::V1(ChatHistoryV1 {
            title: head.title,
            messages: join_items(manifest, chunks, MESSAGES)?,
            bindings: head.bindings,
            forked_from: head.forked_from,
            extra: head.extra,
        }))
    }
}

/// 消息 chunk 列表名;追加与 HashEdit 都作用于它。
pub const MESSAGES: &str = "messages";

#[derive(Serialize)]
struct HistoryHead<'a> {
    version: &'static str,
    title: &'a str,
    bindings: &'a SessionBindings,
    forked_from: &'a Option<ForkOrigin>,
    extra: &'a Extra,
}

#[derive(Deserialize)]
struct OwnedHistoryHead {
    title: String,
    #[serde(default)]
    bindings: SessionBindings,
    #[serde(default)]
    forked_from: Option<ForkOrigin>,
    #[serde(default)]
    extra: Extra,
}
