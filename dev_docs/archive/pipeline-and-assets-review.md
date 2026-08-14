# Pipeline、Assets 与 IR 设计评审（已归档）

本文对照以下内容整理：

- `pipeline-and-assets.md` 中的 Pipeline/Assets 草案
- `sillytavern-resource-formats.md` 中的 SillyTavern 调查结论
- 当前 `backend/core/src/llm` 下的 IR 与 codec 实现

结论：整体方案主体方向正确，尤其是 `ContextBuild` 的节点粒度、版本化图定义和
分阶段实现方式。协议专用 JSON overlay 可以作为按目标 kind 条件生效的可选参数
层；此外，流式执行语义、pipeline 的运行时资源绑定和 asset 关系模型需要在编码前
修订。

## 1. 对现有 IR 问题的核对

| 判断 | 结论 |
| --- | --- |
| `top_k` / `seed` / `stop` 是死字段 | 确认。当前只要启用就无条件返回 `UnsupportedCapability`，不检查目标协议 |
| 缺少 `frequency_penalty` / `presence_penalty` | 确认。ST OpenAI 预设需要，当前 `SamplingOptions` 无法表达 |
| 缺少 reasoning 请求控制 | 确认。当前只能携带历史 reasoning 和接收 reasoning 输出，不能请求 effort、budget 或 summary |
| 没有 provider 逃生口 | 确认。需要增加按目标协议条件生效的可选参数层 |
| 历史中不能插入 system message | 确认。当前 `MessageRole` 只有 user 和 assistant |
| 每个工具单独 clone 并 transform | 确认，是不必要的 O(tool count) 转换 |
| transform diagnostics 全部报错过于严格 | 部分接受。实现确实如此，但不能直接改成忽略 semantic loss |

当前采样参数应按目标协议分别处理：

| 参数 | OpenAI Responses | OpenAI Chat | Claude Messages | Gemini GenerateContent |
| --- | --- | --- | --- | --- |
| `temperature` / `top_p` | 支持 | 支持 | 支持 | 支持 |
| `top_k` | 不支持 | 不支持 | 支持 | 支持 |
| `seed` | 不支持 | 支持 | 不支持 | 支持 |
| `stop` | 不支持 | 支持 | 支持 | 支持 |
| frequency/presence penalty | 不支持 | 支持 | 不支持 | 支持 |

因此，消息、工具和内容等结构转换仍交给 gproxy-transform；采样参数在结构转换后，
由目标协议 codec 显式写入。这可以解除 canonical request 的表达上限。

采样参数属于调优提示：目标协议支持就写入，不支持就丢弃，不应因为 preset 中存在
`top_k` 而让 OpenAI Chat route 整体失败。工具、结构化输出、输入内容等会改变请求
核心语义的能力仍然采用严格校验，不能与 best-effort sampling 混为一类。

## 2. 协议专用 JSON merge patch

接受在生成请求中增加按目标协议 kind 条件生效的可选 overlay：

```rust
pub struct ProtocolOptions {
    pub kind: ContentGenerationKind,
    pub patch: JsonMergePatch,
}

pub struct GenerateRequest {
    // 公共语义字段……
    pub protocol_options: Vec<ProtocolOptions>,
}
```

它的语义不是“所有 route 都必须兑现的公共能力”，而是“如果最终选中的目标协议
与 kind 匹配，就应用这份专用参数；否则丢弃”。因此：

- target kind 匹配：在 gproxy-transform 之后应用 JSON merge patch。
- target kind 不匹配：默认忽略，不报错，也不阻止 route fallback。
- 同一个 preset 可以同时携带 OpenAI Chat、Claude 和 Gemini 三组 options。
- 同一份 preset 切换 channel 或 route 时，不会因为其他协议的专用参数而失效。

这里的“默认丢弃”是保证 preset 通用性的必要语义。协议专用参数本来就是
best-effort 条件配置，不能被当成跨协议强约束。

仍需保留一个边界：overlay 是“协议专用参数”，不是完整 wire request 替换器。
codec 应保护由公共 IR 负责的结构字段，例如：

- `model`
- `input` / `messages` / `contents`
- `instructions` / `system` / `systemInstruction`
- `tools` / `tool_choice`
- `stream`

