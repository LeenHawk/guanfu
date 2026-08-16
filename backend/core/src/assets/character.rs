//! Character definition:CCv2 可移植内容 + 观复的 typed 引用。
//!
//! SillyTavern 本地 chat、收藏、文件名与 proxy 状态不进入可分享 definition。

use serde::{Deserialize, Serialize};

use super::refs::{Extra, MediaRef, RegexScriptRef, WorldBookRef};
use super::{join_inline, split_inline, AssetDefinition, ChunkContents, Manifest, SplitManifest};
use crate::entities::asset::AssetKind;
use crate::CoreError;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "version")]
pub enum CharacterDefinition {
    V1(CharacterV1),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct CharacterV1 {
    pub name: String,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub creator_notes: String,
    /// 主系统提示词覆盖(CCv2 `system_prompt`)。
    pub system_prompt: String,
    /// 历史后指令覆盖(CCv2 `post_history_instructions`)。
    pub post_history_instructions: String,
    /// 有序开场白:第一项对应 `first_mes`,其余对应 `alternate_greetings`。
    pub greetings: Vec<String>,
    /// 示例对话(CCv2 `mes_example`)原文。
    pub example_dialogue: String,
    pub tags: Vec<String>,
    pub creator: String,
    pub character_version: String,
    /// 角色专用的按深度提示词(ST `extensions.depth_prompt`)。
    pub depth_prompt: Option<DepthPrompt>,
    pub world_books: Vec<WorldBookRef>,
    pub regex_scripts: Vec<RegexScriptRef>,
    pub avatar: Option<MediaRef>,
    /// CCv2 extensions 与未识别字段的保真数据。
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct DepthPrompt {
    pub prompt: String,
    pub depth: u32,
    pub role: InjectionRole,
}

/// 注入消息的角色;ST 以 0/1/2 编码,观复用具名值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum InjectionRole {
    #[default]
    System,
    User,
    Assistant,
}

impl AssetDefinition for CharacterDefinition {
    const KIND: AssetKind = AssetKind::Character;

    fn split(&self) -> Result<SplitManifest, CoreError> {
        split_inline(self)
    }

    fn join(manifest: &Manifest, _: &ChunkContents) -> Result<Self, CoreError> {
        join_inline(manifest)
    }
}
