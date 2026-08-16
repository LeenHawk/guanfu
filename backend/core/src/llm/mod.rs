//! LLM 接入层。
//!
//! 调用方只传入 provider-neutral semantic IR。渠道路由表选择目标 wire
//! 格式，codec 使用 gproxy 协议组件完成双向转换；provider 原生载荷不会
//! 越过 codec 边界。凭证排序与 failover 执行位于 `services::llm`。

pub mod client;
pub mod codec;
pub mod ir;
pub mod pool;
pub mod realtime;
pub mod routing;
pub mod wire;

/// 本地估算 token 数（tiktoken → 字符估算的降级阶梯，见 gproxy-tokenize）。
pub fn count_tokens_local(model: &str, body: &[u8]) -> u64 {
    gproxy_tokenize::count(model, body, None, ())
}

/// 安装 rustls 的加密后端。
///
/// `rustls-no-provider` 让我们选 ring 而不是默认的 AWS-LC;每个自建
/// reqwest client 之前都得先跑这一次,否则构造时直接 panic。
pub fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // 宿主已装则保留宿主选择。
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
