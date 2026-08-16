//! 图执行:按拓扑层推进,同层节点并发。
//!
//! 并行分支不是特殊节点,而是图里没有依赖关系的自然结果;每层内的节点
//! 同时发起,层与层之间才等待(计划 §4.6 图泛化)。

use std::collections::BTreeMap;
use std::sync::Arc;

use futures_util::stream::StreamExt;
use sea_orm::DatabaseConnection;

use crate::assets::pipeline::{MediaKind, Node, NodeKind, PipelineV1};
use crate::assets::AssetStore;
use crate::context::{build_context, ContextSnapshot, GenerationDraft};
use crate::llm::ir::images::{GenerateImageRequest, ImageMode, ImageOptions};
use crate::llm::ir::ModelId;
use crate::services::llm::{LlmService, SemanticLlmOutput};
use crate::services::media::MediaService;
use crate::CoreError;

use super::{AssistantTurn, ChatRunRequest, PipelineEvent};

/// 端口上流动的值;类型与 [`crate::assets::pipeline::PortType`] 一一对应。
#[derive(Clone, Debug)]
pub enum PortValue {
    PromptBundle(Box<GenerationDraft>),
    Text(String),
    Json(serde_json::Value),
    GenerationResult(Box<AssistantTurn>),
    AssetRef(i32),
}

pub struct GraphContext {
    pub db: DatabaseConnection,
    pub llm: Arc<LlmService>,
    pub store: Arc<dyn AssetStore>,
    pub snapshot: ContextSnapshot,
    pub request: ChatRunRequest,
    pub run_id: i32,
    pub progress: tokio::sync::mpsc::UnboundedSender<PipelineEvent>,
}

/// 执行整张图,返回 `node.port` → 值。
pub async fn execute(
    ctx: &GraphContext,
    pipeline: &PipelineV1,
) -> Result<BTreeMap<String, PortValue>, CoreError> {
    let waves = pipeline.topological_order()?;
    let mut values: BTreeMap<String, PortValue> = BTreeMap::new();

    for wave in waves {
        let nodes: Vec<&Node> = wave
            .iter()
            .filter_map(|id| pipeline.nodes.iter().find(|node| &node.id == id))
            .collect();
        // Map 的子节点由 Map 自己驱动,不在主图里单独跑。
        let driven: Vec<&str> = pipeline
            .nodes
            .iter()
            .filter_map(|node| match &node.kind {
                NodeKind::Map { node, .. } => Some(node.as_str()),
                _ => None,
            })
            .collect();

        let pending = nodes
            .into_iter()
            .filter(|node| !driven.contains(&node.id.as_str()))
            .map(|node| run_node(ctx, pipeline, node, &values));
        for outcome in futures_util::future::join_all(pending).await {
            let (id, produced) = outcome?;
            for (port, value) in produced {
                values.insert(format!("{id}.{port}"), value);
            }
        }
    }
    Ok(values)
}

