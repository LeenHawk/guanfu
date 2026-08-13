use sea_orm::entity::prelude::*;

/// 渠道：一个可用的上游 LLM 服务端点。
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "channel")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique)]
    pub name: String,
    /// Provider 家族："openai" | "claude" | "gemini"
    pub provider: String,
    pub base_url: String,
    pub enabled: bool,
    pub created_at: DateTimeUtc,
    #[sea_orm(has_many)]
    pub credentials: HasMany<super::credential::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