patch 中这些受保护字段不参与合并。其他目标协议字段，包括嵌套的
`generationConfig`、`safetySettings`、`logit_bias`、`logprobs`、`speed`、
`service_tier` 和后续新增采样器，可以保留并按目标 kind 应用。

建议把数据分为四层。

### 2.1 交换格式保真数据

例如：

- ST preset 的 `extra`
- CCv2 的未知 `extensions`
- world book 的未知交换字段

这些字段只参与导入和导出，不自动进入模型请求。

### 2.2 可移植语义字段

进入公共 typed IR：

- sampling
- reasoning
- system message
- tools
- output constraint

### 2.3 协议专用运行参数

协议专用运行参数使用带明确 `ContentGenerationKind` 的 JSON merge patch。JSON 在
这里表达的就是目标 wire 协议本身的动态扩展字段，因此属于合理的动态数据边界，
但它不能离开 codec 层直接传给 Tauri/Axum 或 UI。

例如：

- OpenAI Chat 的 `logit_bias`
- Gemini 的 `safetySettings`
- Claude 的 `speed`、OpenAI/Gemini 的 `service_tier`
- 其他协议专用推理或服务参数
- 暂时不值得提升为公共语义、但目标协议可以直接接收的字段

ST 中暂未建模的 textgen sampler 可以保真存储；只有项目拥有对应 protocol kind 和
codec 时才能成为可执行 overlay。没有对应目标协议时保留但忽略，不影响 preset 在
其他 channel 上使用。

### 2.4 Channel 与路由配置

以下内容继续由 channel、credential 和 routing rule 负责：

- base URL
- 凭证和鉴权
- 固定 headers
- route/codec 的环境配置

普通可分享 preset 和 pipeline 不保存这些连接信息。

### 2.5 修订后的编码过程

```text
语义 GenerateRequest
  → canonical 结构编码
  → gproxy-transform 转换消息、工具和内容结构
  → 写入该协议支持的可移植采样参数
  → 选择与 target kind 匹配的 ProtocolOptions
  → 过滤受保护的核心结构字段
  → 应用 JSON merge patch
  → wire JSON
```

如果 overlay 和公共 sampling 写入同一个允许覆盖的参数，匹配目标协议的 overlay
优先。这样通用 preset 可以提供默认 sampling，同时针对某一种协议做精细调优。

## 3. Transform diagnostics 的处理

不能简单忽略 semantic-loss diagnostics。请求和响应应分别处理。

### 请求方向

- 整个请求只 transform 一次。
- 不再为每个工具 clone 请求并单独探测。
- diagnostics 转换成结构化的“当前 route 不兼容此请求”。
- `LlmService` 可以继续尝试 channel 路由表中的下一条 route。
- 未命中的 `ProtocolOptions` 和目标协议不支持的 best-effort sampling 不产生
  diagnostics，也不阻止当前 route。

### 响应方向

- 响应 transform 出现 semantic loss 时仍然报错。
- 此时上游请求已经执行，不能把缺失语义的结果伪装成完整 IR。
- 未建模 provider 事件继续显式报错，不增加原生协议事件透传。

## 4. ReasoningOptions

建议新增公共 reasoning 请求配置，至少表达：

```rust
pub struct ReasoningOptions {
    pub effort: Option<ReasoningEffort>,
    pub budget_tokens: Option<u64>,
    pub output: Option<ReasoningOutputPolicy>,
}
```

其中 `ReasoningOutputPolicy` 应明确区分不返回、返回摘要和返回完整 reasoning。
不同协议不能完整映射时，由 codec 返回 route 不兼容，而不是静默降级。

`effort` 和 `budget_tokens` 可以同时保留为独立字段；目标协议是否允许组合由对应
codec 校验，不需要现在人为收窄整个 IR。

## 5. 历史中的 system message

为 `MessageRole` 增加 `System`：

```rust
pub enum MessageRole {
    System,
    User,
    Assistant,
}
```

开头的全局 system/developer 指令仍可放在 `instructions`。历史中间由世界书、
角色 depth prompt、persona 或作者注产生的 system 消息放在 `input` 中，保持其
相对位置。

## 6. Pipeline 的节点粒度

接受以下裁决：

- 节点表示一次模型交互或一次完整纯变换。
- 提示词片段不是节点。
- ST 的 prompt order、世界书预算和递归、深度注入、宏展开收敛到一个
  `ContextBuild` 节点。
- 图用于表达会话流：检索、生成、评审、重写、角色接力和 agent 工具循环。

