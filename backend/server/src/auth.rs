//! 访问控制:账号会话。
//!
//! 桌面壳是本地单用户进程,不需要鉴权;Axum 一旦对外监听就必须挡住匿名
//! 访问——渠道里存着上游密钥,拿到 API 就等于拿到密钥的使用权。
//!
//! 令牌由登录签发(见 `AuthService`),这里只负责把它换成 `Actor` 并挂到
//! 请求扩展上。

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use guanfu_core::services::auth::AuthService;
use guanfu_core::AppState;

/// 从 `Authorization: Bearer` 取令牌。
pub fn bearer(request: &Request) -> Option<&str> {
    request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}

/// 校验会话并把 `Actor` 放进请求扩展,供各 handler 取用。
pub async fn require_session(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(token) = bearer(&request).map(str::to_owned) else {
        return unauthorized();
    };
    match AuthService::actor_for(&state.db, &token).await {
        Ok((actor, _)) => {
            request.extensions_mut().insert(actor);
            next.run(request).await
        }
        Err(_) => unauthorized(),
    }
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
    )
        .into_response()
}

/// 引导期的门槛。
///
/// 首次注册端点不能要令牌(还没有账号可登录),但对外监听时这意味着
/// **谁先到谁当管理员**。所以:公网监听且尚无账号时,必须配
/// `GUANFU_BOOTSTRAP_TOKEN`,首次注册要带上它。
pub fn bootstrap_secret() -> Option<String> {
    std::env::var("GUANFU_BOOTSTRAP_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

pub async fn guard_public_bind(
    state: &AppState,
    address: &std::net::SocketAddr,
) -> Result<(), String> {
    if address.ip().is_loopback() {
        return Ok(());
    }
    let empty = AuthService::needs_setup(&state.db)
        .await
        .map_err(|error| error.to_string())?;
    if empty && bootstrap_secret().is_none() {
        return Err(format!(
            "refusing to listen on {address} with no accounts and no GUANFU_BOOTSTRAP_TOKEN: \
             the first caller would become administrator"
        ));
    }
    Ok(())
}
