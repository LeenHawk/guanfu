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

/// 节点:一次模型交互、一次纯变换,或一个结构节点。
///
/// 并行分支不需要专门的节点种类——图里没有依赖关系的节点本来就并发执行。
/// AgentLoop 要等 Asset 操作挂成工具(计划 §5.3)才有意义,先不建模。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeKind {
    /// 槽位 + trigger → PromptBundle,纯函数。
    ContextBuild {
        /// 参与组装的输入槽位名。
        slots: Vec<String>,
    },
    /// 流式直通到 UI,累积 GenerationResult。
    Generate {
        /// 模型与渠道由 run 参数提供,不固定在可分享 pipeline 里。
        model_param: String,
        channel_param: String,
        stream: bool,
    },
    /// 正则变换。
    TextTransform { regex_slots: Vec<String> },
    /// 按提示词生成媒体并落成 Media Asset,输出 AssetRef。
    MediaGenerate {
        model_param: String,
        channel_param: String,
        media: MediaKind,
    },
    /// 对一个 many 槽位逐项套用子节点,输出 Json 数组。
    Map {
        /// 被遍历的输入槽位名。
        slot: String,
        /// 对每一项执行的节点 id。
        node: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Speech,
    Video,
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
            (Self::MediaGenerate { .. }, "prompt") => Some(PortType::Text),
            (Self::Map { .. }, "input") => Some(PortType::Json),
            _ => None,
        }
    }

    pub fn output_port(&self, port: &str) -> Option<PortType> {
        match (self, port) {
            (Self::ContextBuild { .. }, "prompt") => Some(PortType::PromptBundle),
            (Self::Generate { .. }, "result") => Some(PortType::GenerationResult),
            (Self::TextTransform { .. }, "output") => Some(PortType::GenerationResult),
            (Self::MediaGenerate { .. }, "asset") => Some(PortType::AssetRef),
            (Self::Map { .. }, "output") => Some(PortType::Json),
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
        self.topological_order()?;
        Ok(())
    }

    /// 拓扑序;有环即报错。无依赖关系的节点在同一层,由 runner 并发执行。
    pub fn topological_order(&self) -> Result<Vec<Vec<String>>, CoreError> {
        use std::collections::{BTreeMap, BTreeSet};

        let mut blocked_by: BTreeMap<&str, BTreeSet<&str>> = self
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), BTreeSet::new()))
            .collect();
        for edge in &self.edges {
            let (from, _) = split_reference(&edge.from);
            let (to, _) = split_reference(&edge.to);
            if let Some(dependencies) = blocked_by.get_mut(to) {
                dependencies.insert(from);
            }
        }

        let mut waves = Vec::new();
        while !blocked_by.is_empty() {
            let ready: Vec<String> = blocked_by
                .iter()
                .filter(|(_, dependencies)| dependencies.is_empty())
                .map(|(id, _)| (*id).to_owned())
                .collect();
            if ready.is_empty() {
                return Err(invalid("pipeline graph has a cycle".to_owned()));
            }
            for id in &ready {
                blocked_by.remove(id.as_str());
            }
            for dependencies in blocked_by.values_mut() {
                dependencies.retain(|dependency| !ready.iter().any(|id| id == dependency));
            }
            waves.push(ready);
        }
        Ok(waves)
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

/// `node.port` 的机械拆分;校验在 [`PipelineV1::port`]。
fn split_reference(reference: &str) -> (&str, &str) {
    reference.split_once('.').unwrap_or((reference, ""))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: NodeKind) -> Node {
        Node {
            id: id.to_owned(),
            kind,
        }
    }

    fn edge(from: &str, to: &str) -> Edge {
        Edge {
            from: from.to_owned(),
            to: to.to_owned(),
        }
    }

    fn generate(id: &str) -> Node {
        node(
            id,
            NodeKind::Generate {
                model_param: "model".to_owned(),
                channel_param: "channel".to_owned(),
                stream: true,
            },
        )
    }

    /// 图的两条纪律:无依赖的节点同层(并行分支),有环即加载失败。
    #[test]
    fn independent_nodes_share_a_wave_and_cycles_are_rejected() {
        let mut pipeline = PipelineV1 {
            nodes: vec![
                node("ctx", NodeKind::ContextBuild { slots: Vec::new() }),
                generate("a"),
                generate("b"),
            ],
            edges: vec![
                edge("ctx.prompt", "a.prompt"),
                edge("ctx.prompt", "b.prompt"),
            ],
            ..Default::default()
        };
        let waves = pipeline.topological_order().unwrap();
        assert_eq!(
            waves,
            vec![vec!["ctx".to_owned()], vec!["a".to_owned(), "b".to_owned()]]
        );
        pipeline.validate().unwrap();

        // 类型不匹配的边在加载期就拒绝。
        pipeline.edges.push(edge("a.result", "b.prompt"));
        assert!(matches!(
            pipeline.validate(),
            Err(CoreError::InvalidPipeline { .. })
        ));

        let cyclic = PipelineV1 {
            nodes: vec![
                node(
                    "x",
                    NodeKind::TextTransform {
                        regex_slots: Vec::new(),
                    },
                ),
                node(
                    "y",
                    NodeKind::TextTransform {
                        regex_slots: Vec::new(),
                    },
                ),
            ],
            edges: vec![edge("x.output", "y.input"), edge("y.output", "x.input")],
            ..Default::default()
        };
        assert!(matches!(
            cyclic.topological_order(),
            Err(CoreError::InvalidPipeline { .. })
        ));
    }
}
