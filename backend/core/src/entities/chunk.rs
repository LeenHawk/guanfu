use sea_orm::entity::prelude::*;

/// chunk 字节的存放地。
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
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::N(8))")]
pub enum ChunkLocation {
    /// 文本内容,字节就在本表 `bytes` 列。
    #[sea_orm(string_value = "db")]
    Db,
    /// 二进制内容,字节在 AssetStore,按 hash 作 storage key。
    #[sea_orm(string_value = "store")]
    Store,
}

/// 内容寻址 chunk:跨资产共享的不可变内容单元。
///
/// hash 是 canonical 内容字节的 sha256(hex);写入按哈希幂等。
/// 孤儿 chunk 由显式维护操作按 manifest 引用清理。
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "chunk")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub hash: String,
    pub location: ChunkLocation,
    pub bytes: Option<Vec<u8>>,
    pub size: i64,
}

impl ActiveModelBehavior for ActiveModel {}
