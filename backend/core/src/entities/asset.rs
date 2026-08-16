use sea_orm::entity::prelude::*;

/// Asset 分类;definition 的具体 schema 由 core 按 kind 解码。
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    EnumIter,
    DeriveActiveEnum,
    serde::Deserialize,
    serde::Serialize,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::N(24))")]
pub enum AssetKind {
    #[sea_orm(string_value = "character")]
    Character,
    #[sea_orm(string_value = "persona")]
    Persona,
    #[sea_orm(string_value = "world_book")]
    WorldBook,
    #[sea_orm(string_value = "open_ai_chat_preset")]
    OpenAiChatPreset,
    #[sea_orm(string_value = "regex_script")]
    RegexScript,
    #[sea_orm(string_value = "pipeline")]
    Pipeline,
    #[sea_orm(string_value = "chat_history")]
    ChatHistory,
    #[sea_orm(string_value = "media")]
    Media,
}

/// Asset 头指针:唯一可变的部分。
///
/// 内容本体是不可变的 `asset_revision`(manifest)与内容寻址 `chunk`;
/// 并发控制收敛为 `(id, head_revision)` 的 CAS。
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "asset")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub kind: AssetKind,
    pub name: String,
    /// 归属者;`None` 表示共享给所有人。
    pub owner_id: Option<i32>,
    pub head_revision: i32,
    pub created_at: TimeDateTimeWithTimeZone,
    pub updated_at: TimeDateTimeWithTimeZone,
    #[sea_orm(has_many)]
    pub revisions: HasMany<super::asset_revision::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
