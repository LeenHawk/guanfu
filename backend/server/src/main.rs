mod auth;
mod realtime;
mod routes;

use std::net::SocketAddr;

use guanfu_core::{AppConfig, AppState, LlmConfig};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://guanfu.db?mode=rwc".to_owned());
    let address = std::env::var("GUANFU_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:3000".to_owned())
        .parse::<SocketAddr>()?;
    let asset_root = std::env::var("GUANFU_ASSET_ROOT")
        .unwrap_or_else(|_| "guanfu-assets".to_owned())
        .into();
    let state = AppState::initialize(AppConfig {
        database_url,
        asset_root,
        llm: LlmConfig::default(),
    })
    .await?;
    auth::guard_public_bind(&state, &address).await?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "server listening");
    axum::serve(listener, routes::router(state)).await?;
    Ok(())
}
