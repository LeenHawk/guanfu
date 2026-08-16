# Roleplay Assets 与 Pipeline 实施计划

本文是 Roleplay 阶段的当前实施依据。角色卡、persona、世界书、预设、正则、
pipeline 与**聊天历史**统一建模为带 kind 的版本化 JSON Asset;执行统一为
**Run(运行)**——以多个 Asset 为输入、以 Asset 为输出的一次 pipeline 执行,
聊天只是默认的 run 模板。数据库保存领域文档和关系数据,File/S3 只保存图片、
音频等二进制内容。

## 1. 当前基础

通用 LLM IR、原生 codec、operation 路由和双端流式适配已经落地,并通过
codex / aistudio / claudecode 三条真实渠道冒烟(模型列表、非流式、流式)。
IR 前置补丁均已完成:

- sampling 支持 frequency/presence penalty，并按目标协议编码
- reasoning 请求参数已经建模
- 历史消息允许 system role
- 协议专用参数使用按目标 kind 匹配的 JSON merge patch
- 未命中的协议参数和目标不支持的 best-effort sampling 默认丢弃
- 请求 transform incompatibility 可以触发 route fallback
- reasoning continuation、finish/failure 和协议核心字段保护已经收口

协议缺口已全部收口(含 realtime、视频、生图双路由),审计与原生 codec
迁移的完结记录见 [`archive/protocol-gap-audit.md`](archive/protocol-gap-audit.md)
与 [`archive/native-codec-migration.md`](archive/native-codec-migration.md);
message definition 和聊天 runner 没有任何协议层阻塞。

## 2. 存储边界

### 2.1 数据库保存领域状态

以下内容以数据库为事实来源：

- Asset 头指针、不可变修订(manifest)与内容寻址 chunk(见 §3)
- Asset 之间由 typed ID 表达的引用
- run 执行记录(输入/输出槽位、状态、用量)
- channel、credential 和 routing rule 等连接配置

数据库提供查询、事务、并发控制和多实例一致性。实心(二进制)Asset 与 JSON
结构 Asset 共用同一套"不可变内容 + 小的可变清单"机制,只是字节的存放地
不同(§2.2);run 记录保持关系行。

### 2.2 AssetStore 保存二进制

图片、音频和导入时选择保留的原始文件不进入数据库:它们是
`location = store` 的 chunk,与 db 内的文本 chunk 共用同一 sha256 命名空间。
Media 的 manifest 只保存:

- chunk 哈希(即 sha256)
- MIME type
- byte size
- storage key

实际字节通过注入 AppState 的 `AssetStore` 读取和写入：Tauri 与单实例 Axum
可以使用本地目录，多实例 Axum 使用 S3 或兼容对象存储。外部存储写入不能放在
数据库事务中；先写对象、再提交元数据，失败产生的孤儿对象由后续显式维护操作
清理。

## 3. 统一 Asset 实体:头指针 + 不可变修订 + 内容寻址 chunk

三张表覆盖全部 Asset(含二进制与聊天历史)。Run 钉住输入 revision 要求历史
修订可取回,这套结构让修订历史近乎免费:

```text
asset          — 可变头指针
- id / kind / name / head_revision / created_at / updated_at

asset_revision — 不可变修订
- asset_id / revision / manifest JSON / created_by_run_id? / created_at

chunk          — 内容寻址,跨资产共享
- hash(canonical 内容的 sha256)/ location(db | store)/ bytes? / size
```

- manifest 是 definition 的骨架:标量字段内联,可编辑单元列表存为 chunk
  哈希数组。粒度按 kind 选:ChatHistory 逐消息、WorldBook 逐 entry 成
  chunk;Character / Persona / Preset / RegexScript / Pipeline 体量小,
  manifest 全内联(零 chunk),同一 schema 不强拆。
- chunk 不可变、按哈希写入幂等;并发只剩 asset 头指针的 CAS
  (`id + head_revision`),修订本身永不改写。
- fork = 新 asset 指向复制的 manifest,全部 chunk 结构共享;跨资产去重
  天然成立。孤儿 chunk 由显式维护操作按 manifest 引用清理,与 Media GC
  同一策略。
- `kind` 使用 SeaORM `ActiveEnum`;manifest 不以裸 `serde_json::Value` 穿过
  core 服务边界;definition 的 schema version 由各 typed serde enum 自己
  携带,不与数据库 schema 版本混淆。

