//! 快照装载:按 run 输入槽位读取并解码 Asset。
//!
//! 全部数据库访问集中在这里,ContextBuild 之后不再有 I/O。

use sea_orm::ConnectionTrait;

use crate::assets::character::CharacterDefinition;
use crate::assets::chat_history::{ChatHistoryDefinition, ChatMessage};
use crate::assets::persona::PersonaDefinition;
use crate::assets::preset::OpenAiChatPresetDefinition;
use crate::assets::world_book::WorldBookDefinition;
use crate::context::{ContextSnapshot, HistoryTurn, TurnRole};
use crate::llm::ir::generation::{InputContent, OutputContent, OutputItem};
use crate::services::assets::AssetService;
use crate::services::runs::ResolvedSlot;
use crate::CoreError;

use super::{ChatRunRequest, HISTORY_SLOT};

pub async fn load_snapshot(
    db: &impl ConnectionTrait,
    inputs: &[ResolvedSlot],
    request: &ChatRunRequest,
    run_id: i32,
) -> Result<ContextSnapshot, CoreError> {
    let mut snapshot = ContextSnapshot {
        user_message: request.user_message.clone(),
        trigger: request.trigger,
        model: request.model.clone(),
        // 掷点种子取 run id:同一个 run 重放得到同一批世界书条目。
        activation_seed: run_id as u64,
        ..Default::default()
    };

    for slot in inputs {
        match slot.slot.as_str() {
            "character" => {
                let loaded = AssetService::load_revision::<CharacterDefinition>(
                    db,
                    slot.asset_id,
                    slot.revision,
                )
                .await?;
                let CharacterDefinition::V1(character) = loaded.definition;
                snapshot.character = Some(character);
            }
            "persona" => {
                let loaded = AssetService::load_revision::<PersonaDefinition>(
                    db,
                    slot.asset_id,
                    slot.revision,
                )
                .await?;
                let PersonaDefinition::V1(persona) = loaded.definition;
                snapshot.persona = Some(persona);
            }
            "world_books" => {
                let loaded = AssetService::load_revision::<WorldBookDefinition>(
                    db,
                    slot.asset_id,
                    slot.revision,
                )
                .await?;
                let WorldBookDefinition::V1(book) = loaded.definition;
                snapshot.world_books.push(book);
            }
            "preset" => {
                let loaded = AssetService::load_revision::<OpenAiChatPresetDefinition>(
                    db,
                    slot.asset_id,
                    slot.revision,
                )
                .await?;
                let OpenAiChatPresetDefinition::V1(preset) = loaded.definition;
                snapshot.preset = preset;
            }
            HISTORY_SLOT => {
                let loaded = AssetService::load_revision::<ChatHistoryDefinition>(
                    db,
                    slot.asset_id,
                    slot.revision,
                )
                .await?;
                let ChatHistoryDefinition::V1(history) = loaded.definition;
                snapshot.history = history.messages.iter().filter_map(to_turn).collect();
            }
            _ => {}
        }
    }
    Ok(snapshot)
}

/// 历史的纯文本视图;工具轮不进本阶段的编排。
fn to_turn(message: &ChatMessage) -> Option<HistoryTurn> {
    match message {
        ChatMessage::User { content, .. } => Some(HistoryTurn {
            role: TurnRole::User,
            text: input_text(content),
        }),
        ChatMessage::Assistant { output, .. } => Some(HistoryTurn {
            role: TurnRole::Assistant,
            text: output_text(output),
        }),
        ChatMessage::System { text, .. } => Some(HistoryTurn {
            role: TurnRole::System,
            text: text.clone(),
        }),
        ChatMessage::Tool { .. } => None,
    }
}

fn input_text(content: &[InputContent]) -> String {
    content
        .iter()
        .filter_map(|part| match part {
            InputContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 只取可见文本;reasoning 与 continuation 留在 Asset 里,不回灌提示词。
fn output_text(output: &[OutputItem]) -> String {
    output
        .iter()
        .filter_map(|item| match item {
            OutputItem::Message(message) => Some(
                message
                    .content
                    .iter()
                    .filter_map(|part| match part {
                        OutputContent::Text { text, .. } => Some(text.as_str()),
                        OutputContent::Refusal { text } => Some(text.as_str()),
                        OutputContent::SummaryText { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
