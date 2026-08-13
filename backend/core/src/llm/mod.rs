//! LLM 接入层。
//!
//! 数据流（内容生成）：
//! 适配层传入带明确 [`gproxy_protocol::OperationKey`] 的 wire JSON
//! → 渠道路由表决定直通、转换、本地处理或不支持
//! → [`transform::TransformPlan`] 按需转换协议（gproxy-transform）
//! → [`client::LlmClient`] 经 reqwest 调用上游（gproxy-protocol 合成端点）
//! → 响应/SSE 反向转换回调用方的源 wire 格式。
//! 凭证排序与失败分类见 [`pool`]，failover 执行在 `services::llm`。

pub mod client;
pub mod pool;
pub mod routing;
pub mod transform;

/// 本地估算 token 数（tiktoken → 字符估算的降级阶梯，见 gproxy-tokenize）。
pub fn count_tokens_local(model: &str, body: &[u8]) -> u64 {
    gproxy_tokenize::count(model, body, None, ())
}
