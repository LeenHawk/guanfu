# 观复实现路线

按阶段推进，每阶段有明确验收标准；阶段结束跑全套门禁并提交。顺序可按需要调整，阶段内条目可拆成独立提交。

## 现状（已完成，2026-08-16）

- 工程与门禁：`frontend/`（SvelteKit + Tailwind 4 + lint/format）+ `backend/`（core / tauri / server 三 crate），pnpm + cargo workspace
- 语义 IR 覆盖全操作面：文本/推理生成、工具（含 MCP 审批）、生图（Gemini 双路由：generateContent 与 Imagen 原生）、音频三件套（含 diarization）、视频异步任务（OpenAI / Veo）、模型列表（含 token 上限）、计数（含本地 tiktoken）、embeddings、搜索/重排、压缩、会话、realtime（WebRTC SDP + WebSocket 双工）
- 原生 codec：四个生成 kind（Responses / Chat / Claude / Gemini）直达 typed wire，gproxy-transform 退出生成通路（仅保留跨操作路由与非生成转换）；经 codex / aistudio / claudecode 三真渠道冒烟（模型列表、非流式、流式）
- 渠道体系：channel / credential / routing_rule 实体，`(operation, kind)` 逐格路由矩阵（含跨操作路由），凭证池排序 / 失败分类 / 指数退避冷却
- 双壳贯通：AppState 共享；Tauri command（Channel 流式 + 取消）与 Axum（SSE）同一 core；ts-rs 契约生成（`pnpm gen:bindings`）
- 渠道管理 UI 首版：invoke/fetch 双通道 api 层、channel / credential / routing 工作区组件、主题基础
- 依赖 gproxy 2.8.2（工具扩展、Realtime 建模、Veo / Imagen、模型 id 归一化随本项目推进发布）

设计文档：阶段 2 依据 `dev_docs/roleplay-assets-plan.md` 与
`dev_docs/sillytavern-resource-formats.md`；已完结的协议审计与原生 codec
迁移记录在 `dev_docs/archive/`。

## 阶段 1：通路收尾与前端基建补课

- [x] failover 真渠道验证：坏凭证触发轮换 / 冷却 / 恢复三条路径
- [x] 取消与超时复查：Axum 客户端断开 → 上游请求中止；两壳超时语义一致
- [x] i18n 引入（Paraglide；中文默认 + fallback），AGENTS.md 回填方案
- [x] 主题收尾：跟随系统 + 手动切换、首屏无闪烁、favicon 亮暗
- [x] 渠道 UI 打磨：表单验证、loading/empty/error 状态、移动端；Chrome DevTools 验收（三档断点、亮暗、console 干净）

验收：UI 完整配置一个真实渠道并调通生成；拔掉首选凭证后请求仍成功。

## 阶段 2：Roleplay 领域（应用主体）

实施依据 `dev_docs/roleplay-assets-plan.md`：

- [x] Asset 实体体系：头指针 + 不可变修订 + 内容寻址 chunk（实心与 JSON 资产同一机制），二进制落 File/S3
- [x] 领域 definition：character / persona / 世界书 / 预设 / 正则 / **聊天历史**（历史即 Asset：typed 消息内容 + 会话默认绑定 + fork 来源）
- [x] **Run（运行）模型**：多 Asset 槽位输入 → pipeline 执行 → Asset 输出（新建 / 追加 / HashEdit 修订）；输入 revision 钉住，聊天只是默认 run 模板
- [x] CCv2 导入导出（PNG `chara` chunk 与 JSON），fixture round-trip 验收
- [x] ContextBuild 纯函数：prompt_order 编排、世界书激活（关键词/预算/位置）、宏子集、深度注入，以 ST fixture 单测
- [x] 超集 workflow schema（槽位签名 + 节点图 + 输出声明）；V1 runner 限线性链，内置聊天默认模板
- [x] 聊天 UI：ChatHistory 视图 + 继续对话 run，流式渲染、markdown、重试与 fork