推荐的默认 ST pipeline 仍然是：

```text
ContextBuild → Generate → TextTransform
```

但需要修订每个节点的输入输出定义。

### 6.1 ContextBuild 的纯函数边界

数据库查询和资源解析放在纯函数之外：

```text
runner 加载角色、persona、历史、世界书和 preset
  → ContextSnapshot
  → build_context(snapshot)
  → GenerationDraft
```

`ContextBuild` 负责确定性的：

- prompt order
- 宏展开
- 世界书激活和预算
- 示例对话裁剪
- 深度注入
- 历史裁剪

若以后需要调用模型压缩历史，`Compact` 应是独立模型交互节点，不能藏在纯
`ContextBuild` 中。

原草案中的 `PromptBundle` 同时包含 instructions、input 和 sampling，已经不只是
提示词。建议改名为 `GenerationDraft`。

### 6.2 Generate 与 TextTransform

`Generate` 输出完整 `GenerationResult`。`TextTransform` 也应输入并输出
`GenerationResult`，只修改选中的文本内容，保留 reasoning、tool call、usage 和
其他输出。

不要让 `TextTransform` 接收 `GenerationResult` 后只返回裸 `Text`，否则默认
pipeline 会丢失非文本语义。

### 6.3 流式执行语义

原草案的“只有终端 Generate 节点把事件流直通 UI”与默认 pipeline 矛盾，因为
Generate 后面还有 TextTransform，而且任意正则不能安全地逐 token 转换。

建议：

1. Generate 的 `OperationEvent` 作为运行进度流向 UI。
2. runner 同时累积完整 `GenerationResult`。
3. Generate 完成后执行 AI_OUTPUT `TextTransform`。
4. pipeline 发出最终 committed output，UI 用它替换临时流式文本。

因此需要 pipeline 级事件，但仍然只包含已建模语义：

```rust
pub enum PipelineEvent {
    Operation {
        node_id: NodeId,
        event: OperationEvent,
    },
    OutputCommitted {
        node_id: NodeId,
        output: PipelineOutput,
    },
}
```

不在 `PipelineEvent` 中加入原生 SSE 或任意协议 JSON。

## 7. Pipeline definition 与运行时绑定

接受单实体 + 版本化 typed JSON definition：

```rust
#[serde(tag = "version", content = "definition")]
pub enum PipelineDefinition {
    V1(PipelineDefinitionV1),
}
```

数据库不拆 node/edge 三张表。节点不参与独立关系查询，图总是整体加载、编辑和
导入导出。

### 7.1 不在默认 pipeline 中硬编码环境资源

默认 pipeline 不应固定：

- `channel_id`
- model
- 用户当前选择的 preset
- conversation

这些应由运行参数提供：

```rust
pub struct PipelineRunInput {
    pub conversation_id: ConversationId,
    pub trigger: GenerationTrigger,
    pub channel_id: ChannelId,
    pub model: ModelId,
    pub preset_id: PresetId,
}
```

否则 pipeline 无法分享或导入，也会与“preset 不保存连接配置”的裁决冲突。

后续自定义 pipeline 如需固定某个资源，可以再增加 typed `ResourceBinding`，5b
不需要提前实现。

### 7.2 图内循环

“不做图内环”应是 V1 runner 约束，而不是 pipeline 永久能力上限。

5c 可以先实现有界 `AgentLoop` 复合节点。它内部管理模型调用与工具执行循环，
对外仍是单节点。definition 的版本设计应允许以后将其升级为带 typed body
subgraph 的结构化循环。

不建议直接允许任意 edge 回环；任意环会立即引入终止条件、恢复、重试、并发和
状态持久化问题。

## 8. Preset 数据模型

接受 preset 作为独立可分享实体，v1 只支持 `openai_chat` kind。

建议数据库列保持精简：

```text
preset: id, name, kind, definition(JSON), created_at, updated_at
```

`definition` 使用 typed serde 结构，包含：

- 有序 prompts
- prompt-order profiles
- sampling
- reasoning
- 提示词辅助字段
- 导入导出使用的 extra

不必把 prompts、prompt_order、sampling 分成多个独立 JSON 列；它们总是整体加载，
单个 definition 更容易版本迁移和导入导出。

连接、模型选择、proxy 和 credential 字段不进入可分享 preset。

