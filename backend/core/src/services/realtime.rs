//! Realtime 会话在壳层之间共享的帧形状。
//!
//! Axum WebSocket 与 Tauri Channel 用同一套下行帧,前端只写一份处理逻辑。

use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::llm::ir::realtime::RealtimeServerEvent;

/// 上行首帧:选渠道并给出会话配置(WebSocket 端点用;Tauri 走命令参数)。
#[derive(Clone, Debug, Deserialize, Serialize, ts_rs::TS)]
pub struct RealtimeHandshake {
    /// 共享令牌;WebSocket 握手带不了 Authorization 头,又不能塞进 URL
    /// (query 会进访问日志),所以随首帧校验。
    #[serde(default)]
    pub token: Option<String>,
    pub channel_id: i32,
    pub request: crate::llm::ir::platform::ConnectRealtimeRequest,
}

/// 下行帧:上游语义事件,或本端错误。
#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RealtimeDownstream {
    /// 上游已连通,可以开始推音频。
    Ready,
    Event {
        event: Box<RealtimeServerEvent>,
    },
    Error {
        error: ApiError,
    },
}
