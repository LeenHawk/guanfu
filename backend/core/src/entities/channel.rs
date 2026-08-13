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
    pub base_url: String,
    pub enabled: bool,
    pub created_at: TimeDateTimeWithTimeZone,
    #[sea_orm(has_many)]
    pub credentials: HasMany<super::credential::Entity>,
    #[sea_orm(has_many)]
    pub routing_rules: HasMany<super::routing_rule::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