验收：完整走一轮角色扮演对话并持久化，重启后可继续；同一份历史可作为其他 run 模板的输入（如总结成世界书）。

已验收（真渠道 codex / gpt-5.6-sol）：UI 导入 Seraphina 角色卡 → 新建对话 → 流式回复 →
刷新后历史仍在（修订推进到 2，assistant 轮存的是 typed output items）。fork 经接口验证：
分支只保留到指定消息数，源历史不变。

阶段 2 未做（滚入后续）：Persona 与世界书的编辑 UI、预设编辑 UI、Media Asset 的
上传/引用接线（`AssetStore` 已就绪但尚未有业务入口）、Asset 操作挂给 LLM 的工具循环。

## 阶段 3：媒体能力接入

- [x] 生图 UI（创作台「生图」页；产物落成 Media Asset 并进媒体库）
- [x] 视频任务 UI：创建 → 轮询 → 下载（`VideoJob.content_ref`）
- [x] 音频：speech 合成与 transcription 转写按路由矩阵接入
- [x] realtime 语音：Axum WebSocket 专用端点 + Tauri Channel 双工命令，
      两壳共用一套下行帧;支持打字发言(合成麦克风下 VAD 切不出语音轮)
- [x] pipeline 图泛化：拓扑分层执行，同层节点并发（并行分支即图形状），
      新增 MediaGenerate / Map 节点，环与端口类型不匹配在加载期拒绝

验收（真渠道 codex，2026-08-16）：媒体库从空开始，UI 生图产出 2.2 MB PNG
并可从 AssetStore 回读；realtime 接通后模型给出实际回复。console 干净。

阶段 3 未做（滚入后续）：图编辑（`edit_image` 已通到 API 与命令，UI 只有生成）、
Gemini 生图双路由的 UI 侧模型分辨（后端路由矩阵已支持，前端不区分）、
AgentLoop 节点（要等 Asset 操作挂成工具才有意义）、正则脚本执行授权
（`TextTransform` 节点当前直通）。

## 阶段 4：打磨与发布

- [x] Axum 侧鉴权：共享令牌（`GUANFU_TOKEN`），未设令牌拒绝绑定非回环地址；
      realtime 令牌随 WebSocket 首帧校验，不进 URL
- [x] 多实例约束复查：数据库侧安全（不可变修订 + 头指针 CAS + 哈希幂等 chunk +
      共享冷却），`LocalAssetStore` 是硬约束，见 `dev_docs/deployment.md`
- [x] 打包发布：`pnpm tauri build` 桌面产物；server 端部署与反向代理要求文档化
- [x] 性能与可观测性：关键链路 span（生成 / run / 图节点 / 媒体 / Asset 提交），
      慢查询按 warn 单独报（>200ms）

补齐（2026-08-16）：

- 真正的多用户：账号 / 会话（argon2id + 只存令牌哈希），资产按归属隔离，
  私有默认、可共享；首个账号即管理员，之后由管理员建号。渠道全局共享、
  仅管理员可改。桌面壳用本地账号，两壳同一套可见性规则。会话可在账号面板
  逐个吊销或「在其他设备上登出」。
- `AssetStore` 的 S3 实现：自签 SigV4，path-style 与 virtual-hosted 两种寻址
  都支持（默认 auto：IP / localhost 走 path，其余走 virtual-hosted），
  三种模式均对 Cloudflare R2 实测。
- 前端首屏 / CLS 基线：见 `dev_docs/deployment.md`；CLS 从 0.047 降到 0。
- 懒加载：媒体库进入视口才取 src（桌面壳的 data URL 会把整份文件读进内存，
  `loading="lazy"` 拦不住）；聊天历史只渲染最近 60 条，更早的按需展开。

仍未做：逐用户的渠道配额、Azure Blob（无 S3 兼容端点，需另写一个
`AssetStore` 实现）、聊天历史的真正虚拟滚动（当前是分段展开）。
