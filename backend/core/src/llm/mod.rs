//! LLM 接入层。
//!
//! 数据流（内容生成）：
//! 适配层传入 canonical（OpenAI Chat Completions）JSON
//! → [`exchange::ExchangePlan`] 转成渠道原生协议（gproxy-transform）
//! → [`client::LlmClient`] 经 reqwest 调用上游（gproxy-protocol 合成端点）
//! → 响应/SSE 反向转换回 canonical。
//! 凭证排序与失败分类见 [`pool`]，failover 执行在 `services::llm`。

pub mod capability;
pub mod client;
pub mod exchange;
pub mod pool;

/// 本地估算 token 数（tiktoken → 字符估算的降级阶梯，见 gproxy-tokenize）。
pub fn count_tokens_local(model: &str, body: &[u8]) -> u64 {
    gproxy_tokenize::count(model, body, None, ())
}