/// 节点产出:`(节点 id, [(端口名, 值)])`。
type NodeOutput = (String, Vec<(&'static str, PortValue)>);

#[tracing::instrument(skip(ctx, pipeline, values), fields(node = node.id))]
async fn run_node(
    ctx: &GraphContext,
    pipeline: &PipelineV1,
    node: &Node,
    values: &BTreeMap<String, PortValue>,
) -> Result<NodeOutput, CoreError> {
    let produced = match &node.kind {
        NodeKind::ContextBuild { .. } => vec![(
            "prompt",
            PortValue::PromptBundle(Box::new(build_context(&ctx.snapshot))),
        )],
        NodeKind::Generate { stream, .. } => {
            let draft = match incoming(pipeline, values, &node.id, "prompt") {
                Some(PortValue::PromptBundle(draft)) => *draft.clone(),
                _ => build_context(&ctx.snapshot),
            };
            let turn = generate(ctx, &draft, *stream).await?;
            vec![("result", PortValue::GenerationResult(Box::new(turn)))]
        }
        // 正则脚本的执行授权是本地状态,尚未接线;此处保持直通而不是假装变换。
        NodeKind::TextTransform { .. } => {
            let value = incoming(pipeline, values, &node.id, "input")
                .cloned()
                .ok_or_else(|| missing_input(&node.id, "input"))?;
            vec![("output", value)]
        }
        NodeKind::MediaGenerate { media, .. } => {
            let prompt = match incoming(pipeline, values, &node.id, "prompt") {
                Some(PortValue::Text(text)) => text.clone(),
                _ => ctx.request.user_message.clone().unwrap_or_default(),
            };
            let asset = media_generate(ctx, *media, &prompt).await?;
            vec![("asset", PortValue::AssetRef(asset))]
        }
        NodeKind::Map { slot, node: inner } => {
            let target = pipeline
                .nodes
                .iter()
                .find(|candidate| &candidate.id == inner)
                .ok_or_else(|| CoreError::InvalidPipeline {
                    reason: format!("map node {} targets unknown node {inner}", node.id),
                })?;
            let items = ctx
                .snapshot
                .world_books
                .len()
                .max(usize::from(!slot.is_empty()));
            let mut collected = Vec::with_capacity(items);
            for _ in 0..items {
                let (_, produced) = Box::pin(run_node(ctx, pipeline, target, values)).await?;
                collected.push(serde_json::to_value(describe(&produced))?);
            }
            vec![(
                "output",
                PortValue::Json(serde_json::Value::Array(collected)),
            )]
        }
    };
    Ok((node.id.clone(), produced))
}

/// 找到连到 `node.port` 的上游值。
fn incoming<'a>(
    pipeline: &PipelineV1,
    values: &'a BTreeMap<String, PortValue>,
    node_id: &str,
    port: &str,
) -> Option<&'a PortValue> {
    let target = format!("{node_id}.{port}");
    let edge = pipeline.edges.iter().find(|edge| edge.to == target)?;
    values.get(&edge.from)
}

async fn generate(
    ctx: &GraphContext,
    draft: &GenerationDraft,
    stream: bool,
) -> Result<AssistantTurn, CoreError> {
    let request = super::to_generate_request(draft, &ctx.request.model, stream);
    let output = ctx
        .llm
        .execute(
            &ctx.db,
            ctx.request.channel_id,
            crate::llm::ir::OperationRequest::Generate(request),
        )
        .await?;
    let mut turn = AssistantTurn::default();
    match output {
        SemanticLlmOutput::Stream(mut events) => {
            while let Some(item) = events.next().await {
                let event = item?;
                turn.observe(&event);
                // 发送失败只说明 UI 已经走了,生成仍要跑完并提交。
                let _ = ctx.progress.send(PipelineEvent::Progress { event });
            }
        }
        SemanticLlmOutput::Complete(crate::llm::ir::OperationResponse::Generate(response)) => {
            turn.output = response.output;
            turn.finish = response.finish;
            turn.usage = response.usage;
        }
        _ => {
            return Err(CoreError::UnsupportedRouteImplementation {
                implementation: "non-generation route on a generate node",
            })
        }
    }
    Ok(turn)
}

async fn media_generate(
    ctx: &GraphContext,
    media: MediaKind,
    prompt: &str,
) -> Result<i32, CoreError> {
    let MediaKind::Image = media else {
        // 视频是异步任务、语音需要音色配置,两者都不适合在图里同步产出。
        return Err(CoreError::UnsupportedRouteImplementation {
            implementation: "non-image media nodes",
        });
    };
    let result = MediaService::generate_image(
        &ctx.db,
        &ctx.llm,
        &ctx.store,
        ctx.request.channel_id,
        prompt,
        GenerateImageRequest {
            model: ModelId(ctx.request.model.0.clone()),
            prompt: prompt.to_owned(),
            count: Some(1),
            options: ImageOptions::default(),
            mode: ImageMode::Complete,
        },
    )
    .await?;
    result
        .assets
        .first()
        .map(|asset| asset.id)
        .ok_or_else(|| CoreError::InvalidExchangePayload {
            reason: "image route returned no inline artifact".to_owned(),
        })
}

fn describe(produced: &[(&'static str, PortValue)]) -> Vec<String> {
    produced
        .iter()
        .map(|(port, value)| match value {
            PortValue::Text(text) => format!("{port}:{text}"),
            PortValue::AssetRef(id) => format!("{port}:asset/{id}"),
            _ => (*port).to_owned(),
        })
        .collect()
}

fn missing_input(node_id: &str, port: &str) -> CoreError {
    CoreError::InvalidPipeline {
        reason: format!("node {node_id} has no value on input port {port}"),
    }
}