第一版 Asset kind：

```rust
pub enum AssetKind {
    Character,
    Persona,
    WorldBook,
    OpenAiChatPreset,
    RegexScript,
    Pipeline,
    ChatHistory,
    Media,
}
```

数据库 Model 不直接作为 API DTO。core 根据 kind 将 JSON 解码为对应类型：

```rust
CharacterDefinition::V1(...)
PersonaDefinition::V1(...)
WorldBookDefinition::V1(...)
OpenAiChatPresetDefinition::V1(...)
RegexScriptDefinition::V1(...)
PipelineDefinition::V1(...)
ChatHistoryDefinition::V1(...)
MediaDefinition::V1(...)
```

kind 与 definition 类型不一致时返回稳定的结构化错误。未知交换字段保存在对应
definition 的 `extra`，不自动进入模型请求。

### 3.1 引用策略

第一版不增加通用 `asset_reference` 表。资源引用作为 typed Asset ID 保存在
definition 中，并由服务在保存和加载时检查目标是否存在、kind 是否匹配：

- Character 可以引用 WorldBook、RegexScript 和 Media
- Persona 可以引用 Media 和 WorldBook
- OpenAiChatPreset 可以引用 RegexScript
- Pipeline definition 可以引用其他 Asset，但默认 pipeline 不固定环境资源
- ChatHistory 可以引用 Character / Persona / Preset / WorldBook / Media
  (会话默认绑定与消息内媒体),并记录 fork 来源

如果以后出现按反向引用查询、级联策略或大规模 Media GC 的实际需求，再把引用
投影到关系表；当前不提前维护 JSON 与关系表两份事实来源。

## 4. 各 Asset definition 的第一版范围

### 4.1 Character

Character definition 保存 CCv2 可移植内容：

- name、description、personality、scenario
- creator notes、system prompt、post-history instructions
- 有序 greetings；第一项对应 `first_mes`，其余对应 `alternate_greetings`
- 示例对话、tags、creator、character version
- WorldBook/RegexScript/Media Asset 引用
- CCv2 extensions 与未识别字段的保真数据

SillyTavern 本地 chat、收藏、文件名和 proxy 状态不进入共享 Character definition。

### 4.2 Persona

Persona definition 保存名称、描述、position、头像 Media 引用和交换扩展。默认
persona 或会话锁定属于运行选择，不写入可分享 Persona。

### 4.3 WorldBook

WorldBook definition 内保存有序 entries，不单独建立 entry 表。第一版表达：

- primary/secondary keys 与 selective logic
- content、enabled、constant、order
- position、depth、role
- probability 与大小写设置
- 递归和预算所需的已调查字段
- 未知交换字段

角色内嵌世界书只是 CCv2 交换形态。导入时创建独立 WorldBook Asset，并在
Character definition 中记录引用。

### 4.4 OpenAI Chat Preset

Preset definition 保存：

- 有序 prompts
- prompt-order profiles
- sampling 与 reasoning
- ST 上下文组装辅助字段
- RegexScript Asset 引用
- 导入导出保真使用的 extra

channel、credential、base URL、proxy 和 model 选择不进入可分享 preset。

### 4.5 RegexScript

Regex definition 保存脚本、placement、顺序和 ST 交换字段。信任/启用授权是
本地运行状态，导入脚本默认不受信任，不能仅凭 Asset 内容获得执行权限。

### 4.6 Pipeline(超集 workflow)

Pipeline definition 使用版本化 typed JSON,是覆盖 ST 聊天与全部 agent run
的超集 schema:

```text
PipelineDefinition::V1
- inputs:  [ { name, kind: AssetKind, many, required } ]   // 槽位 = 类型签名
- params:  [ { name, ty } ]                                // 非 Asset 运行参数:
                                                           //   user_message、trigger…
- nodes:   [ { id, kind: NodeKind, config } ]
- edges:   [ { from: "node.port", to: "node.port" } ]      // 端口类型化
- outputs: [ { slot, op: Create | Append | HashEdit, from: "node.port" } ]
```

三条纪律:

- **槽位是类型签名**:run 发起时按 kind 校验绑定并钉 revision;UI 可据此
  自动生成表单。
