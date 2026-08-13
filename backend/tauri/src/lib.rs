mod commands;

use guanfu_core::{AppConfig, AppState, LlmConfig};
use tauri::Manager;
use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let database_url =
                format!("sqlite://{}?mode=rwc", data_dir.join("guanfu.db").display());
            let state = tauri::async_runtime::block_on(AppState::initialize(AppConfig {
                database_url,
                llm: LlmConfig::default(),
            }))?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::create_channel,
            commands::list_channels,
            commands::set_channel_enabled,
            commands::delete_channel,
            commands::add_credential,
            commands::list_credentials,
            commands::remove_credential,
            commands::put_routing_rule,
            commands::list_routing_rules,
            commands::remove_routing_rule,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
