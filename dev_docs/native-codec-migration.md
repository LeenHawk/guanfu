# 原生 codec 迁移(取代 canonical 两跳)

决策(2026-08-16):生成通路从 `IR → Responses canonical → gproxy-transform →
目标 wire` 迁移为 **IR ↔ 各协议 typed wire 直达编解码**。方向 A:codec 留在
guanfu,wire 解析/序列化复用 gproxy-protocol 的 typed schema(wire 真理仍在
gproxy),gproxy-transform 退出生成通路。

## 动机

- 两跳的保真上限是 min(IR, Responses, transform):Claude/Gemini 原生语义必须
  先在 Responses 格式找到落点(thinking signature 绕道 encrypted_content、
  max_uses/memory 扩展字段手术),每个新语义都要 gproxy 发版配合。
- 流式每个 delta 经三种表示;损失政策要靠解读 transform diagnostics 间接实现。
- IR 类型层本身基本中立,问题集中在 codec 通路,不推倒类型。

## 目标架构

```text
codec/generation/
  request.rs / response.rs / stream.rs   ← Responses(现有,退化为"仅 OpenAI 系")
  claude.rs (+ claude_stream.rs)         ← IR ↔ claude::* typed schema 直达
  gemini.rs (+ gemini_stream.rs)         ← IR ↔ gemini::* typed schema 直达
  options.rs                             ← 各协议参数直写(已原生,并入各 codec)
  mod.rs                                 ← 按 target kind 分派到对应 codec
```

- 构造 gproxy-protocol 的 non_exhaustive struct 用其 WireBuilder(`T::builder()`);
  枚举 variant 可直接构造。
- 损失政策直接落在字段映射处:可移植字段直写;无法表达的语义按既有政策
  (采样静默有损 / reasoning 与工具 IncompatibleRoute 参与回退)。
- ProtocolOptions merge patch 与保护路径机制保留,应用于各协议最终体。

## 正确性策略

**现有 codec 测试全部断言 wire JSON 与语义事件,和实现方式无关——迁移前后
必须逐字节不变**(requests_and_replays_claude_reasoning_continuation、
decodes_claude_complete_response、两个 Claude SSE 测试、Gemini 回放等)。
新增测试保持克制:仅对新覆盖的语义补 fixture。

## 阶段(2026-08-16 全部完成)

1. **P1 Claude Messages(已落地)**:claude.rs / claude_tools.rs /
   claude_response.rs / claude_stream.rs;既有 9 项测试断言未改全过。有意偏离
   旧两跳并改善 live 兼容性:tool_choice 仅在有工具时发出、不再隐式展开
   web_fetch、内联媒体用 base64 source(旧路的 data: URL 会被 live API 拒)、
   文本块按块序 inline、mcp_tool_use 解码为 ToolExecution。历史 system 消息
   复用 `gproxy_transform::common::supports_mid_conv_system` 的模型判定。
2. **P2 Gemini generateContent(已落地)**:gemini.rs 系四文件 + 2 个新
   decode fixture。关键修复:流式 functionCall 参数在旧两跳会丢失,原生完整
   发出;thought 全文跨 chunk 聚合;instructions 改走 `systemInstruction`
   (旧路塞 contents);安全类 finishReason 统一 → ContentFilter。
3. **P3 Chat Completions(已落地,直达)**:chat.rs 系五文件。关键修复与
   增益:tool_choice 仅在有工具时发出(旧路恒发,live API 拒)、
   `stream_options.include_usage` 补齐(旧路流式 usage 缺失)、
   `reasoning_content` 解码为 Reasoning item(旧路丢弃)、流式补全
   Started/OutputStarted/…/Finished 完整生命周期、allowed_tools 直写
   (旧路静默丢)。旧路静默丢弃的托管工具与 `modalities.image` 等改为显式
   IncompatibleRoute。WebSocket 生成目标显式报
   UnsupportedRouteImplementation(HTTP 通路无法传输 WS 帧)。
4. **P4 IR 残留复核:无需删除。** `InstructionRole::Developer`(OpenAI 系有
   真实落点,他处并入 system)、`OutputContent::Refusal`(Responses wire 有
   来源)、两级索引模型(三协议均可映射)——都满足"有 wire 来源"的存留
   标准。options.rs 的 Claude/Gemini 死分支已随迁移删除。
5. **P5 transform 退役(定稿)**:生成通路四个 kind 全部原生直达,
   transform 机器(CANONICAL_KIND、transform_request/response、
   StreamConverter)已从 generation/ 整体删除;generation/ 内对
   gproxy-transform 的唯一引用是 `common::supports_mid_conv_system` 的模型
   判定(非转换管线)。gproxy-transform 在 guanfu 仅剩两类用途:跨操作路由
   (生图 generateContent 路由、视频 Veo、压缩)与非生成操作的 provider 间
   转换(models/count_tokens/embeddings)。

## 风险

- 两个流式状态机是主要工作量(各约 300-500 行);以现有 SSE fixture 为规格。
- WireBuilder 构造较冗长;换来的是 schema 变更时的编译期报错(比裸 JSON 强)。
- gproxy-transform 里沉淀的边角映射知识(citation 丢失报告、工具结果不对称等)
  需要在映射处逐条重新对照,不能默认"转换层会处理"。
