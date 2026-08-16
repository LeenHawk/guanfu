//! guanfu 共享业务核心。
//!
//! 模块地图：
//! - [`entities`]：SeaORM 实体（entity-first，schema 由 sync 生成）
//! - [`llm`]：LLM 接入层——能力映射、reqwest 客户端、协议转换、凭证轮换/failover
//! - [`services`]：面向适配层（tauri / axum）的服务接口，与传输层无关
//! - [`db`]：数据库连接与 schema 同步

mod app;
pub mod assets;
pub mod context;
mod db;
pub mod entities;
pub mod error;
pub mod exchange;
pub mod llm;
pub mod services;

pub use app::{AppConfig, AppState, LlmConfig};
pub use error::CoreError;
