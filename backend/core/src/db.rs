use std::time::Duration;

use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr};

/// 连接数据库。`url` 形如 `sqlite://guanfu.db?mode=rwc`。
///
/// 语句日志走 trace 级(默认不出),超过阈值的查询单独按 warn 报出来——
/// 慢查询要能在正常日志里看到,而不是靠临时打开全量 SQL 日志。
pub(crate) async fn connect(url: &str) -> Result<DatabaseConnection, DbErr> {
    let mut options = ConnectOptions::new(url.to_owned());
    options
        .sqlx_logging_level(tracing::log::LevelFilter::Trace)
        .sqlx_slow_statements_logging_settings(
            tracing::log::LevelFilter::Warn,
            Duration::from_millis(200),
        );
    Database::connect(options).await
}

/// entity-first：按实体注册表同步 schema（只增不删）。
pub(crate) async fn sync_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    db.get_schema_registry("guanfu_core::entities::*")
        .sync(db)
        .await
}
