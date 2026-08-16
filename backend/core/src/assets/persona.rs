//! Persona definition:用户侧身份文本与头像引用。
//!
//! 默认 persona、会话锁定属于运行选择,不写入可分享 Persona。

use serde::{Deserialize, Serialize};

use super::character::InjectionRole;
use super::refs::{Extra, MediaRef, WorldBookRef};
use super::{join_inline, split_inline, AssetDefinition, ChunkContents, Manifest, SplitManifest};
use crate::entities::asset::AssetKind;
use crate::CoreError;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "version")]
pub enum PersonaDefinition {
    V1(PersonaV1),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct PersonaV1 {
    pub name: String,
    pub description: String,
    pub position: PersonaPosition,
    /// `position = at_depth` 时的注入深度与角色。
    pub depth: Option<u32>,
    pub role: Option<InjectionRole>,
    pub avatar: Option<MediaRef>,
    pub world_books: Vec<WorldBookRef>,
    #[serde(default)]
    pub extra: Extra,
}

/// persona 描述的注入位置;ST 的已废弃 `after_char` 不建模,导入时归一。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonaPosition {
    #[default]
    InPrompt,
    TopAuthorNote,
    BottomAuthorNote,
    AtDepth,
    None,
}

impl AssetDefinition for PersonaDefinition {
    const KIND: AssetKind = AssetKind::Persona;

    fn split(&self) -> Result<SplitManifest, CoreError> {
        split_inline(self)
    }

    fn join(manifest: &Manifest, _: &ChunkContents) -> Result<Self, CoreError> {
        join_inline(manifest)
    }
}
