use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use gproxy_protocol::OperationKey;
use gproxy_transform::stream_adapter::SseTransformer;
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;

use crate::entities::{channel, credential, routing_rule};
use crate::llm::client::{ByteStream, CallTarget, LlmClient, LlmReply, LlmResponse};
use crate::llm::pool::{self, FailureKind};
use crate::llm::routing::{self, RouteDecision};
use crate::llm::transform::TransformPlan;
use crate::CoreError;

pub struct LlmRequest {
    pub channel_id: i32,
    /// 调用方提供的请求体所使用的 operation 与 wire kind。
    pub operation: OperationKey,
    pub model: String,
    pub stream: bool,
    /// `operation` 对应 wire 格式的 JSON 请求体；无体操作为 None。
    pub body: Option<Vec<u8>>,
}

/// LLM 调用服务：加载渠道 → 凭证排序 → 协议转换 → failover 执行。
pub struct LlmService {
    client: LlmClient,
    rotation: AtomicUsize,
}

impl Default for LlmService {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmService {
    pub fn new() -> Self {
        Self::with_timeouts(Duration::from_secs(10), Duration::from_secs(120))
    }

    pub fn with_timeouts(connect_timeout: Duration, request_timeout: Duration) -> Self {
        Self {
            client: LlmClient::with_timeouts(connect_timeout, request_timeout),
            rotation: AtomicUsize::new(0),
        }
    }

