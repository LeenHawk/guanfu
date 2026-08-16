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

### 1.5 采样参数跨协议有损是显式决策

`SamplingOptions` 在 transform 后按目标协议直写；目标官方协议本身不支持的参数
会被静默丢弃，不产生 diagnostics，也不触发 route incompatible。这是有意的宽松
策略，不是缺口。当前落点（WebSocket 同 Responses）：

| 参数 | Responses | Chat Completions | Claude Messages | Gemini |
| --- | --- | --- | --- | --- |
| `temperature` / `top_p` | 写入 | 写入 | 写入 | 写入 |
| `top_k` | 丢弃 | 丢弃 | 写入 | 写入 |
| `seed` | 丢弃 | 写入 | 丢弃 | 写入 |
| `stop` | 丢弃 | 写入 | 写入 | 写入 |
| `frequency_penalty` / `presence_penalty` | 丢弃 | 写入 | 丢弃 | 写入 |

理由：

- 空档来自官方协议自身的能力差异，不是 transform 丢失；严格拒绝会让携带常见
  sampler 组合的预设（如 SillyTavern 导入件）在大量渠道上直接不可用。
- 采样参数只影响生成分布，丢弃后请求语义仍成立。与之相对，reasoning 选项影响
  计费、思维预算和历史回放，无法表达时必须 route incompatible。同一请求里两种
  策略并存是有意区分，不作统一。
- 需要提示时由 UI 层依据本表提示"当前路由下无效的参数"，codec 层不报错。

## 2. 2026-08-16 收口批次（原 P1/P2）

以下缺口已全部处理。所需的 gproxy 扩展（web_search/web_fetch 的
`max_uses` 与 `blocked_domains`、apply_patch 的 `max_characters`、`memory`
扩展 variant、OpenAI Model 的 `max_input_tokens`/`max_output_tokens`）已随
gproxy 2.7.0 发布；Realtime 建模同批发布，视频与生图路由随 2.8.0 发布。

### 2.1 工具语义

- web_search `max_uses`/`blocked_domains`、text_editor `max_characters`、memory
  工具定义经 gproxy Responses 侧扩展映射到 Claude 原生工具；OpenAI 系目标无法
  表达这些扩展时显式 route incompatible 参与路由回退，不会静默发给真实上游。
- `McpApproval::PerTool` 补 typed 载荷（always/never 各自的 tool_names），编码为
  官方 filter 对象；`mcp_approval_request` 建模为输出 item，答复走独立输入项
  `McpApprovalResponse`；`ToolResultKind::Mcp` 删除（MCP 工具服务端执行，客户端
  只回批准与否）。
- 服务端执行的托管调用（web_search / file_search / code_interpreter /
  image_generation / mcp / mcp_list_tools）完整响应与流式统一解码为
  `ToolExecution`（状态 + 原始 item + error），`ToolCall` 只保留需要客户端行动的
  kind。`ToolCall::WebFetch` 与 `ToolCall::Memory` 删除：canonical 线上
  web_fetch 结果并入 web_search_call item，memory 调用以名为 `memory` 的
  function call 到达。
- `ToolChoice::Allowed` 依据请求内工具定义还原类型，编码 typed 条目。

### 2.2 流事件

- `GenerateDelta::Compaction`、`ContentKind::ToolInput`、`OutputKind::Image`
  删除——canonical 线上没有来源（Claude compaction 流是文本 delta，工具输入走
  专用 delta 事件，图像走 image_generation_call）。
- `OutputKind` 新增 `Compaction` 与 `McpApprovalRequest`，两类 item 在流式
  added/done 全程可解码不再报 unmodeled；`OutputKind::ToolExecution` 随托管调用
  真实产生。

### 2.3 媒体 codec

- 图像编辑 multipart 补齐 background / output_compression / moderation /
  partial_images 与 stream 标志；`*.completed` 单图事件的包装修正（顶层
  `b64_json` 不再按 `data` 数组硬解）。
- 转录 multipart 编码 diarization（`known_speaker_names[]` 与
  `known_speaker_references[]` 成对）、`chunking_strategy`；`response_format`
  按请求内容选择 diarized_json / verbose_json；`Transcription.usage` 解码
  Duration / Tokens 两型。`KnownSpeaker.reference` 收紧为必填（wire 上成对
  数组，无 reference 的 speaker 无法表达）。
- `ImageEvent::Progress` 由 `*.partial_image` 帧产生；`ImageEvent::Started/
  Failed`、`SpeechEvent::Failed`、`TranscriptionEvent::Started/Failed` 删除，
  操作失败统一由外层 `SemanticStreamMessage::Error` 承接。

### 2.4 模型列表

- `Model.capabilities` 删除：没有可靠的跨 provider 能力矩阵，路由表是能力的
  事实来源。
- token 上限经 canonical OpenAI model 的 gproxy 扩展字段
  （`max_input_tokens` / `max_output_tokens`）从 Claude / Gemini 渠道透出，
  `context_limit` / `output_limit` 不再恒空。

## 3. 剩余

无。视频与生图的跨供应商路由已于 2026-08-16 补齐(gproxy 2.8.0):

- **生图**:Gemini 渠道有两条路由,按模型选。`gemini-*-image` 系走批准的
  跨操作路由(`create_image/edit_image (open_ai) → transform_to
  generate_content (gemini_generate_content)`);`imagen-*` 系走原生
  `:predict` 端点(gproxy 2.8.1,规则 dest_kind 用 provider kind `gemini`,
  同操作转换)。两条链路 guanfu 均有 fixture 验证。
- **视频**:语义 IR 为异步任务形态(Create/Retrieve/List/Delete/
  DownloadContent,`VideoJob.content_ref` 承接下载标识)。OpenAI 渠道直通
  /v1/videos 家族;Gemini 渠道经 gproxy 转为 Veo predictLongRunning 与长时
  操作轮询,完成后从 `gproxy_video_uri` 提取文件 id 供下载。Remix/角色/
  编辑/续写等 OpenAI 专属操作暂不进 guanfu IR,需要时再加。

Realtime 已于 2026-08-16 落地(gproxy 2.7.0 建模 GA 协议;guanfu 侧
`CreateRealtimeCall` 走 multipart SDP 交换,`ConnectRealtime` 走 WebSocket
双工,语义事件见 `ir/realtime.rs`)。设计上的两个显式取舍:

- 未建模的 realtime 服务端事件(item 生命周期细节、限流通报、未来新增
  事件)被**有意跳过**而不是报错——长连接会话不因协议演进中断。这与 HTTP
  流式解码"未知即 unmodeled 错误"的策略不同,是 per-transport 的显式决策。
- Realtime 双工不经过通用 invoke / `/api/llm` 通路;壳层的专用通道
  (Tauri command + Channel、Axum WebSocket 端点)在对应 UI 阶段接入,
  接入前两壳对 Realtime 输出显式报错。

验证保持克制：每个已收口语义只保留一个完整响应或流式 fixture，证明 IR 字段
确实能从 wire 进入并在需要时回到 wire；不建设全量 provider 兼容矩阵测试。
