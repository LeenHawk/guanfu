use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use chrono::Utc;
use futures_util::StreamExt;
use gproxy_protocol::Provider;
use gproxy_transform::stream_adapter::SseTransformer;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use crate::entities::{channel, credential};
use crate::llm::capability::{self, Capability};
use crate::llm::client::{ByteStream, CallTarget, LlmClient, LlmReply, LlmResponse};
use crate::llm::exchange::ExchangePlan;
use crate::llm::pool::{self, FailureKind};
use crate::CoreError;

pub struct LlmRequest {
    pub channel_id: i32,
    pub capability: Capability,
    pub model: String,
    pub stream: bool,
    /// canonical（OpenAI Chat Completions）线格式的 JSON 请求体；无体操作为 None。
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
        Self {
            client: LlmClient::new(),
            rotation: AtomicUsize::new(0),
        }
    }

    pub async fn execute(
        &self,
        db: &DatabaseConnection,
        req: LlmRequest,
    ) -> Result<LlmReply, CoreError> {
        let ch = channel::Entity::find_by_id(req.channel_id)
            .one(db)
            .await?
            .filter(|c| c.enabled)
            .ok_or(CoreError::ChannelNotFound(req.channel_id))?;
        let provider = capability::parse_provider(&ch.provider)
            .ok_or_else(|| CoreError::UnknownProvider(ch.provider.clone()))?;
        let target_key = capability::operation_key(req.capability, provider, req.stream)
            .ok_or(CoreError::UnsupportedCapability(req.capability))?;

        // 内容生成走 canonical → 渠道原生的转换；其余操作以渠道原生协议直连。
        let plan = if req.capability == Capability::GenerateContent {
            let source_key =
                capability::operation_key(req.capability, Provider::OpenAi, req.stream)
                    .expect("canonical generation key always exists");
            Some(ExchangePlan::plan(source_key, target_key)?)
        } else {
            None
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
        let ordered = pool::order_credentials(&creds, rotation, Utc::now());
        if ordered.is_empty() {
            return Err(CoreError::NoUsableCredential(ch.id));
        }

        let mut last_err = None;
        for cred in ordered {
            let target = CallTarget {
                base_url: &ch.base_url,
                provider,
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
fn finish(reply: LlmReply, plan: Option<ExchangePlan>) -> Result<LlmReply, CoreError> {
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

async fn mark_success(db: &DatabaseConnection, cred: &credential::Model) -> Result<(), CoreError> {
    let mut am: credential::ActiveModel = cred.clone().into();
    am.failure_count = Set(0);
    am.cooldown_until = Set(None);
    am.last_used_at = Set(Some(Utc::now()));
    am.update(db).await?;
    Ok(())
}

async fn mark_failure(
    db: &DatabaseConnection,
    cred: &credential::Model,
    kind: FailureKind,
) -> Result<(), CoreError> {
    let failures = cred.failure_count + 1;
    let mut am: credential::ActiveModel = cred.clone().into();
    am.failure_count = Set(failures);
    if let Some(d) = pool::cooldown_after(kind, failures) {
        am.cooldown_until = Set(Some(Utc::now() + d));
    }
    am.update(db).await?;
    Ok(())
}
