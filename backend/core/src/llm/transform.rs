use gproxy_protocol::OperationKey;
use gproxy_transform::stream_adapter::SseTransformer;
use gproxy_transform::{dispatch, resolve, TransformContext, TransformError, TransformPair};

use crate::CoreError;

/// 双向协议转换计划。
///
/// `source` 为调用方提交的 wire 格式，`target` 为渠道路由表选出的上游格式。
/// 请求走正向 pair，响应/SSE 走反向 pair（与 gproxy pipeline 的用法一致）。
/// source == target 时直通。
pub struct TransformPlan {
    source: OperationKey,
    target: OperationKey,
    forward: Option<TransformPair>,
    reverse: Option<TransformPair>,
}

impl TransformPlan {
    pub fn plan(source: OperationKey, target: OperationKey) -> Result<Self, CoreError> {
        if source == target {
            return Ok(Self {
                source,
                target,
                forward: None,
                reverse: None,
            });
        }
        let forward = resolve(source, target).map_err(to_err)?;
        let reverse = resolve(target, source).map_err(to_err)?;
        Ok(Self {
            source,
            target,
            forward: Some(forward),
            reverse: Some(reverse),
        })
    }

    pub fn is_passthrough(&self) -> bool {
        self.forward.is_none()
    }

    pub fn transform_request(&self, body: &[u8]) -> Result<Vec<u8>, CoreError> {
        match self.forward {
            None => Ok(body.to_vec()),
            Some(pair) => {
                let ctx = TransformContext::new(self.source, self.target);
                dispatch::request_bytes(pair, &ctx, body).map_err(to_err)
            }
        }
    }

    pub fn transform_response(&self, body: &[u8]) -> Result<Vec<u8>, CoreError> {
        match self.reverse {
            None => Ok(body.to_vec()),
            Some(pair) => {
                let ctx = TransformContext::new(self.target, self.source);
                dispatch::response_bytes(pair, &ctx, body).map_err(to_err)
            }
        }
    }

    /// 流式响应的字节级 SSE 转换器；直通时返回 None（原样透传）。
    pub fn sse_transformer(&self) -> Result<Option<SseTransformer>, CoreError> {
        match self.reverse {
            None => Ok(None),
            Some(pair) => {
                let ctx = TransformContext::new(self.target, self.source);
                SseTransformer::new(pair, ctx).map(Some).map_err(to_err)
            }
        }
    }
}

fn to_err(e: TransformError) -> CoreError {
    CoreError::Transform(format!("{e:?}"))
}
