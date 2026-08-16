//! 线性 runner:装载快照 → ContextBuild → Generate → 提交历史。
//!
//! 逐节点事件不持久化;流式 delta 只是 UI 的临时进度,当轮结束时把用户
//! 消息与助手输出一次性追加到 ChatHistory 并记录 run(计划 §5 / §7 / §9)。

mod snapshot;
mod template;

use futures_util::stream::{Stream, StreamExt};
use sea_orm::{ConnectionTrait, DatabaseConnection};
use serde::{Deserialize, Serialize};

use crate::assets::chat_history::{ChatMessage, MESSAGES};
use crate::assets::preset::GenerationTrigger;
use crate::context::{build_context, DraftMessage, GenerationDraft, TurnRole};
use crate::error::ApiError;
use crate::llm::codec::OperationEvent;
use crate::llm::ir::generation::{
    FinishReason, GenerateEvent, GenerateMode, GenerateRequest, GenerationLimits, InputContent,
    InputItem, Message, MessageRole, OutputConstraint, OutputItem, OutputModality, ToolChoice,
};
use crate::llm::ir::{ModelId, OperationRequest, Usage};
use crate::services::assets::AssetService;
use crate::services::llm::{LlmService, SemanticLlmOutput};
use crate::services::runs::{ResolvedSlot, RunService, SlotBinding};
use crate::CoreError;

pub use snapshot::load_snapshot;
pub use template::{chat_pipeline, ensure_chat_pipeline};

/// 发起一次聊天 run。
#[derive(Clone, Debug, Deserialize, Serialize, ts_rs::TS)]
pub struct ChatRunRequest {
    pub pipeline_asset_id: i32,
    pub bindings: Vec<SlotBinding>,
    pub channel_id: i32,
    pub model: ModelId,
    pub user_message: Option<String>,
    #[serde(default)]
    pub trigger: GenerationTrigger,
}

/// run 期间流向 UI 的事件;只有已建模的语义,不透传协议 JSON。
#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PipelineEvent {
    Started {
        run_id: i32,
    },
    /// 临时进度,不落库。
    Progress {
        event: OperationEvent,
    },
    /// 当轮已提交:历史推进到新 revision。
    Committed {
        run_id: i32,
        history: ResolvedSlot,
    },
    Failed {
        run_id: Option<i32>,
        error: ApiError,
    },
}

/// 历史槽位名;内置聊天模板与 runner 共用。
pub const HISTORY_SLOT: &str = "history";

pub struct RunnerService;

impl RunnerService {
    /// 执行一次聊天 run,返回事件流。
    ///
    /// 事件流被消费完毕时,当轮已经提交或已记为失败。
    pub async fn run_chat(
        db: DatabaseConnection,
        llm: std::sync::Arc<LlmService>,
        request: ChatRunRequest,
    ) -> Result<impl Stream<Item = PipelineEvent>, CoreError> {
        let pipeline = template::load_pipeline(&db, request.pipeline_asset_id).await?;
        let inputs = RunService::resolve_slots(&db, &pipeline, &request.bindings).await?;
        let history_slot = inputs
            .iter()
            .find(|slot| slot.slot == HISTORY_SLOT)
            .cloned()
            .ok_or_else(|| CoreError::InvalidRunBinding {
                slot: HISTORY_SLOT.to_owned(),
                reason: "chat runs need a history slot".to_owned(),
            })?;
        let run = RunService::start(&db, request.pipeline_asset_id, &inputs).await?;
        let snapshot = snapshot::load_snapshot(&db, &inputs, &request, run.id).await?;
        let draft = build_context(&snapshot);

        Ok(async_stream::stream! {
            yield PipelineEvent::Started { run_id: run.id };
            match execute(&db, &llm, &request, &draft, run.id, &history_slot).await {
                Ok(events) => {
                    let mut events = std::pin::pin!(events);
                    while let Some(event) = events.next().await {
                        yield event;
                    }
                }
                Err(error) => {
                    let api_error = error.api_error();
                    let _ = RunService::fail(&db, run.id, &api_error).await;
                    yield PipelineEvent::Failed {
                        run_id: Some(run.id),
                        error: api_error,
                    };
                }
            }
        })
    }
}

