mod commands;

use guanfu_core::llm::ir::realtime::RealtimeClientEvent;
use guanfu_core::{AppConfig, AppState, LlmConfig};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::Manager;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[derive(Default)]
struct ActiveLlmRequests(Mutex<HashMap<String, CancellationToken>>);

/// 进行中的 realtime 会话:客户端事件经此推给上游。
#[derive(Default)]
struct ActiveRealtimeSessions(
    Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<RealtimeClientEvent>>>,
);

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
                asset_root: data_dir.join("assets"),
                llm: LlmConfig::default(),
            }))?;
            app.manage(state);
            app.manage(ActiveLlmRequests::default());
            app.manage(ActiveRealtimeSessions::default());
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
            commands::execute_llm,
            commands::cancel_llm,
            commands::list_assets,
            commands::delete_asset,
            commands::set_asset_shared,
            commands::import_character,
            commands::bootstrap_chat,
            commands::create_chat_history,
            commands::load_chat_history,
            commands::fork_chat_history,
            commands::run_chat,
            commands::media_data_url,
            commands::generate_image,
            commands::edit_image,
            commands::create_speech,
            commands::transcribe,
            commands::create_video,
            commands::poll_video,
            commands::download_video,
            commands::connect_realtime,
            commands::send_realtime,
            commands::close_realtime,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
