//! Realtime 语音的专用 WebSocket 通道。
//!
//! 通用 `/api/llm` 只承载请求/响应,双工会话需要独立端点:浏览器连上来后
//! 先发一条会话配置,服务端据此建立上游连接,之后两个方向各自泵事件。

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use guanfu_core::llm::ir::platform::PlatformRequest;
use guanfu_core::llm::ir::realtime::RealtimeClientEvent;
use guanfu_core::llm::ir::OperationRequest;
use guanfu_core::services::llm::SemanticLlmOutput;
use guanfu_core::services::realtime::{
    RealtimeDownstream as Downstream, RealtimeHandshake as Handshake,
};
use crate::routes::RealtimeState;
pub async fn handler(upgrade: WebSocketUpgrade, State(state): State<RealtimeState>) -> Response {
    upgrade.on_upgrade(move |socket| session(socket, state))
}

async fn session(socket: WebSocket, realtime: RealtimeState) {
    let RealtimeState { state, token } = realtime;
    let (mut client_tx, mut client_rx) = socket.split();

    let handshake = match client_rx.next().await {
        Some(Ok(Message::Text(text))) => serde_json::from_str::<Handshake>(&text),
        _ => {
            // 没有首帧就没有会话配置,直接结束。
            return;
        }
    };
    let handshake = match handshake {
        Ok(handshake) => handshake,
        Err(error) => {
            let _ = send_down(
                &mut client_tx,
                Downstream::Error {
                    error: guanfu_core::CoreError::Json(error).api_error(),
                },
            )
            .await;
            return;
        }
    };

    if let Some(expected) = &token {
        if handshake.token.as_deref() != Some(expected.0.as_str()) {
            let _ = send_down(
                &mut client_tx,
                Downstream::Error {
                    error: guanfu_core::CoreError::WebSocket("unauthorized".to_owned())
                        .api_error(),
                },
            )
            .await;
            return;
        }
    }

    let output = state
        .llm
        .execute(
            &state.db,
            handshake.channel_id,
            OperationRequest::Platform(PlatformRequest::ConnectRealtime(handshake.request)),
        )
        .await;
    let connection = match output {
        Ok(SemanticLlmOutput::Realtime(connection)) => connection,
        Ok(_) => {
            let _ = send_down(
                &mut client_tx,
                Downstream::Error {
                    error: guanfu_core::CoreError::UnsupportedRouteImplementation {
                        implementation: "non-realtime route on the realtime endpoint",
                    }
                    .api_error(),
                },
            )
            .await;
            return;
        }
        Err(error) => {
            tracing::error!(error = ?error, "realtime connect failed");
            let _ = send_down(
                &mut client_tx,
                Downstream::Error {
                    error: error.api_error(),
                },
            )
            .await;
            return;
        }
    };

    let mut sender = connection.sender;
    let mut events = connection.events;
    let _ = send_down(&mut client_tx, Downstream::Ready).await;

    // 上行:浏览器 → 上游。客户端断开即结束会话。
    let uplink = tokio::spawn(async move {
        while let Some(Ok(message)) = client_rx.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            match serde_json::from_str::<RealtimeClientEvent>(&text) {
                Ok(event) => {
                    if sender.send(&event).await.is_err() {
                        break;
                    }
                }
                Err(error) => tracing::warn!(error = ?error, "unmodeled realtime client event"),
            }
        }
        let _ = sender.close().await;
    });

    // 下行:上游 → 浏览器。
    while let Some(item) = events.next().await {
        let frame = match item {
            Ok(event) => Downstream::Event {
                event: Box::new(event),
            },
            Err(error) => Downstream::Error {
                error: error.api_error(),
            },
        };
        if send_down(&mut client_tx, frame).await.is_err() {
            break;
        }
    }
    uplink.abort();
}

async fn send_down(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    frame: Downstream,
) -> Result<(), axum::Error> {
    let text = serde_json::to_string(&frame).expect("downstream frames are serializable");
    sink.send(Message::Text(text.into())).await
}
