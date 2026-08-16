//! 聊天门面:为 UI 提供开箱可用的默认资产与历史读写。
//!
//! 聊天只是默认的 run 模板,这里不含执行逻辑——执行在
//! [`crate::services::runner`]。

use sea_orm::ConnectionTrait;
use serde::{Deserialize, Serialize};

use crate::assets::character::InjectionRole;
use crate::assets::chat_history::{
    ChatHistoryDefinition, ChatHistoryV1, ChatMessage, SessionBindings,
};
use crate::assets::preset::{
    ContextAssembly, OpenAiChatPresetDefinition, OpenAiChatPresetV1, PromptInjection, PromptItem,
    PromptMarker, PromptOrderEntry, PromptOrderProfile,
};
use crate::entities::asset::AssetKind;
use crate::services::assets::{AssetHeadDto, AssetService};
use crate::services::runner::ensure_chat_pipeline;
use crate::CoreError;

/// UI 启动时需要的默认资产。
#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct ChatBootstrap {
    pub pipeline_asset_id: i32,
    pub preset_asset_id: i32,
}

/// 一段历史的视图:消息 + 当前 revision(用于 HashEdit 与并发)。
#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct ChatHistoryView {
    pub head: AssetHeadDto,
    pub revision: i32,
    pub title: String,
    pub bindings: SessionBindings,
    pub messages: Vec<ChatMessage>,
}

pub const DEFAULT_PRESET_NAME: &str = "默认预设";

pub struct ChatService;

impl ChatService {
    /// 确保内置模板与默认预设存在。
    pub async fn bootstrap(db: &impl ConnectionTrait) -> Result<ChatBootstrap, CoreError> {
        let pipeline_asset_id = ensure_chat_pipeline(db).await?;
        let existing = AssetService::list(db, Some(AssetKind::OpenAiChatPreset)).await?;
        let preset_asset_id = match existing
            .into_iter()
            .find(|head| head.name == DEFAULT_PRESET_NAME)
        {
            Some(head) => head.id,
            None => {
                AssetService::create(
                    db,
                    DEFAULT_PRESET_NAME,
                    None,
                    &OpenAiChatPresetDefinition::V1(default_preset()),
                )
                .await?
                .id
            }
        };
        Ok(ChatBootstrap {
            pipeline_asset_id,
            preset_asset_id,
        })
    }

    pub async fn create_history(
        db: &impl ConnectionTrait,
        title: &str,
        bindings: SessionBindings,
    ) -> Result<AssetHeadDto, CoreError> {
        AssetService::create(
            db,
            title,
            None,
            &ChatHistoryDefinition::V1(ChatHistoryV1 {
                title: title.to_owned(),
                bindings,
                ..Default::default()
            }),
        )
        .await
    }

    /// 分支:复制到指定消息数为止,并记录来源。
    ///
    /// manifest 复制 + chunk 结构共享,因此 fork 不按历史长度收费。
    pub async fn fork_history(
        db: &impl ConnectionTrait,
        id: i32,
        message_count: u32,
        title: &str,
    ) -> Result<AssetHeadDto, CoreError> {
        use crate::assets::refs::ChatHistoryRef;

        let loaded = AssetService::load::<ChatHistoryDefinition>(db, id).await?;
        let ChatHistoryDefinition::V1(mut history) = loaded.definition;
        history.messages.truncate(message_count as usize);
        history.title = title.to_owned();
        history.forked_from = Some(crate::assets::chat_history::ForkOrigin {
            source: ChatHistoryRef(id),
            source_revision: loaded.revision,
            message_count,
        });
        AssetService::create(db, title, None, &ChatHistoryDefinition::V1(history)).await
    }

    pub async fn load_history(
        db: &impl ConnectionTrait,
        id: i32,
    ) -> Result<ChatHistoryView, CoreError> {
        let loaded = AssetService::load::<ChatHistoryDefinition>(db, id).await?;
        let ChatHistoryDefinition::V1(history) = loaded.definition;
        Ok(ChatHistoryView {
            head: loaded.head,
            revision: loaded.revision,
            title: history.title,
            bindings: history.bindings,
            messages: history.messages,
        })
    }
}

/// 最小可用的 ST 形态预设:标记槽位齐全,不含任何连接配置。
fn default_preset() -> OpenAiChatPresetV1 {
    let items = [
        (
            "main",
            "你正在扮演 {{char}},与 {{user}} 对话。保持角色的语气与设定。",
            None,
        ),
        ("worldInfoBefore", "", Some(PromptMarker::WorldInfoBefore)),
        (
            "charDescription",
            "",
            Some(PromptMarker::CharacterDescription),
        ),
        (
            "charPersonality",
            "",
            Some(PromptMarker::CharacterPersonality),
        ),
        ("scenario", "", Some(PromptMarker::Scenario)),
        (
            "personaDescription",
            "",
            Some(PromptMarker::PersonaDescription),
        ),
        ("worldInfoAfter", "", Some(PromptMarker::WorldInfoAfter)),
        ("dialogueExamples", "", Some(PromptMarker::DialogueExamples)),
        ("chatHistory", "", Some(PromptMarker::ChatHistory)),
    ];
    let prompts: Vec<PromptItem> = items
        .iter()
        .map(|(identifier, content, marker)| PromptItem {
            identifier: (*identifier).to_owned(),
            name: (*identifier).to_owned(),
            role: InjectionRole::System,
            content: (*content).to_owned(),
            marker: *marker,
            injection: PromptInjection::Relative,
            forbid_overrides: false,
            extra: Default::default(),
        })
        .collect();
    let order = PromptOrderProfile {
        name: "default".to_owned(),
        entries: prompts
            .iter()
            .map(|prompt| PromptOrderEntry {
                identifier: prompt.identifier.clone(),
                enabled: true,
            })
            .collect(),
    };
    OpenAiChatPresetV1 {
        name: DEFAULT_PRESET_NAME.to_owned(),
        prompts,
        prompt_orders: vec![order],
        context: ContextAssembly {
            max_output_tokens: Some(1024),
            ..Default::default()
        },
        ..Default::default()
    }
}
