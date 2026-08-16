use std::time::Duration;

use futures_util::StreamExt;
use gproxy_protocol::Provider;

use crate::llm::wire::{
    parse_json_sse, BinaryResponse, BinaryStreamResponse, JsonResponse, JsonSseResponse,
    MultipartBody, MultipartValue, RequestBody, ResponseMetadata, ResponseMode,
    UpstreamErrorResponse, WireRequest, WireResponse, WireResult,
};
use crate::CoreError;

/// 一次上游调用的目标：渠道地址 + 凭证。
pub struct CallTarget<'a> {
    pub base_url: &'a str,
    pub secret: &'a str,
}

pub struct LlmClient {
    http: reqwest::Client,
    connect_timeout: Duration,
    request_timeout: Duration,
}

impl Default for LlmClient {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmClient {
    pub fn new() -> Self {
        Self::with_timeouts(Duration::from_secs(10), Duration::from_secs(120))
    }

    pub fn with_timeouts(connect_timeout: Duration, request_timeout: Duration) -> Self {
        crate::llm::install_crypto_provider();
        let http = reqwest::Client::builder()
            .connect_timeout(connect_timeout)
            // 流式响应不设整请求超时,靠逐次读的空闲超时兜底。
            .read_timeout(request_timeout)
            .build()
            .expect("reqwest client construction is infallible with these options");
        Self {
            http,
            connect_timeout,
            request_timeout,
        }
    }

    /// Execute a fully prepared provider request. Authentication remains a
    /// channel concern and is injected after protocol encoding.
    pub async fn execute_wire(
        &self,
        target: &CallTarget<'_>,
        provider: Provider,
        request: WireRequest,
    ) -> Result<WireResult, CoreError> {
        let url = format!("{}{}", target.base_url.trim_end_matches('/'), request.path);
        let mut builder = self.http.request(request.method, url);
        // 整请求超时只适用于缓冲响应;对 SSE/二进制流会把长生成中途掐断。
        if matches!(
            request.response_mode,
            ResponseMode::Json | ResponseMode::Binary
        ) {
            builder = builder.timeout(self.request_timeout);
        }
        if !request.query.is_empty() {
            let query = request
                .query
                .iter()
                .map(|param| (&param.name, &param.value))
                .collect::<Vec<_>>();
            builder = builder.query(&query);
        }
        builder = apply_auth(builder, provider, target.secret).headers(request.headers);
        builder = match request.body {
            RequestBody::Empty => builder,
            RequestBody::Json(body) => builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.into_bytes()),
            RequestBody::Multipart(body) => builder.multipart(build_multipart(body)?),
        };

        let response = builder.send().await?;
        let metadata = ResponseMetadata {
            status: response.status(),
            headers: response.headers().clone(),
        };
        if !metadata.status.is_success() {
            return Ok(WireResult::Rejected(UpstreamErrorResponse {
                metadata,
                body: response.bytes().await?,
            }));
        }

        let response = match request.response_mode {
            ResponseMode::Json => WireResponse::Json(JsonResponse {
                metadata,
                body: crate::llm::wire::JsonBody::from_bytes(response.bytes().await?)?,
            }),
            ResponseMode::Binary => {
                let content_type = metadata
                    .headers
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                WireResponse::Binary(BinaryResponse {
                    metadata,
                    content_type,
                    body: response.bytes().await?,
                })
            }
            ResponseMode::BinaryStream => {
                let content_type = metadata
                    .headers
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                let stream = response
                    .bytes_stream()
                    .map(|item| item.map_err(CoreError::from));
                WireResponse::BinaryStream(BinaryStreamResponse {
                    metadata,
                    content_type,
                    stream: Box::pin(stream),
                })
            }
            ResponseMode::JsonSse => {
                let stream = response
                    .bytes_stream()
                    .map(|item| item.map_err(CoreError::from));
                WireResponse::JsonSse(JsonSseResponse {
                    metadata,
                    stream: parse_json_sse(Box::pin(stream)),
                })
            }
        };
        Ok(WireResult::Success(response))
    }

    /// 建立 Realtime WebSocket 会话并应用初始会话配置。
    /// 仅 OpenAI 系上游(端点合成对其他 provider 返回错误)。
    pub async fn connect_realtime(
        &self,
        target: &CallTarget<'_>,
        request: &crate::llm::ir::platform::ConnectRealtimeRequest,
    ) -> Result<crate::llm::realtime::RealtimeConnection, CoreError> {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let key = gproxy_protocol::OperationKey::provider(
            gproxy_protocol::Operation::ConnectRealtime,
            Provider::OpenAi,
        );
        let endpoint =
            gproxy_protocol::endpoint::request_target(key, &request.session.model.0, false)
                .map_err(|error| CoreError::Endpoint(error.to_string()))?;
        let base = target.base_url.trim_end_matches('/');
        let base = if let Some(rest) = base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = base.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            return Err(CoreError::Endpoint(format!(
                "realtime requires an http(s) base url, got {base}"
            )));
        };
        let mut url = format!("{base}{}", endpoint.path);
        if let Some(query) = endpoint.query {
            url.push('?');
            url.push_str(&query);
        }
        let mut ws_request = url
            .into_client_request()
            .map_err(|error| CoreError::WebSocket(error.to_string()))?;
        ws_request.headers_mut().insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", target.secret))
                .map_err(|error| CoreError::WebSocket(error.to_string()))?,
        );

        let (mut socket, _) = tokio::time::timeout(
            self.connect_timeout,
            tokio_tungstenite::connect_async(ws_request),
        )
        .await
        .map_err(|_| CoreError::WebSocket("realtime connect timed out".to_owned()))?
        .map_err(crate::llm::realtime::ws_error)?;

        let update = crate::llm::codec::realtime::encode_client_event(
            &crate::llm::ir::realtime::RealtimeClientEvent::UpdateSession {
                session: Box::new(request.session.clone()),
            },
        )?;
        socket
            .send(tokio_tungstenite::tungstenite::Message::text(
                serde_json::to_string(&update)?,
            ))
            .await
            .map_err(crate::llm::realtime::ws_error)?;

        Ok(crate::llm::realtime::RealtimeConnection::new(socket))
    }
}

fn build_multipart(body: MultipartBody) -> Result<reqwest::multipart::Form, CoreError> {
    let mut form = reqwest::multipart::Form::new();
    for part in body.parts {
        form = match part.value {
            MultipartValue::Text(value) => form.text(part.name, value),
            MultipartValue::File {
                filename,
                content_type,
                data,
            } => {
                let mut file = reqwest::multipart::Part::bytes(data.to_vec());
                if let Some(filename) = filename {
                    file = file.file_name(filename);
                }
                if let Some(content_type) = content_type {
                    file = file
                        .mime_str(&content_type)
                        .map_err(|error| CoreError::Endpoint(error.to_string()))?;
                }
                form.part(part.name, file)
            }
        };
    }
    Ok(form)
}

/// 临时按目标 wire family 选择标准鉴权头。
/// 特殊上游的请求准备逻辑后续应由独立 channel adapter 承担。
fn apply_auth(
    req: reqwest::RequestBuilder,
    provider: Provider,
    secret: &str,
) -> reqwest::RequestBuilder {
    match provider {
        Provider::Claude => req
            .header("x-api-key", secret)
            .header("anthropic-version", "2023-06-01"),
        Provider::Gemini => req.header("x-goog-api-key", secret),
        _ => req.bearer_auth(secret),
    }
}
