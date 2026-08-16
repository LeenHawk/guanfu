//! 访问控制:共享令牌。
//!
//! 桌面壳是单用户本地进程,不需要鉴权;Axum 一旦对外监听就必须挡住匿名
//! 访问——渠道里存着上游密钥,拿到 API 就等于拿到密钥的使用权。
//!
//! 这是**共享令牌**,不是多用户身份:所有持令牌者共用同一份资产与渠道。
//! 逐用户的数据隔离需要真正的账号体系,尚未实现。

use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

#[derive(Clone)]
pub struct Token(pub String);

/// 从环境读取令牌;未设置时返回 None(仅允许回环监听)。
pub fn from_env() -> Option<Token> {
    std::env::var("GUANFU_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty())
        .map(Token)
}

/// 未配置令牌时,拒绝对外监听——否则等于把上游密钥开放给整个网络。
pub fn guard_public_bind(address: &std::net::SocketAddr, token: &Option<Token>) -> Result<(), String> {
    if token.is_some() || address.ip().is_loopback() {
        return Ok(());
    }
    Err(format!(
        "refusing to listen on {address} without GUANFU_TOKEN: \
         the API exposes stored upstream credentials"
    ))
}

pub async fn require_token(
    axum::extract::State(token): axum::extract::State<Token>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    // 比较长度相同的字节序列,避免按前缀提前返回。
    let ok = presented.is_some_and(|presented| constant_time_eq(presented, &token.0));
    if !ok {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response();
    }
    next.run(request).await
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}
