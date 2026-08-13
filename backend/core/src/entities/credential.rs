use sea_orm::entity::prelude::*;

/// 凭证：隶属某个渠道，可多个，支持轮换与 failover。
///
/// 可用性 = `!disabled` 且冷却已过期（`cooldown_until` 为空或早于当前时间）。
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "credential")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub channel_id: i32,
    pub label: String,
    pub secret: String,
    /// 轮换权重，越大越优先。
    pub weight: i32,
    pub disabled: bool,
    pub failure_count: i32,
    pub cooldown_until: Option<TimeDateTimeWithTimeZone>,
    pub last_used_at: Option<TimeDateTimeWithTimeZone>,
    #[sea_orm(belongs_to, from = "channel_id", to = "id")]
    pub channel: HasOne<super::channel::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
