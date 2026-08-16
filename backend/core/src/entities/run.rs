use sea_orm::entity::prelude::*;

/// 一次 run 的生命周期状态。
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    EnumIter,
    DeriveActiveEnum,
    serde::Deserialize,
    serde::Serialize,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::N(16))")]
pub enum RunStatus {
    #[sea_orm(string_value = "pending")]
    Pending,
    #[sea_orm(string_value = "running")]
    Running,
    #[sea_orm(string_value = "succeeded")]
    Succeeded,
    #[sea_orm(string_value = "failed")]
    Failed,
    #[sea_orm(string_value = "cancelled")]
    Cancelled,
}

/// Run:一次 pipeline 执行——多 Asset 槽位输入 → Asset 输出。
///
/// 输入槽位钉住 revision,因而可复现、可追溯;逐节点事件不持久化。
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "run")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub pipeline_asset_id: i32,
    pub status: RunStatus,
    /// `[{ slot, asset_id, revision }]`
    pub inputs: serde_json::Value,
    /// `[{ slot, asset_id, revision }]`
    pub outputs: serde_json::Value,
    pub error: Option<serde_json::Value>,
    pub usage: Option<serde_json::Value>,
    pub created_at: TimeDateTimeWithTimeZone,
    pub finished_at: Option<TimeDateTimeWithTimeZone>,
}

impl ActiveModelBehavior for ActiveModel {}