## 9. Assets 关系模型

接受以下部分：

- asset 保存 blob 元数据，数据库不保存大文件内容。
- 使用 sha256 内容寻址和去重。
- `AssetStore` 由 AppState 注入。
- MIME 使用字符串。
- GC 是显式维护操作，不自动删除。

不建议使用：

```text
asset_link(owner_kind, owner_id)
```

这种多态外键无法建立真实数据库 FK，会失去 SeaORM 类型关系、级联删除和引用
完整性，并容易产生悬空 owner。

v1 使用显式关系表：

```text
asset
character_asset
persona_asset
message_asset
```

当前明确不做 CharX，因此没有需求时不提前增加 `world_book_asset`。

各关系表可以使用各自的 typed kind：

- character：avatar / expression / background / gallery
- persona：avatar
- message：attachment / generated_image / audio

这样不需要一个持续膨胀的全局 `owner_kind` 字符串。

### 9.1 AssetStore 与多实例

本地目录实现只适用于：

- Tauri 桌面端
- 明确的单实例 Axum 部署

多个实例共享数据库时必须同时使用共享目录或对象存储，否则数据库中的
`storage_key` 可能只在其中一台机器上存在。

## 10. CCv2 范围

第一版支持：

- CCv2 JSON
- PNG `chara` chunk
- 以 `data.*` 为准
- alternate greetings 保持顺序
- embedded character book 转换为独立 world book 和绑定
- 未知 extensions 保真

第一版不支持：

- CCv3
- CharX
- BYAF
- text completion/instruct preset

如果 PNG 同时带有 `chara` 和 `ccv3`，第一版只读取 `chara`，不能把 ccv3 内容
误报为完整 CCv2 支持。

## 11. 修订后的实施顺序

### 5a：实体与交换格式

- character + 有序 greetings
- persona
- conversation/message
- world_book/world_book_entry/world_book_binding
- preset/regex_script
- asset 与显式 asset 关系
- pipeline 实体和版本化 definition DTO
- CCv2 JSON/PNG `chara` 导入导出
- 使用少量 ST fixture 验收标准字段、顺序和未知扩展保真

原草案的 5a 清单遗漏了 `conversation`、`message` 和 `pipeline`，但 5b 的聊天与
runner 都依赖它们，需要补回。

### 5b-0：IR 前置修复

- `frequency_penalty` / `presence_penalty`
- `ReasoningOptions`
- 历史 system message
- 解封并按目标 kind 编码 sampling
- 按 `ContentGenerationKind` 匹配的 `ProtocolOptions` JSON merge patch
- protocol options 未命中时默认丢弃
- codec 保护公共 IR 所有权范围内的核心 wire 字段
- 单次 transform
- diagnostics 转换为结构化 route incompatibility

### 5b：最小聊天通路

- ContextSnapshot loader
- 纯 `ContextBuild`
- 只支持线性链的 runner
- pipeline-level 流式进度和最终 committed output
- 内置默认 pipeline
- 聊天消息持久化
- 聊天 UI 打通

5b 阶段 pipeline 对用户不可见，用户只需要选择角色、preset、channel 和 model。

### 5c：图泛化

- typed ports 和 DAG 校验
- 并行分支
- 有界 AgentLoop
- pipeline 编辑 UI
- 后续再考虑条件节点、复合子图和结构化循环

## 12. 最终裁决

直接接受：

- `ContextBuild` 是 ST 上下文组装的正确节点粒度。
- pipeline 单实体 + 版本化 typed JSON。
- preset 独立复用。
- v1 使用线性 runner，图编辑 UI 后置。
- IR 增加 penalties、reasoning 和历史 system message。
- 采样参数在 transform 后按目标协议写入。
- 协议专用 JSON merge patch 按目标 kind 条件生效，未命中时默认丢弃。

修改后接受：

- V1 不做任意图内环，但保留结构化 AgentLoop/子图的升级空间。
- diagnostics 请求侧用于 route fallback，响应侧仍严格报错。
- Generate 可以流式报告进度，但 downstream transform 决定最终 committed output。
- provider overlay 只能修改协议参数，不能取代公共 IR 管理的请求结构。

拒绝：

- `asset_link(owner_kind, owner_id)` 多态外键。
- pipeline 硬编码环境相关 channel/model/preset 选择。
- 把未识别 preset `extra` 自动当成运行时请求参数。
