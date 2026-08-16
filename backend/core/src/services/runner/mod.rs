//! 线性 runner:装载快照 → ContextBuild → Generate → 提交历史。
//!
//! 逐节点事件不持久化;流式 delta 只是 UI 的临时进度,当轮结束时把用户
//! 消息与助手输出一次性追加到 ChatHistory 并记录 run(计划 §5 / §7 / §9)。

mod graph;
mod snapshot;
mod template;

use futures_util::stream::Stream;
use sea_orm::{ConnectionTrait, DatabaseConnection};
use serde::{Deserialize, Serialize};

use crate::assets::chat_history::{ChatMessage, MESSAGES};
use crate::assets::preset::GenerationTrigger;
use crate::assets::PipelineDefinition;
use crate::context::{DraftMessage, GenerationDraft, TurnRole};
use crate::error::ApiError;
use crate::llm::codec::OperationEvent;
use crate::llm::ir::generation::{
    FinishReason, GenerateEvent, GenerateMode, GenerateRequest, GenerationLimits, InputContent,
    InputItem, Message, MessageRole, OutputConstraint, OutputItem, OutputModality, ToolChoice,
};
use crate::llm::ir::{ModelId, Usage};
use crate::services::assets::AssetService;
use crate::services::llm::LlmService;
use crate::services::runs::{ResolvedSlot, RunService, SlotBinding};
use crate::CoreError;

pub use graph::{GraphContext, PortValue};
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
    #[tracing::instrument(skip(db, llm, store, request), fields(
        pipeline = request.pipeline_asset_id,
        channel_id = request.channel_id,
        run_id = tracing::field::Empty,
    ))]
    pub async fn run_chat(
        db: DatabaseConnection,
        llm: std::sync::Arc<LlmService>,
        store: std::sync::Arc<dyn crate::assets::AssetStore>,
        request: ChatRunRequest,
    ) -> Result<impl Stream<Item = PipelineEvent>, CoreError> {
        let definition = template::load_pipeline(&db, request.pipeline_asset_id).await?;
        let inputs = RunService::resolve_slots(&db, &definition, &request.bindings).await?;
        let PipelineDefinition::V1(pipeline) = definition;
        let history_slot = inputs
            .iter()
            .find(|slot| slot.slot == HISTORY_SLOT)
            .cloned()
            .ok_or_else(|| CoreError::InvalidRunBinding {
                slot: HISTORY_SLOT.to_owned(),
                reason: "chat runs need a history slot".to_owned(),
            })?;
        let run = RunService::start(&db, request.pipeline_asset_id, &inputs).await?;
        tracing::Span::current().record("run_id", run.id);
        let snapshot = snapshot::load_snapshot(&db, &inputs, &request, run.id).await?;

        let (progress, mut incoming) = tokio::sync::mpsc::unbounded_channel();
        let context = graph::GraphContext {
            db,
            llm,
            store,
            snapshot,
            request,
            run_id: run.id,
            progress,
        };

        Ok(async_stream::stream! {
            yield PipelineEvent::Started { run_id: run.id };
            // 图执行与事件转发在同一个任务里交替推进,壳层无需额外 runtime。
            let mut work = std::pin::pin!(run_graph(&context, &pipeline, &history_slot));
            let terminal = loop {
                tokio::select! {
                    event = incoming.recv() => {
                        if let Some(event) = event {
                            yield event;
                        }
                    }
                    outcome = &mut work => break outcome,
                }
            };
            // 图已结束,把还排在通道里的进度事件放完再收口。
            while let Ok(event) = incoming.try_recv() {
                yield event;
            }
            match terminal {
                Ok(history) => yield PipelineEvent::Committed { run_id: run.id, history },
                Err(error) => {
                    let api_error = error.api_error();
                    let _ = RunService::fail(&context.db, run.id, &api_error).await;
                    yield PipelineEvent::Failed { run_id: Some(run.id), error: api_error };
                }
            }
        })
    }
}

/// 跑完整张图,并把终端生成结果作为当轮追加提交。
async fn run_graph(
    context: &graph::GraphContext,
    pipeline: &crate::assets::pipeline::PipelineV1,
    history_slot: &ResolvedSlot,
) -> Result<ResolvedSlot, CoreError> {
    let values = graph::execute(context, pipeline).await?;
    // 历史的输出声明指向哪个端口,就取哪个端口的生成结果。
    let turn = pipeline
        .outputs
        .iter()
        .find(|output| output.slot == HISTORY_SLOT)
        .and_then(|output| values.get(&output.from).cloned())
        .and_then(|value| match value {
            graph::PortValue::GenerationResult(turn) => Some(*turn),
            _ => None,
        })
        .ok_or_else(|| CoreError::InvalidPipeline {
            reason: "chat pipeline produced no generation result for the history slot".to_owned(),
        })?;
    commit_turn(
        &context.db,
        context.run_id,
        history_slot,
        context.request.user_message.clone(),
        context.request.model.clone(),
        turn,
    )
    .await
}

/// 从流式事件累积当轮结果;`OutputFinished` 已携带完整 item,
/// 不需要自行拼接 delta。
#[derive(Clone, Debug)]
pub struct AssistantTurn {
    pub output: Vec<OutputItem>,
    /// 流未给出结束原因时视作被截断,而不是假装正常收尾。
    pub finish: FinishReason,
    pub usage: Option<Usage>,
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
    pub(crate) fn observe(&mut self, event: &OperationEvent) {
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

pub(crate) fn to_generate_request(
    draft: &GenerationDraft,
    model: &ModelId,
    stream: bool,
) -> GenerateRequest {
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
        mode: if stream {
            GenerateMode::Stream
        } else {
            GenerateMode::Complete
        },
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