- **写入是输出声明,不是节点**:节点保持纯(一次模型交互或一次纯变换,
  提示词片段不作为节点);对 Asset 的新建/追加/HashEdit 收敛在 `outputs`,
  run 结束原子提交(与 §5.3 的工具改动暂存同一规则)。
- **端口值类型有限集**:`PromptBundle / Text / Json / GenerationResult /
  AssetRef`,edges 在加载期校验类型匹配;未知节点 kind 加载即报错,不静默
  跳过。

节点种类分层:V1 为 `ContextBuild`(槽位 + trigger → PromptBundle)、
`Generate`(终端节点流式直通)、`TextTransform`(正则);图泛化阶段追加
`MediaGenerate`、`AgentLoop`(有界工具循环,挂 §5.3 的 asset 操作)与
`Parallel` / `Map` 结构节点,schema 不变,只放开 runner 校验。

ST 聊天是参数最普通的内置模板:

```text
inputs:  history character persona? world_books[] preset
params:  user_message, trigger
nodes:   ctx(ContextBuild) → gen(Generate) → post(TextTransform)
outputs: history ← Append { user_message + post 结果(含 reasoning parts) }
```

"总结成世界书"只是换签名(inputs: history;outputs: world_book ← Append)。
默认 pipeline 不固定 channel、model,由 params 提供。V1 runner 只接受线性链;
DAG、AgentLoop 和编辑 UI 后置。

### 4.7 Media

Media definition 只描述存储在 AssetStore 中的对象，不包含二进制。Character、
Persona 和 ChatHistory 消息通过 Asset ID 引用它。

### 4.8 ChatHistory

聊天历史是一等 Asset,definition 保存:

- 有序 messages:typed content(文本、有序 ReasoningPart、tool call/result、
  Media 引用),不传裸协议 JSON
- 会话默认绑定:Character / Persona / Preset / Pipeline 的 typed Asset 引用与
  channel/model 选择(可被单次 run 覆盖)
- title、fork 来源(源 ChatHistory 与消息位置)、交换扩展

历史即 Asset 带来的自由度是本设计的核心:任何 run 都能以历史为输入
(继续对话、总结成世界书、提炼记忆、翻译、合并多段历史),输出新的历史或
其他 Asset;分支 = fork 一份 definition 并记录来源,无需分支树表。

存储形态:逐消息成 chunk,每轮对话 = 新增消息 chunk + 一份新 manifest
(流式 delta 是临时 UI 进度,不落库)。追加是小写入,修订历史近乎免费,
fork 零成本(§3);头指针 CAS 保证并发安全。

## 5. Run(运行)模型

执行统一为 run:一次 pipeline 执行,以命名槽位绑定多个 Asset 输入,产出
Asset 输出。聊天回合、总结、批量生图都是 run,区别只在 pipeline 模板与槽位:

```text
run
- id
- pipeline_asset_id
- status: pending / running / succeeded / failed / cancelled
- inputs JSON:  [{ slot, asset_id, revision }]
- outputs JSON: [{ slot, asset_id, revision }]
- error JSON optional
- usage JSON optional
- created_at / finished_at
```

- Pipeline definition 声明槽位及其 AssetKind(如聊天模板:输入
  `history/character/persona?/world_books[]/preset`,输出 `history`);
  runner 按槽位解析并**钉住输入 revision**,可复现、可追溯。
- 输出操作暂定三种:**新建**(new Asset)、**追加**(新增 chunk + 新
  manifest:历史消息、世界书 entry)、**修订**(以 HashEdit 锚定,见 §5.2);
  全部落地为 manifest 手术 + 头指针 CAS,run 结束原子提交并记录 run 行;
  不持久化逐节点事件。
- channel/model 与槽位绑定由 RunInput 提供,UI 可从 ChatHistory 的会话默认
  绑定预填。
- 聊天界面 = ChatHistory Asset 视图 + "继续对话" run 模板;重试 = 从上一
  revision 再跑;分支 = fork 历史后继续。

Asset ID 引用只保证目标存在,具体 kind 由 core 服务按槽位声明校验。

### 5.2 HashEdit:修订的锚定原语

可编辑单元就是 chunk(§3),`target_hash` 即 chunk 哈希——编辑原语、
存储寻址、并发校验共用一个哈希体系。修订指令:

```text
{ target_hash, op: replace | delete | insert_after, new_content }
```

