use std::pin::Pin;
use std::sync::Once;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use gproxy_protocol::endpoint::request_target;
use gproxy_protocol::{OperationKey, Provider};

use crate::CoreError;

/// 一次上游调用的目标：渠道地址 + 凭证。
pub struct CallTarget<'a> {
    pub base_url: &'a str,
    pub secret: &'a str,
}

pub struct LlmResponse {
    pub status: u16,
    pub body: Bytes,
}

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, CoreError>> + Send>>;

pub enum LlmReply {
    Complete(LlmResponse),
    /// 流式（SSE）响应，body 为原始字节流。
    Stream {
        status: u16,
        stream: ByteStream,
    },
}

impl LlmReply {
    pub fn status(&self) -> u16 {
        match self {
            LlmReply::Complete(r) => r.status,
            LlmReply::Stream { status, .. } => *status,
        }
    }
}

pub struct LlmClient {
    http: reqwest::Client,
}

impl Default for LlmClient {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmClient {
    pub fn new() -> Self {
        static INSTALL_RUSTLS_PROVIDER: Once = Once::new();
        INSTALL_RUSTLS_PROVIDER.call_once(|| {
            // reqwest 0.13 的 `rustls-no-provider` 允许选择 ring，避免默认
            // AWS-LC provider；若宿主已安装 provider，则保留宿主选择。
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client construction is infallible with these options");
        Self { http }
    }

    /// 发起一次上游请求。`body` 为目标协议的 JSON 字节。
    ///
    /// 非流式或非 2xx 时聚合完整响应体；流式成功时返回原始字节流。
    pub async fn execute(
        &self,
        target: &CallTarget<'_>,
        key: OperationKey,
        model: &str,
        stream: bool,
        body: Option<Vec<u8>>,
    ) -> Result<LlmReply, CoreError> {
        let rt = request_target(key, model, stream)
            .map_err(|e| CoreError::Endpoint(format!("{e:?}")))?;
        let mut url = format!("{}{}", target.base_url.trim_end_matches('/'), rt.path);
        if let Some(q) = &rt.query {
            url.push('?');
            url.push_str(q);
        }
        let method: reqwest::Method = rt.method.into();
        let mut req = self.http.request(method, url);
        req = apply_auth(req, key.provider_family(), target.secret);
        if let Some(body) = body {
            req = req
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        if stream && resp.status().is_success() {
            let s = resp.bytes_stream().map(|r| r.map_err(CoreError::from));
            Ok(LlmReply::Stream {
                status,
                stream: Box::pin(s),
            })
        } else {
            let body = resp.bytes().await?;
            Ok(LlmReply::Complete(LlmResponse { status, body }))
        }
    }
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
