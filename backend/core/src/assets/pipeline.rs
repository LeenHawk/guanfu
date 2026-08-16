//! Pipeline definition:覆盖 ST 聊天与全部 agent run 的超集 workflow schema。
//!
//! 三条纪律(计划 §4.6):槽位是类型签名;写入是输出声明而非节点;端口值
//! 类型是有限集,edges 在加载期校验类型匹配,未知节点 kind 加载即报错。

use serde::{Deserialize, Serialize};

use super::refs::Extra;
use super::{join_inline, split_inline, AssetDefinition, ChunkContents, Manifest, SplitManifest};
use crate::entities::asset::AssetKind;
use crate::CoreError;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "version")]
pub enum PipelineDefinition {
    V1(PipelineV1),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct PipelineV1 {
    pub name: String,
    /// 槽位签名:run 发起时按 kind 校验绑定并钉住 revision。
    pub inputs: Vec<InputSlot>,
    /// 非 Asset 运行参数(user_message、trigger…)。
    pub params: Vec<ParamSpec>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// 对 Asset 的写入声明;run 结束原子提交。
    pub outputs: Vec<OutputDecl>,
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct InputSlot {
    pub name: String,
    pub kind: AssetKind,
    /// 允许绑定多个 Asset(如 world_books[])。
    pub many: bool,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct ParamSpec {
    pub name: String,
    pub ty: ParamType,
    pub required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    Text,
    Integer,
    Boolean,
    Json,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct Node {
    pub id: String,
    #[serde(flatten)]
    pub kind: NodeKind,
}

/// V1 节点:一次模型交互或一次纯变换;提示词片段不作为节点。
/// 图泛化阶段追加 MediaGenerate / AgentLoop / Parallel / Map,schema 不变。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeKind {
    /// 槽位 + trigger → PromptBundle,纯函数。
    ContextBuild {
        /// 参与组装的输入槽位名。
        slots: Vec<String>,
    },
    /// 终端节点:流式直通到 UI,累积 GenerationResult。
    Generate {
        /// 模型与渠道由 run 参数提供,不固定在可分享 pipeline 里。
        model_param: String,
        channel_param: String,
        stream: bool,
    },
    /// 正则变换。
    TextTransform { regex_slots: Vec<String> },
}

/// 端口值类型的有限集。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum PortType {
    PromptBundle,
    Text,
    Json,
    GenerationResult,
    AssetRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct Edge {
    /// `node.port`。
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
pub struct OutputDecl {
    /// 目标输入槽位名(写回该槽绑定的 Asset)。
    pub slot: String,
    pub op: OutputOp,
    /// `node.port`。
    pub from: String,
}

/// 输出操作:新建 / 追加 / 以 HashEdit 锚定的修订。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum OutputOp {
    Create,
    Append,
    HashEdit,
}

impl NodeKind {
    pub fn input_port(&self, port: &str) -> Option<PortType> {
        match (self, port) {
            (Self::Generate { .. }, "prompt") => Some(PortType::PromptBundle),
            (Self::TextTransform { .. }, "input") => Some(PortType::GenerationResult),
            _ => None,
        }
    }

    pub fn output_port(&self, port: &str) -> Option<PortType> {
        match (self, port) {
            (Self::ContextBuild { .. }, "prompt") => Some(PortType::PromptBundle),
            (Self::Generate { .. }, "result") => Some(PortType::GenerationResult),
            (Self::TextTransform { .. }, "output") => Some(PortType::GenerationResult),
            _ => None,
        }
    }
}

impl PipelineV1 {
    /// 加载期校验:节点 id 唯一、边端口存在且类型匹配、输出引用已声明的槽位。
    pub fn validate(&self) -> Result<(), CoreError> {
        let mut seen = std::collections::BTreeSet::new();
        for node in &self.nodes {
            if !seen.insert(node.id.as_str()) {
                return Err(invalid(format!("duplicate node id {}", node.id)));
            }
        }
        for edge in &self.edges {
            let from = self.port(&edge.from, PortSide::Output)?;
            let to = self.port(&edge.to, PortSide::Input)?;
            if from != to {
                return Err(invalid(format!(
                    "edge {} -> {} connects {from:?} to {to:?}",
                    edge.from, edge.to
                )));
            }
        }
        for output in &self.outputs {
            if !self.inputs.iter().any(|slot| slot.name == output.slot) {
                return Err(invalid(format!(
                    "output writes undeclared slot {}",
                    output.slot
                )));
            }
            self.port(&output.from, PortSide::Output)?;
        }
        Ok(())
    }

    fn port(&self, reference: &str, side: PortSide) -> Result<PortType, CoreError> {
        let (node_id, port) = reference
            .split_once('.')
            .ok_or_else(|| invalid(format!("port reference {reference} is not node.port")))?;
        let node = self
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| invalid(format!("unknown node {node_id}")))?;
        let resolved = match side {
            PortSide::Input => node.kind.input_port(port),
            PortSide::Output => node.kind.output_port(port),
        };
        resolved.ok_or_else(|| invalid(format!("node {node_id} has no port {port}")))
    }
}

#[derive(Clone, Copy)]
enum PortSide {
    Input,
    Output,
}

fn invalid(reason: String) -> CoreError {
    CoreError::InvalidPipeline { reason }
}

impl AssetDefinition for PipelineDefinition {
    const KIND: AssetKind = AssetKind::Pipeline;

    fn split(&self) -> Result<SplitManifest, CoreError> {
        let Self::V1(pipeline) = self;
        pipeline.validate()?;
        split_inline(self)
    }

    fn join(manifest: &Manifest, _: &ChunkContents) -> Result<Self, CoreError> {
        let definition: Self = join_inline(manifest)?;
        let Self::V1(pipeline) = &definition;
        pipeline.validate()?;
        Ok(definition)
    }
}
