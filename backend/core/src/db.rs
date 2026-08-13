use sea_orm::{Database, DatabaseConnection, DbErr};

/// 连接数据库。`url` 形如 `sqlite://guanfu.db?mode=rwc`。
pub async fn connect(url: &str) -> Result<DatabaseConnection, DbErr> {
    Database::connect(url).await
}

/// entity-first：按实体注册表同步 schema（只增不删）。
pub async fn sync_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    db.get_schema_registry("guanfu_core::entities::*")
        .sync(db)
        .await
}
