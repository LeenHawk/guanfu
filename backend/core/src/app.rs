use std::sync::Arc;
use std::time::Duration;

use sea_orm::DatabaseConnection;

use crate::assets::{AssetStore, LocalAssetStore, S3AssetStore};
use crate::services::llm::LlmService;
use crate::{db, CoreError};

#[derive(Clone, Debug)]
pub struct LlmConfig {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(120),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub database_url: String,
    /// 二进制内容的本地存放目录。
    pub asset_root: std::path::PathBuf,
    pub llm: LlmConfig,
}

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub llm: Arc<LlmService>,
    pub assets: Arc<dyn AssetStore>,
    pub config: AppConfig,
}

impl AppState {
    /// 应用的唯一数据库启动入口：连接后立即同步 entity-first schema。
    pub async fn initialize(config: AppConfig) -> Result<Self, CoreError> {
        let db = db::connect(&config.database_url).await?;
        db::sync_schema(&db).await?;
        let llm = Arc::new(LlmService::with_timeouts(
            config.llm.connect_timeout,
            config.llm.request_timeout,
        ));
        // 配了 S3 就用对象存储,否则本地目录;多实例部署必须走前者。
        let assets: Arc<dyn AssetStore> = match S3AssetStore::from_env() {
            Some(s3) => {
                tracing::info!(bucket = %s3.bucket, "using s3 asset store");
                Arc::new(S3AssetStore::new(s3))
            }
            None => {
                std::fs::create_dir_all(&config.asset_root).map_err(|error| {
                    CoreError::AssetStore {
                        reason: error.to_string(),
                    }
                })?;
                Arc::new(LocalAssetStore::new(config.asset_root.clone()))
            }
        };
        Ok(Self {
            db,
            llm,
            assets,
            config,
        })
    }
}
