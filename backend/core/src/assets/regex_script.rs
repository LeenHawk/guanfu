//! RegexScript definition:脚本内容与生效阶段。
//!
//! 信任/启用授权是本地运行状态:导入脚本默认不受信任,不能仅凭 Asset
//! 内容获得执行权限(计划 §4.5)。

use serde::{Deserialize, Serialize};

use super::refs::Extra;
use super::{join_inline, split_inline, AssetDefinition, ChunkContents, Manifest, SplitManifest};
use crate::entities::asset::AssetKind;
use crate::CoreError;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "version")]
pub enum RegexScriptDefinition {
    V1(RegexScriptV1),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct RegexScriptV1 {
    pub name: String,
    /// JS 风格 `/pattern/flags` 原文;编译在执行侧完成。
    pub find_regex: String,
    /// 支持 `{{match}}`、`$1`、`$<name>`。
    pub replace_string: String,
    /// 写入 replacement 前从捕获内容移除的字符串。
    pub trim_strings: Vec<String>,
    pub placements: Vec<RegexPlacement>,
    /// 只影响展示 / 只影响发送给模型的内容。
    pub display_only: bool,
    pub prompt_only: bool,
    pub run_on_edit: bool,
    pub substitute_macros: MacroSubstitution,
    /// 只处理指定聊天深度范围。
    pub min_depth: Option<u32>,
    pub max_depth: Option<u32>,
    #[serde(default)]
    pub extra: Extra,
}

/// 生效阶段;ST 的已废弃 `MD_DISPLAY`/`sendAs` 在导入时归一。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum RegexPlacement {
    UserInput,
    AiOutput,
    SlashCommand,
    WorldInfo,
    Reasoning,
}

/// find regex 中的宏处理方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum MacroSubstitution {
    #[default]
    None,
    Raw,
    Escaped,
}

impl AssetDefinition for RegexScriptDefinition {
    const KIND: AssetKind = AssetKind::RegexScript;

    fn split(&self) -> Result<SplitManifest, CoreError> {
        split_inline(self)
    }

    fn join(manifest: &Manifest, _: &ChunkContents) -> Result<Self, CoreError> {
        join_inline(manifest)
    }
}