- 按哈希定位目标;找不到或不唯一 → 结构化错误(stale / ambiguous),调用方
  重读后再试——锚内容而非索引,失配显式报错,不静默错位。
- 一批指令原子应用于 revision N 的 manifest,提交为 N+1;与头指针 CAS
  正交组合,并发冲突走 CAS 重放。
- 对 LLM:免整文档重写(省 token、防覆盖丢失),是后续"Asset 操作挂给
  模型"的编辑原语。

### 5.3 Asset 操作面(方向,后置)

按 kind 内建标准操作集(WorldBook:append_entry / edit_entry / delete_entry;
ChatHistory:append_message / edit_message;Character:edit_field…),run 期间
由 runner 挂成 function tools(如 `asset.<slot>.<op>`),模型在工具循环里
直接操作 Asset:调用 → 服务端应用 HashEdit / 追加 → 工具结果携带新哈希,
run 结束统一按输出槽提交 revision。definition 自声明的自定义操作更后置。

### 5.1 Reasoning continuation

assistant message 必须保存完整、有序的 reasoning 输出，而不是只保存 UI 可见文本。
第一版使用 typed parts：

```rust
pub struct ReasoningOutput {
    pub id: OutputId,
    pub parts: Vec<ReasoningPart>,
}

pub enum ReasoningPart {
    Summary { text: String },
    Text {
        text: String,
        continuation: Option<ReasoningContinuation>,
    },
    Opaque { continuation: ReasoningContinuation },
}

pub enum ReasoningContinuation {
    OpenAiEncrypted { content: String },
    ClaudeSignature { signature: String },
    ClaudeRedacted { data: String },
    GeminiThoughtSignature { signature: String },
}
```

这些 continuation 是 provider 生成、客户端不能解释的续接状态，不是原生协议
事件透传：

- 随 ChatHistory Asset 的当轮 revision 提交原样保存，不放 AssetStore。
- 保持 reasoning、tool call 和普通文本的原始次序，不拼接或重新签名。
- 不写日志、不默认发送给 UI、不进入角色卡或普通聊天导出。
- UI 只展示 summary 或 provider 明确返回的可见 reasoning text。
- 同协议、同模型的 continuation 原样回放；普通已提交历史切换协议时只使用可移植
  文本和摘要。
- 未完成的工具循环固定原 route/model。缺少必要 continuation 时返回 route incompatible，
  不能静默丢弃后继续执行。

gproxy 2.6.4 在协议边界把 Claude signature/redacted data 和 Gemini
`thought_signature` 统一放入 Responses reasoning item 的 `encrypted_content`，并能
在回放时按目标协议还原。观复的 decoder 根据实际 route 和 reasoning item 是否带
可见 text，将这个 canonical opaque value 标注为对应的 `ReasoningContinuation`；不自行
解析内容，也不增加 provider 原生 JSON 旁路。

OpenAI `encrypted_content`、Claude redacted data 和各家 signature 本身按 opaque
字符串保存，不需要只对这些字段再加一层应用加密。如果威胁模型要求防止数据库
泄露，应加密整个 message payload 或数据库，因为用户消息、可见 reasoning 和
summary 同样敏感。

原生 codec 已完成有序 parts 解码、流式最终项和同协议回放(Claude/Gemini
signature 直达,不再经 canonical 绕行)。消息持久化在 ChatHistory Asset 的
definition 中实现。

## 6. CCv2 导入导出

第一版支持：

- CCv2 JSON
- PNG `chara` chunk
- `data.*` 优先
- greetings 顺序保持
- embedded character book 转换为 WorldBook Asset 引用
- 未知 extensions 保真

第一版不支持 CCv3、CharX、BYAF、instruct preset 或 text completion preset。
PNG 同时包含 `chara` 和 `ccv3` 时只读取 `chara`。

导入分为两层：

1. 纯 exchange codec 将外部格式转换为 typed definitions。
2. service 在一个数据库事务中保存 Character、WorldBook 等 Asset。

PNG 文件本身只有在用户选择保留原件时才作为 Media Asset 保存；解析后的
Character definition 始终是数据库中的正式可编辑数据。

## 7. ContextBuild 与执行边界

5b 的数据流：

