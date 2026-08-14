# Pipeline 与 Assets 机制设计（已归档草案）

依据 `sillytavern-resource-formats.md` 的调查结论。目标：

- 表达 SillyTavern 的生成机制，目标 wire 格式以 OpenAI Chat 为主
- 角色卡只支持 CCv2
- pipeline 不限于单次请求，泛化为图，为 agent 工作流留路
- 控制范围：图引擎按数据模型先行、执行能力渐进的方式切分

## 1. 核心裁决：图的粒度

节点 = 一次模型交互，或一次纯数据变换。**不把提示词片段做成节点。**

理由：ST 的上下文组装（prompt_order 编排、世界书扫描/预算/递归、深度注入、
宏展开）是一组全局纠缠的语义——token 预算在所有激活 entry 间分配，递归扫描
以整个已组装文本为输入，深度注入要在最终消息序列上按深度定位。拆成图节点
只会得到假模块化：节点间需要共享预算器和扫描状态，边上传的不再是值而是
半成品上下文。因此整个组装过程收敛为一个 `ContextBuild` 节点，内部是纯函数，
用 ST fixture 可单测。

图的价值在会话流层面：生成 → 评审 → 重写、检索后生成、旁路总结/压缩、
多角色接力。这些才是节点。

## 2. ST 一次生成的图表达

```text
[ContextBuild] ──PromptBundle──> [Generate] ──GenerationResult──> [TextTransform(regex)]
```

- `ContextBuild`：输入为运行时快照（角色卡、persona、世界书、历史、预设编排、
  触发类型），输出 `PromptBundle`（IR 的 instructions + input + 采样合并结果）。
  宏展开、世界书激活、深度注入、示例对话裁剪都在这里。
- `Generate`：持 channel_id + model + 采样/预设引用，走现有
  `LlmService::execute` 语义通路。
- `TextTransform`：正则脚本（AI_OUTPUT placement）。USER_INPUT placement 在
  runner 入口处理，不进图。

触发类型（normal / continue / regenerate / impersonate / swipe）是 runner 的
输入参数，不是图结构；对应 ST 的 `triggers` 语义。

## 3. 数据模型

### pipeline

单实体 + 版本化 JSON definition，不做 node/edge 三张表：

- 图总是整体加载、整体执行、整体编辑，节点不参与任何关系查询
- 导入导出、复制、版本迁移都是文档操作
- definition 用 typed serde enum 建模（非裸 JSON），ts-rs 导出给前端

```text
pipeline: id, name, description, definition(JSON, 带 schema version), created/updated
```

definition 内：`nodes: [{ id, kind, config }]`、`edges: [{ from, from_port, to, to_port }]`。
config 中以 ID 引用 preset / channel 等实体，加载时校验、运行时解析。

### preset

独立实体（可分享、可在 pipeline 间复用），v1 只做 `openai_chat` kind：

```text
preset: id, name, kind, prompts(JSON 有序数组), prompt_order(JSON),
        sampling(结构化已知字段), extra(JSON 保真桶), created/updated
```

- `prompts` / `prompt_order` 按 ST PromptManager 语义结构化（identifier、role、
  content、marker、injection_position/depth/order、forbid_overrides）
- 已知采样字段进结构化列/JSON；连接与 proxy 字段按调查结论丢弃（channel 负责）
- 未识别字段进 `extra`，导出时由 adapter 重建

### 执行产物

v1 不建 `pipeline_run` 持久化，聊天只需要落 message；观测走 tracing。
后续 agent 工作流需要审计时再加。

## 4. 执行语义

- DAG，拓扑序执行；边上传类型化值：`PromptBundle` / `Text` / `Json` /
  `GenerationResult` / `AssetRef`
- 只有终端 `Generate` 节点把 `OperationEvent` 流直通 UI；中间节点等待完整结果
- 取消：runner 持 CancellationToken，传播到进行中的上游请求
- **不做图内环。** agent 的工具循环建模为后续的 `AgentLoop` 节点：内部有界
  循环（max_iterations），对图仍是一个节点。条件分支、子图同样后置
- 失败：节点错误即整图失败上抛（v1 不做重试/降级策略）

## 5. ST 兼容映射要点

- 世界书激活 v1 子集：关键词扫描（key/secondary + selectiveLogic）、constant、
  order/预算、position（before/after char、at depth）、大小写。递归扫描、
  sticky/cooldown、group scoring、vectorized 后置
- 宏 v1 子集：`{{char}}` `{{user}}` `{{description}}` `{{personality}}`
  `{{scenario}}` `{{mesExamples}}` `{{persona}}` `{{random}}` `{{roll}}`
  `{{time}}` `{{date}}`；引擎做成 core 内可扩展纯函数
- 正则 placement 映射：`USER_INPUT` → runner 入口；`AI_OUTPUT` → 图内
  TextTransform；`WORLD_INFO` → ContextBuild 内；作用域顺序全局 → 角色 → 预设,
  导入默认禁用（信任状态本地单独存）
- 角色卡导入：CCv2 JSON 与 PNG `chara` chunk;`data.*` 为准，`character_book`
  转独立世界书实体 + 绑定，未知 extensions 保真。CCv3 / CharX 不做

## 6. IR 前置补丁

pipeline 组装的终点是 IR 的 `GenerateRequest`，当前 IR 表达不了 ST 预设，
需要先补（详见对 IR 的点评）：

1. `SamplingOptions` 补 `frequency_penalty` / `presence_penalty`
2. 新增 `ReasoningOptions`（effort / budget_tokens / 摘要可见性）
3. 历史消息允许 system 角色（深度注入 role=system 是 ST 核心机制）
4. 增加按目标 kind 的 provider overlay（JSON merge patch，transform 后应用），
   承接 safety settings、logit_bias、textgen 采样器等专有参数
5. 采样参数改为 transform 后按目标 kind 直写，解除 canonical 格式的表达上限

## 7. Assets

```text
asset:      id, sha256, media_type, byte_size, storage_key, created_at
asset_link: id, asset_id, owner_kind(character/persona/conversation/message/world_book),
            owner_id, kind(avatar/emotion/background/gallery/attachment...), name,
            sort_order, created_at
```

- 内容寻址（sha256），同 blob 自动去重；DB 只存元数据
- 存储走 `AssetStore` trait（put/get/delete by key）：v1 本地目录（应用数据
  目录下），多实例部署时换共享目录或对象存储实现，DB 层不变
- `kind` 用字符串不用 enum（调查结论：分类会持续扩展），媒体类型用 MIME
- 生图/图编辑输出落 asset 并 link 到 message
- GC：按 link 引用计数，孤儿清理做成显式维护命令，不做自动删除

## 8. 实施切分（对应 ROADMAP 阶段 5 展开）

- **5a 实体与导入**：character / persona / world_book(+entry) / world_book_binding /
  regex_script / preset / asset(+link) 实体；CCv2 卡导入导出 + fixture 验收
- **5b 最小通路**：IR 前置补丁 → ContextBuild 纯函数（对 ST fixture 单测）→
  pipeline runner 只支持线性链 → 内置默认 pipeline → 聊天 UI 打通
- **5c 图泛化**：并行分支、AgentLoop 节点、pipeline 编辑 UI

图编辑 UI 是最大的可延期项：5b 阶段用户看到的只是"选预设、选渠道、开聊"，
默认 pipeline 内置即可，图对用户不可见。

## 9. 明确不做（v1）

图内环与条件分支、子图复用、CCv3/CharX/BYAF、instruct 与 text completion
预设、完整宏集、群聊、向量化世界书、pipeline_run 持久化、自动 GC。