async fn execute(
    db: &DatabaseConnection,
    llm: &LlmService,
    request: &ChatRunRequest,
    draft: &GenerationDraft,
    run_id: i32,
    history_slot: &ResolvedSlot,
) -> Result<impl Stream<Item = PipelineEvent>, CoreError> {
    let generate = to_generate_request(draft, &request.model);
    let output = llm
        .execute(db, request.channel_id, OperationRequest::Generate(generate))
        .await?;

    let db = db.clone();
    let history_slot = history_slot.clone();
    let user_message = request.user_message.clone();
    let model = request.model.clone();

    Ok(async_stream::stream! {
        let mut turn = AssistantTurn::default();
        match output {
            SemanticLlmOutput::Stream(mut stream) => {
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(event) => {
                            turn.observe(&event);
                            yield PipelineEvent::Progress { event };
                        }
                        Err(error) => {
                            let api_error = error.api_error();
                            let _ = RunService::fail(&db, run_id, &api_error).await;
                            yield PipelineEvent::Failed { run_id: Some(run_id), error: api_error };
                            return;
                        }
                    }
                }
            }
            SemanticLlmOutput::Complete(response) => {
                if let crate::llm::ir::OperationResponse::Generate(response) = response {
                    turn.output = response.output;
                    turn.finish = response.finish;
                    turn.usage = response.usage;
                }
            }
            SemanticLlmOutput::Realtime(_) => {
                let error = CoreError::UnsupportedRouteImplementation {
                    implementation: "realtime inside a chat run",
                }
                .api_error();
                let _ = RunService::fail(&db, run_id, &error).await;
                yield PipelineEvent::Failed { run_id: Some(run_id), error };
                return;
            }
        }

        match commit_turn(&db, run_id, &history_slot, user_message, model, turn).await {
            Ok(history) => yield PipelineEvent::Committed { run_id, history },
            Err(error) => {
                let api_error = error.api_error();
                let _ = RunService::fail(&db, run_id, &api_error).await;
                yield PipelineEvent::Failed { run_id: Some(run_id), error: api_error };
            }
        }
    })
}

/// 从流式事件累积当轮结果;`OutputFinished` 已携带完整 item,
/// 不需要自行拼接 delta。
struct AssistantTurn {
    output: Vec<OutputItem>,
    /// 流未给出结束原因时视作被截断,而不是假装正常收尾。
    finish: FinishReason,
    usage: Option<Usage>,
}

impl Default for AssistantTurn {
    fn default() -> Self {
        Self {
            output: Vec::new(),
            finish: FinishReason::Incomplete,
            usage: None,
        }
    }
}

impl AssistantTurn {
    fn observe(&mut self, event: &OperationEvent) {
        let OperationEvent::Generate(event) = event else {
            return;
        };
        match event {
            GenerateEvent::OutputFinished(finished) => self.output.push(finished.item.clone()),
            GenerateEvent::Finished(finished) => {
                self.finish = finished.finish.clone();
                self.usage = finished.usage.clone();
            }
            GenerateEvent::UsageUpdated(usage) => self.usage = Some(usage.clone()),
            _ => {}
        }
    }
}

async fn commit_turn(
    db: &impl ConnectionTrait,
    run_id: i32,
    history_slot: &ResolvedSlot,
    user_message: Option<String>,
    model: ModelId,
    turn: AssistantTurn,
) -> Result<ResolvedSlot, CoreError> {
    let now = now_ms();
    let mut units = Vec::new();
    if let Some(text) = user_message {
        units.push(serde_json::to_value(ChatMessage::User {
            content: vec![InputContent::Text { text }],
            created_at_ms: now,
        })?);
    }
    units.push(serde_json::to_value(ChatMessage::Assistant {
        output: turn.output,
        model,
        finish: turn.finish,
        usage: turn.usage.clone(),
        created_at_ms: now,
    })?);

    let revision = AssetService::append_units(
        db,
        history_slot.asset_id,
        history_slot.revision,
        MESSAGES,
        &units,
        Some(run_id),
    )
    .await?;
    let committed = ResolvedSlot {
        slot: history_slot.slot.clone(),
        asset_id: history_slot.asset_id,
        revision,
    };
    let usage = turn.usage.as_ref().map(serde_json::to_value).transpose()?;
    RunService::succeed(db, run_id, std::slice::from_ref(&committed), usage).await?;
    Ok(committed)
}

fn to_generate_request(draft: &GenerationDraft, model: &ModelId) -> GenerateRequest {
    GenerateRequest {
        model: model.clone(),
        input: draft.messages.iter().map(to_input_item).collect(),
        instructions: draft.instructions.clone(),
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        output: OutputConstraint::Text,
        sampling: draft.sampling.clone(),
        reasoning: draft.reasoning.clone(),
        protocol_options: Vec::new(),
        limits: GenerationLimits {
            max_output_tokens: draft.max_output_tokens,
            max_tool_calls: None,
        },
        modalities: vec![OutputModality::Text],
        mode: GenerateMode::Stream,
    }
}

fn to_input_item(message: &DraftMessage) -> InputItem {
    InputItem::Message {
        message: Message {
            role: match message.role {
                TurnRole::User => MessageRole::User,
                TurnRole::Assistant => MessageRole::Assistant,
                TurnRole::System => MessageRole::System,
            },
            content: vec![InputContent::Text {
                text: message.text.clone(),
            }],
        },
    }
}

fn now_ms() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp_nanos() as i64 / 1_000_000
}