```text
runner 按 run 输入槽位加载并解析 Asset(含 ChatHistory)
  -> ContextSnapshot
  -> build_context(snapshot)
  -> GenerationDraft
  -> Generate
  -> GenerationResult
  -> TextTransform
  -> committed output
```

`ContextBuild` 是纯函数，负责 prompt order、宏展开、世界书激活与预算、示例
对话裁剪、深度注入和历史裁剪。数据库查询和 Asset JSON 解码在 snapshot loader
完成；需要模型调用的 compact 不能藏进 ContextBuild。

Generate 的 OperationEvent 作为临时进度流向 UI，runner 累积完整结果；下游
TextTransform 完成后再发出最终 committed output。PipelineEvent 只携带已经建模
的语义事件，不透传原生 SSE 或协议 JSON。

## 8. 分步实施与提交

### 5a：Asset、会话与交换格式

1. `feat: add unified asset storage`
   - asset / asset_revision / chunk 三表与头指针 CAS
   - 各 definition 的版本外壳、typed Asset ID 与 manifest 编解码边界
   - 按 kind 的 chunk 粒度(ChatHistory 逐消息、WorldBook 逐 entry,其余内联)

2. `feat: model roleplay asset definitions`
   - Character、Persona、WorldBook
   - OpenAI Chat Preset、RegexScript、Pipeline、Media
   - 重新生成 ts-rs bindings

3. `fix: preserve reasoning continuation state`（已完成）
   - 有序 ReasoningPart 与 typed ReasoningContinuation
   - 完整响应和流式响应对称累积
   - 消费 gproxy 2.6.4 的 canonical continuation 转换
   - 结束原因与失败语义收口

4. `feat: add chat history assets and runs`
   - ChatHistoryDefinition(typed message content、会话默认绑定、fork 来源)
   - run entity 与槽位解析(输入 revision 钉住、输出落 Asset)
   - 每轮整体提交与 revision CAS 的最小 service

5. `feat: support ccv2 json assets`
   - typed exchange DTO
   - JSON 导入导出
   - embedded world book 事务导入

6. `feat: add media asset storage`
   - `AssetStore` trait 与本地目录实现
   - Media manifest 元数据与 `location = store` chunk 接线

7. `feat: support ccv2 png assets`
   - PNG `chara` 读取和写出
   - 可选保留原始 PNG

### 5b：最小聊天通路

8. `feat: build roleplay context snapshots`
   - snapshot loader
   - 纯 ContextBuild
   - ST 必要语义子集

9. `feat: run linear generation pipelines`
   - 线性 runner
   - pipeline-level progress 与 committed output
   - 内置默认 pipeline

10. `feat: expose persistent roleplay chat`
    - run 服务接口(发起/取消/查询,聊天为默认模板)
    - Tauri Channel 与 Axum SSE 薄适配
    - 取消传播与当轮 revision 提交

11. `feat: add roleplay chat interface`
    - 选择 Character、Persona、Preset、Channel 和 model
    - 流式临时文本与最终 committed output
    - 消息持久化、重试和基础 markdown

### 5c：图泛化

- typed ports 与 DAG 校验
- 并行分支
- 有界 AgentLoop
- pipeline 编辑 UI
- 条件节点、子图和结构化循环按实际需求继续评估

## 9. 验证范围

不做大而全的兼容测试。当前只保留：

- schema sync 覆盖新增实体
- Asset kind 与 definition 不匹配时的边界测试
- 一个 CCv2 JSON fixture：标准字段、greetings 顺序、extensions 保真
- 一个 embedded character book 导入测试
- 一个 PNG `chara` round-trip
- 一个 ContextBuild fixture
- 一条线性 pipeline 的流式进度与最终提交测试

每个 Rust 提交运行 `cargo fmt`、`cargo check` 和无警告的 `cargo clippy`；只有改变
前端或 bindings 时运行对应 pnpm 检查。

## 10. 明确后置

- CCv3、CharX、BYAF
- text completion/instruct preset
- 群聊;分支树 UI(fork-by-copy 天然支持,树状可视化后置);逐节点 run 事件持久化
- Asset 操作挂给 LLM 的工具循环(§5.3,内建操作集先行,definition 自声明操作更后置)
- 完整 SillyTavern 宏与世界书高级行为
- 图内任意环、条件节点和可视化编辑器
- 对象存储具体厂商实现
- 自动 Media GC
