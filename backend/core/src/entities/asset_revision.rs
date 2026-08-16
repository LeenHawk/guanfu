use sea_orm::entity::prelude::*;

/// 不可变修订:一个 Asset 在某个 revision 的 manifest。
///
/// manifest 是 definition 的骨架(标量内联 + 命名 chunk 哈希列表),
/// 结构见 `crate::assets::Manifest`;写入后永不改写。
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "asset_revision")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub asset_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    pub revision: i32,
    pub manifest: serde_json::Value,
    pub created_by_run_id: Option<i32>,
    pub created_at: TimeDateTimeWithTimeZone,
    #[sea_orm(belongs_to, from = "asset_id", to = "id")]
    pub asset: HasOne<super::asset::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
