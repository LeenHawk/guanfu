# 语义协议实现缺口审计

审计日期：2026-08-14。当前依赖为 `gproxy-protocol` / `gproxy-transform` 2.6.4。

本文只记录“公共 IR 已经承诺某项语义，但 codec、stream decoder 或 provider
transform 没有真正兑现”的问题。协议本来就不支持、并且 core 会明确返回
unsupported 的能力不算静默缺口。

## 1. 已收口的 P0

### 1.1 Reasoning continuation

观复现使用有序 `ReasoningPart` 和 typed `ReasoningContinuation`：

- 完整响应和流式 `OutputFinished` 都能保存可见 reasoning text、summary 和 opaque
  continuation。
- OpenAI encrypted content、Claude signature/redacted data 和 Gemini
  `thought_signature` 会按实际 route 标注。
- summary delta 与 reasoning text delta 是不同事件。
- 带 continuation 的历史只允许回放到兼容协议，否则返回 route incompatible。
- OpenAI Responses 请求 reasoning 时自动包含
  `reasoning.encrypted_content`。

gproxy 2.6.4 已经修复此前的上游 continuation 保真缺口：

- Claude thinking text、signature 和 redacted thinking data 在完整响应中直接映射为
  Responses reasoning item。
- Claude 流式 signature delta 会进入完成的 reasoning item。
- Gemini 完整响应与流式响应都会把 `thought_signature` 映射为
  `encrypted_content`。
- Responses 历史中的 `encrypted_content` 能按目标协议映回 Claude signature 或
  Gemini `thought_signature`。

这里不再需要提交 gproxy issue。消息落库由阶段 5 的 conversation/message 实体
完成，不属于协议 codec 缺口。本仓库只消费 typed transform，不增加 provider
原生 JSON 旁路。

### 1.2 生成结束原因和失败状态

完整响应现在结合 status、output 和 `incomplete_details` 产生：

- `Stop`
- `Length`
- `ToolCalls`
- `ContentFilter`
- `Refusal`
- 无法进一步归因时的 `Incomplete`

流式 decoder 会记住需要客户端处理的 tool call/refusal，再决定最终 finish。
`response.failed` 产生 `GenerationFailure`，完整 failed response 返回结构化
`OperationFailed`，不再伪装为成功响应。

### 1.3 ProtocolOptions 核心字段保护

merge patch 现在按目标协议保护公共 IR 拥有的 JSON path：

- Responses 的 `text`、`reasoning`、`modalities` 和 output limits
- Claude 的 `max_tokens`、`thinking`
- Gemini `generationConfig` 下的 output schema、output limit 和 thinking config

包含受保护子路径的对象不能通过 `null` 或标量整体替换。sampling path 和没有公共
IR 落点的协议专用字段仍可覆盖；未匹配目标协议的 options 仍默认丢弃。

### 1.4 Reasoning summary 跨协议语义

公共 IR 继续区分 `auto/concise/detailed`：

- Claude 只接受可等价表达的 `auto`。
- Gemini 不再把 summary 错映射成 `includeThoughts`。
- Chat、Gemini 或 Claude 无法表达的 summary 值返回 route incompatible，参与路由
  fallback。

## 2. P1：对应能力开放前修复

### 2.1 工具定义、调用和结果不对称

工具 IR 中存在以下未兑现字段或 variant：

- `WebSearchTool.max_uses` 没有编码。
- `TextEditorTool.max_characters` 被完全忽略。
- `McpApproval::PerTool` 被编码成字符串 `per_tool`，但 Responses 要求 typed tool
  filter 对象；当前 wire 值无效。
- `mcp_approval_request` 没有输出模型和 decoder，`ToolResultKind::Mcp` 却固定编码成
  approval response，调用与审批语义混在一起。
- `ToolCall::WebFetch` 和 `ToolCall::Memory` 没有任何 decoder 构造路径。
- Memory definition 被降成普通 function，响应只能解码成 Function call，声明专用
  variant 没有意义。
- `ToolExecution` 只有部分进度 delta，没有完整 output item 对称解码；相关
  `OutputKind::ToolExecution` 也不会产生。
- `ToolChoice::Allowed` 只保存名称并把所有目标编码成 function，无法选择 hosted、
  custom 等其他 tool kind。

默认角色扮演聊天只需要 function tool 时可暂不扩展全部工具，但进入 AgentLoop 前
必须删除虚假的专用 variant或补齐 typed 语义。

### 2.2 Generation stream 中存在死事件

已经声明但 decoder 不会产生的内容包括：

- `GenerateDelta::Compaction`
- `OutputKind::ToolExecution`
- `ContentKind::ToolInput`

Compaction output item 也不在 `OutputKind` 中，流式收到 compaction item 时会直接报
unmodeled event。修复时应保持完整响应和流式最终 `OutputFinished.item` 对称；如果
当前版本不准备支持某个事件，应先从公共 enum 删除，避免调用方误以为可用。

### 2.3 图像编辑 multipart 丢字段

`EditImageRequest` 先生成完整 JSON，随后 OpenAI multipart 重建只保留一部分字段。
以下已声明选项在 multipart 路径丢失：

- background
- compression
- moderation
- partial_images
- stream flag

因此 image edit stream 虽然选择 SSE response mode，上游请求却没有 `stream=true`。
在图编辑 UI 开放前补齐；不支持的选项应显式 route incompatible，不能无声忽略。

### 2.4 转录 diarization 和 usage 是死能力

`TranscriptionRequest` 已声明 `DiarizationConfig`、known speakers 和 chunking，但
multipart encoder 完全不读取这些字段。时间戳请求也没有配套选择 verbose/diarized
response format，可能无法得到 IR 声明的 words/segments。

响应 decoder 永远把 `Transcription.usage` 写成 `None`，`AudioUsage` 两个 variant
没有构造路径。音频能力接入 UI 前需要完成编码、响应格式选择和 usage 解码；否则
删除尚未支持的字段。

## 3. P2：清理误导性表面能力

### 3.1 模型能力信息永远为空

`Model.capabilities`、context limit 和 output limit 已经公开，但当前 model decoder
固定把 capabilities 写成 `None`。limit 也只有上游恰好返回 canonical 字段时才有
值。若没有可靠 provider 映射，应把它们明确标成 optional discovery，而不是据此
决定 route 能力；route 的事实来源仍是 channel routing table。

### 3.2 媒体流式生命周期 enum 与实际 decoder 不一致

`ImageEvent::Started/Progress/Failed`、`SpeechEvent::Failed`、
`TranscriptionEvent::Started/Failed` 没有 codec 构造路径。部分失败实际通过外层
`SemanticStreamMessage::Error` 传递。

应选择一个统一规则：操作失败都走外层结构化 stream error，或各 operation event
都产生 Failed；不要同时公开两套但只实现一套。当前没有真实 wire 来源的
Started/Progress 应暂时删除。

### 3.3 Realtime 是明确未实现，不是静默缺口

Realtime request/response 类型已经存在，但 codec 会明确返回
`UnsupportedCapability::Realtime`。这部分可以继续后置，但在实现前不能在产品
能力矩阵中标记为可用。

## 4. 修复顺序

1. 确定 message content 的版本化 schema，进入会话持久化。
2. AgentLoop 前收口工具不对称。
3. 图像/音频 UI 接入前分别收口 media codec。
4. 删除没有当前实现、也没有近期使用方的公共 enum variant。

验证保持克制：每个缺口只保留一个完整响应或流式 fixture，证明 IR 字段确实能从
wire 进入、持久化并在需要时回到 wire；不建设全量 provider 兼容矩阵测试。