    #[tracing::instrument(skip(self, db, req), fields(channel_id = req.channel_id))]
    pub async fn execute(
        &self,
        db: &impl ConnectionTrait,
        req: LlmRequest,
    ) -> Result<LlmReply, CoreError> {
        let ch = channel::Entity::find_by_id(req.channel_id)
            .one(db)
            .await?
            .filter(|c| c.enabled)
            .ok_or(CoreError::ChannelNotFound(req.channel_id))?;
        let rules = routing_rule::Entity::find()
            .filter(routing_rule::Column::ChannelId.eq(ch.id))
            .all(db)
            .await?;
        let source_key = req.operation;
        let (target_key, plan) = match routing::decide(&rules, source_key)? {
            RouteDecision::Passthrough => (source_key, None),
            RouteDecision::TransformTo(target) if target == source_key => (source_key, None),
            RouteDecision::TransformTo(target) => {
                if target.operation().is_content_generation()
                    != source_key.operation().is_content_generation()
                    || target.operation() != source_key.operation()
                {
                    return Err(CoreError::UnsupportedRouteImplementation {
                        implementation: "cross-operation transform_to",
                    });
                }
                (target, Some(TransformPlan::plan(source_key, target)?))
            }
            RouteDecision::Local => {
                return Err(CoreError::UnsupportedRouteImplementation {
                    implementation: "local",
                });
            }
            RouteDecision::Unsupported => {
                return Err(CoreError::UnsupportedRoute {
                    channel_id: ch.id,
                    operation: source_key,
                });
            }
        };
        let body = match (&plan, req.body) {
            (Some(p), Some(b)) => Some(p.transform_request(&b)?),
            (_, b) => b,
        };

        let creds = credential::Entity::find()
            .filter(credential::Column::ChannelId.eq(ch.id))
            .all(db)
            .await?;
        let rotation = self.rotation.fetch_add(1, Ordering::Relaxed);
        let ordered = pool::order_credentials(&creds, rotation, OffsetDateTime::now_utc());
        if ordered.is_empty() {
            return Err(CoreError::NoUsableCredential(ch.id));
        }

        let mut last_err = None;
        for cred in ordered {
            let target = CallTarget {
                base_url: &ch.base_url,
                secret: &cred.secret,
            };
            let reply = match self
                .client
                .execute(&target, target_key, &req.model, req.stream, body.clone())
                .await
            {
                Ok(reply) => reply,
                Err(e) => {
                    // 网络层错误：换下一个凭证。
                    last_err = Some(e);
                    continue;
                }
            };
            match pool::classify_status(reply.status()) {
                None => {
                    mark_success(db, cred).await?;
                    return finish(reply, plan);
                }
                Some(FailureKind::Fatal) => return Err(upstream_error(reply)),
                Some(kind) => {
                    mark_failure(db, cred, kind).await?;
                    last_err = Some(upstream_error(reply));
                }
            }
        }
        Err(last_err.unwrap_or(CoreError::NoUsableCredential(ch.id)))
    }
}

/// 成功后的响应/流转换（直通时原样返回）。
fn finish(reply: LlmReply, plan: Option<TransformPlan>) -> Result<LlmReply, CoreError> {
    let Some(plan) = plan else { return Ok(reply) };
    match reply {
        LlmReply::Complete(r) => {
            let body = plan.transform_response(&r.body)?;
            Ok(LlmReply::Complete(LlmResponse {
                status: r.status,
                body: Bytes::from(body),
            }))
        }
        LlmReply::Stream { status, stream } => match plan.sse_transformer()? {
            None => Ok(LlmReply::Stream { status, stream }),
            Some(t) => Ok(LlmReply::Stream {
                status,
                stream: transform_stream(stream, t),
            }),
        },
    }
}

fn transform_stream(inner: ByteStream, transformer: SseTransformer) -> ByteStream {
    let seed = (inner, transformer, false);
    Box::pin(futures_util::stream::unfold(
        seed,
        |(mut inner, mut t, done)| async move {
            if done {
                return None;
            }
            loop {
                match inner.next().await {
                    Some(Ok(chunk)) => match t.push(&chunk) {
                        Ok(out) if out.is_empty() => continue,
                        Ok(out) => return Some((Ok(Bytes::from(out)), (inner, t, false))),
                        Err(e) => {
                            let e = CoreError::Transform(format!("{e:?}"));
                            return Some((Err(e), (inner, t, true)));
                        }
                    },
                    Some(Err(e)) => return Some((Err(e), (inner, t, true))),
                    None => {
                        return match t.finish() {
                            Ok(out) if out.is_empty() => None,
                            Ok(out) => Some((Ok(Bytes::from(out)), (inner, t, true))),
                            Err(e) => {
                                let e = CoreError::Transform(format!("{e:?}"));
                                Some((Err(e), (inner, t, true)))
                            }
                        };
                    }
                }
            }
        },
    ))
}

fn upstream_error(reply: LlmReply) -> CoreError {
    match reply {
        LlmReply::Complete(r) => CoreError::Upstream {
            status: r.status,
            body: String::from_utf8_lossy(&r.body).into_owned(),
        },
        LlmReply::Stream { status, .. } => CoreError::Upstream {
            status,
            body: String::new(),
        },
    }
}

async fn mark_success(
    db: &impl ConnectionTrait,
    cred: &credential::Model,
) -> Result<(), CoreError> {
    let mut am: credential::ActiveModel = cred.clone().into();
    am.failure_count = Set(0);
    am.cooldown_until = Set(None);
    am.last_used_at = Set(Some(OffsetDateTime::now_utc()));
    am.update(db).await?;
    Ok(())
}

async fn mark_failure(
    db: &impl ConnectionTrait,
    cred: &credential::Model,
    kind: FailureKind,
) -> Result<(), CoreError> {
    // 多实例：计数用 SQL 原子自增，避免 read-modify-write 竞态；
    // 退避时长按本地估算的次数计算，竞态下略有偏差可接受（best-effort）。
    let mut update = credential::Entity::update_many()
        .col_expr(
            credential::Column::FailureCount,
            Expr::col(credential::Column::FailureCount).add(1),
        )
        .filter(credential::Column::Id.eq(cred.id));
    if let Some(d) = pool::cooldown_after(kind, cred.failure_count + 1) {
        update = update.col_expr(
            credential::Column::CooldownUntil,
            Expr::value(Some(OffsetDateTime::now_utc() + d)),
        );
    }
    update.exec(db).await?;
    Ok(())
}
