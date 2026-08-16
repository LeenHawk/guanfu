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

- [ ] failover 真渠道验证：坏凭证触发轮换 / 冷却 / 恢复三条路径
- [ ] 取消与超时复查：Axum 客户端断开 → 上游请求中止；两壳超时语义一致
- [ ] i18n 引入（Paraglide；中文默认 + fallback），AGENTS.md 回填方案
- [ ] 主题收尾：跟随系统 + 手动切换、首屏无闪烁、favicon 亮暗
- [ ] 渠道 UI 打磨：表单验证、loading/empty/error 状态、移动端；Chrome DevTools 验收（三档断点、亮暗、console 干净）

验收：UI 完整配置一个真实渠道并调通生成；拔掉首选凭证后请求仍成功。

## 阶段 2：Roleplay 领域（应用主体）

实施依据 `dev_docs/roleplay-assets-plan.md`：

- [ ] Asset 实体体系：头指针 + 不可变修订 + 内容寻址 chunk（实心与 JSON 资产同一机制），二进制落 File/S3
- [ ] 领域 definition：character / persona / 世界书 / 预设 / 正则 / **聊天历史**（历史即 Asset：typed 消息内容 + 会话默认绑定 + fork 来源）
- [ ] **Run（运行）模型**：多 Asset 槽位输入 → pipeline 执行 → Asset 输出（新建 / 追加 / HashEdit 修订）；输入 revision 钉住，聊天只是默认 run 模板
- [ ] CCv2 导入导出（PNG `chara` chunk 与 JSON），fixture round-trip 验收
- [ ] ContextBuild 纯函数：prompt_order 编排、世界书激活（关键词/预算/位置）、宏子集、深度注入，以 ST fixture 单测
- [ ] 超集 workflow schema（槽位签名 + 节点图 + 输出声明）；V1 runner 限线性链，内置聊天默认模板
- [ ] 聊天 UI：ChatHistory 视图 + 继续对话 run，流式渲染、markdown、重试与 fork

验收：完整走一轮角色扮演对话并持久化，重启后可继续；同一份历史可作为其他 run 模板的输入（如总结成世界书）。

## 阶段 3：媒体能力接入

- [ ] 生图 / 图编辑 UI（角色头像、场景图；按模型走 Gemini 双路由）
- [ ] 视频任务 UI：创建 → 轮询 → 下载（`VideoJob.content_ref`）
- [ ] 音频：speech / transcription 按路由矩阵接入
- [ ] realtime 语音：Tauri Channel + Axum WebSocket 专用通道（core 双工已就绪，壳层目前显式拒绝）
- [ ] pipeline 图泛化：并行分支、AgentLoop（裁决沿革见 `dev_docs/archive/pipeline-and-assets.md`）

验收：角色配图与一次实时语音对话跑通。

## 阶段 4：打磨与发布

- [ ] Axum 侧多用户：鉴权、多实例约束复查（跨实例状态、并发写）
- [ ] 打包发布：`pnpm tauri build` 桌面产物；server 端部署方式
- [ ] 性能与可观测性复查：span 覆盖关键链路、慢查询、前端 CLS / 首屏
