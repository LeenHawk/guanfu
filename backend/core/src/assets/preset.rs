//! OpenAI Chat Preset definition:提示词编排与生成参数。
//!
//! channel、credential、base URL、proxy 与 model 选择不进入可分享 preset;
//! 连接配置由 channel/credential 负责(计划 §4.4)。

use serde::{Deserialize, Serialize};

use super::character::InjectionRole;
use super::refs::{Extra, RegexScriptRef};
use super::{join_inline, split_inline, AssetDefinition, ChunkContents, Manifest, SplitManifest};
use crate::entities::asset::AssetKind;
use crate::llm::ir::generation::{ReasoningOptions, SamplingOptions};
use crate::CoreError;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "version")]
pub enum OpenAiChatPresetDefinition {
    V1(OpenAiChatPresetV1),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct OpenAiChatPresetV1 {
    pub name: String,
    /// 全部提示词项,按 identifier 被 order profile 引用。
    pub prompts: Vec<PromptItem>,
    /// prompt-order profile;ST 用虚拟 character_id 区分,观复用具名 profile。
    pub prompt_orders: Vec<PromptOrderProfile>,
    pub sampling: SamplingOptions,
    pub reasoning: Option<ReasoningOptions>,
    pub context: ContextAssembly,
    pub regex_scripts: Vec<RegexScriptRef>,
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct PromptItem {
    /// 稳定标识;排序和覆盖按它关联。
    pub identifier: String,
    pub name: String,
    pub role: InjectionRole,
    /// marker 项可没有正文。
    pub content: String,
    /// 动态槽位(角色描述、历史、世界书等)而非静态文本。
    pub marker: Option<PromptMarker>,
    pub injection: PromptInjection,
    /// 禁止角色卡覆盖此提示词。
    pub forbid_overrides: bool,
    #[serde(default)]
    pub extra: Extra,
}

/// 内置动态槽位;未知 marker 名保留在 `PromptItem::extra`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum PromptMarker {
    WorldInfoBefore,
    WorldInfoAfter,
    CharacterDescription,
    CharacterPersonality,
    Scenario,
    PersonaDescription,
    DialogueExamples,
    ChatHistory,
}

/// 提示词的注入方式:相对编排,或按聊天深度注入。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PromptInjection {
    #[default]
    Relative,
    AtDepth {
        depth: u32,
        /// 同深度下的次序。
        order: i32,
        /// 限定生成类型;空表示不限。
        triggers: Vec<GenerationTrigger>,
    },
}

/// 生成类型;run 的 `trigger` 参数取值同源。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum GenerationTrigger {
    #[default]
    Normal,
    Continue,
    Impersonate,
    Swipe,
    Regenerate,
    Quiet,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct PromptOrderProfile {
    pub name: String,
    pub entries: Vec<PromptOrderEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct PromptOrderEntry {
    pub identifier: String,
    pub enabled: bool,
}

/// ST 上下文组装辅助字段:包裹世界书/场景/性格的模板与固定提示语。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct ContextAssembly {
    pub max_context_tokens: Option<u32>,
    pub max_output_tokens: Option<u32>,
    /// 内容为空的提示词是否仍然发送。
    pub send_if_empty: bool,
    pub world_info_format: String,
    pub scenario_format: String,
    pub personality_format: String,
    pub new_chat_prompt: String,
    pub new_example_chat_prompt: String,
    pub continue_nudge_prompt: String,
    pub impersonation_prompt: String,
}

impl AssetDefinition for OpenAiChatPresetDefinition {
    const KIND: AssetKind = AssetKind::OpenAiChatPreset;

    fn split(&self) -> Result<SplitManifest, CoreError> {
        split_inline(self)
    }

    fn join(manifest: &Manifest, _: &ChunkContents) -> Result<Self, CoreError> {
        join_inline(manifest)
    }
}
