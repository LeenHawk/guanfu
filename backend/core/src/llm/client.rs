use std::sync::Once;
use std::time::Duration;

use futures_util::StreamExt;
use gproxy_protocol::Provider;

use crate::llm::wire::{
    parse_json_sse, BinaryResponse, JsonResponse, JsonSseResponse, MultipartBody, MultipartValue,
    RequestBody, ResponseMetadata, ResponseMode, UpstreamErrorResponse, WireRequest, WireResponse,
    WireResult,
};
use crate::CoreError;

/// 一次上游调用的目标：渠道地址 + 凭证。
pub struct CallTarget<'a> {
    pub base_url: &'a str,
    pub secret: &'a str,
}

pub struct LlmClient {
    http: reqwest::Client,
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
        static INSTALL_RUSTLS_PROVIDER: Once = Once::new();
        INSTALL_RUSTLS_PROVIDER.call_once(|| {
            // reqwest 0.13 的 `rustls-no-provider` 允许选择 ring，避免默认
            // AWS-LC provider；若宿主已安装 provider，则保留宿主选择。
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
        let http = reqwest::Client::builder()
            .connect_timeout(connect_timeout)
            .build()
            .expect("reqwest client construction is infallible with these options");
        Self {
            http,
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
        if request.response_mode != ResponseMode::JsonSse {
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
