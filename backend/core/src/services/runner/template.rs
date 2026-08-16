//! 内置聊天模板:超集 workflow 里参数最普通的那一份。
//!
//! "总结成世界书"之类只是换签名(inputs: history;outputs: world_book),
//! 因此模板本身不特殊——runner 认的是槽位名,不是模板名。

use sea_orm::ConnectionTrait;

use crate::assets::pipeline::{
    Edge, InputSlot, Node, NodeKind, OutputDecl, OutputOp, ParamSpec, ParamType,
    PipelineDefinition, PipelineV1,
};
use crate::entities::asset::AssetKind;
use crate::services::assets::AssetService;
use crate::CoreError;

use super::HISTORY_SLOT;

pub const CHAT_TEMPLATE_NAME: &str = "内置聊天";

/// ST 聊天模板:history + character/persona/world_books/preset → 追加历史。
pub fn chat_pipeline() -> PipelineDefinition {
    PipelineDefinition::V1(PipelineV1 {
        name: CHAT_TEMPLATE_NAME.to_owned(),
        inputs: vec![
            InputSlot {
                name: HISTORY_SLOT.to_owned(),
                kind: AssetKind::ChatHistory,
                many: false,
                required: true,
            },
            InputSlot {
                name: "character".to_owned(),
                kind: AssetKind::Character,
                many: false,
                required: false,
            },
            InputSlot {
                name: "persona".to_owned(),
                kind: AssetKind::Persona,
                many: false,
                required: false,
            },
            InputSlot {
                name: "world_books".to_owned(),
                kind: AssetKind::WorldBook,
                many: true,
                required: false,
            },
            InputSlot {
                name: "preset".to_owned(),
                kind: AssetKind::OpenAiChatPreset,
                many: false,
                required: true,
            },
        ],
        params: vec![
            ParamSpec {
                name: "user_message".to_owned(),
                ty: ParamType::Text,
                required: false,
            },
            ParamSpec {
                name: "trigger".to_owned(),
                ty: ParamType::Text,
                required: false,
            },
            // 渠道与模型由运行参数提供,不固定在可分享 pipeline 里。
            ParamSpec {
                name: "channel".to_owned(),
                ty: ParamType::Integer,
                required: true,
            },
            ParamSpec {
                name: "model".to_owned(),
                ty: ParamType::Text,
                required: true,
            },
        ],
        nodes: vec![
            Node {
                id: "ctx".to_owned(),
                kind: NodeKind::ContextBuild {
                    slots: vec![
                        HISTORY_SLOT.to_owned(),
                        "character".to_owned(),
                        "persona".to_owned(),
                        "world_books".to_owned(),
                        "preset".to_owned(),
                    ],
                },
            },
            Node {
                id: "gen".to_owned(),
                kind: NodeKind::Generate {
                    model_param: "model".to_owned(),
                    channel_param: "channel".to_owned(),
                    stream: true,
                },
            },
        ],
        edges: vec![Edge {
            from: "ctx.prompt".to_owned(),
            to: "gen.prompt".to_owned(),
        }],
        outputs: vec![OutputDecl {
            slot: HISTORY_SLOT.to_owned(),
            op: OutputOp::Append,
            from: "gen.result".to_owned(),
        }],
        extra: Default::default(),
    })
}

/// 取回内置模板 Asset;没有就建一个。
pub async fn ensure_chat_pipeline(db: &impl ConnectionTrait) -> Result<i32, CoreError> {
    let existing = AssetService::list(db, Some(AssetKind::Pipeline)).await?;
    if let Some(head) = existing
        .into_iter()
        .find(|head| head.name == CHAT_TEMPLATE_NAME)
    {
        return Ok(head.id);
    }
    let head = AssetService::create(db, CHAT_TEMPLATE_NAME, None, &chat_pipeline()).await?;
    Ok(head.id)
}

pub(super) async fn load_pipeline(
    db: &impl ConnectionTrait,
    id: i32,
) -> Result<PipelineDefinition, CoreError> {
    Ok(AssetService::load::<PipelineDefinition>(db, id)
        .await?
        .definition)
}
