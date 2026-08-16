//! Realtime WebSocket 传输:类型化双工连接。
//!
//! 事件在此处完成 IR ↔ wire 编解码;发送与接收两半可分别持有,
//! 便于壳层在同一任务里 `select!` 或拆成两个任务。

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::llm::codec::realtime as codec;
use crate::llm::ir::realtime::{RealtimeClientEvent, RealtimeServerEvent};
use crate::CoreError;

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

pub struct RealtimeConnection {
    pub sender: RealtimeSender,
    pub events: RealtimeEvents,
}

impl RealtimeConnection {
    pub(crate) fn new(socket: WsStream) -> Self {
        let (sink, stream) = socket.split();
        Self {
            sender: RealtimeSender { sink },
            events: RealtimeEvents { stream },
        }
    }
}

pub struct RealtimeSender {
    sink: futures_util::stream::SplitSink<WsStream, Message>,
}

impl RealtimeSender {
    pub async fn send(&mut self, event: &RealtimeClientEvent) -> Result<(), CoreError> {
        let wire = codec::encode_client_event(event)?;
        self.sink
            .send(Message::text(serde_json::to_string(&wire)?))
            .await
            .map_err(ws_error)
    }

    /// 发送关闭帧并结束连接。
    pub async fn close(&mut self) -> Result<(), CoreError> {
        self.sink.close().await.map_err(ws_error)
    }
}

pub struct RealtimeEvents {
    stream: futures_util::stream::SplitStream<WsStream>,
}

impl RealtimeEvents {
    /// 下一个语义事件;连接关闭返回 `None`。
    /// 未建模的服务端事件在此处被跳过(见 IR 文档)。
    pub async fn next(&mut self) -> Option<Result<RealtimeServerEvent, CoreError>> {
        loop {
            match self.stream.next().await? {
                Ok(Message::Text(text)) => match serde_json::from_str(text.as_str()) {
                    Ok(event) => match codec::decode_server_event(event) {
                        Some(event) => return Some(Ok(event)),
                        None => continue,
                    },
                    Err(error) => return Some(Err(CoreError::Json(error))),
                },
                Ok(Message::Close(_)) => return None,
                Ok(_) => continue,
                Err(error) => return Some(Err(ws_error(error))),
            }
        }
    }
}

pub(crate) fn ws_error(error: tokio_tungstenite::tungstenite::Error) -> CoreError {
    CoreError::WebSocket(error.to_string())
}
