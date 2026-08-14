# 观复实现路线

按阶段推进，每阶段有明确验收标准；阶段结束跑全套门禁并提交。顺序可按需要调整，阶段内条目可拆成独立提交。

## 现状（已完成）

- 工程结构：`frontend/`（SvelteKit + Tailwind 4 + lint/format）+ `backend/`（Cargo workspace），pnpm workspace，门禁齐备
- core 骨架：
  - 实体（entity-first，sync 已实测）：`channel` / `credential` / `routing_rule`
  - 渠道路由矩阵：`(operation, kind)` → passthrough / transform_to / local / unsupported，`local` 尚未落地实现
  - `LlmClient`：reqwest 0.13 + rustls(ring)，gproxy-protocol 合成端点，SSE 字节流
  - `TransformPlan`：请求正向、响应与 SSE 反向转换（gproxy-transform）
  - 凭证池：权重排序 + 轮换偏移、失败分类、指数退避冷却（原子自增）、渠道级联删除走事务
- 应用图标亮暗两套；AGENTS.md 约束体系成型

## 阶段 1：中间层贯通

目标：后端骨架真正跑起来，双壳共用同一 core。

- [x] `AppState`（db + LlmService + 结构化 Config）启动时构建；Tauri State 与 Axum State 共用同一结构
- [x] DB 初始化：桌面端用应用数据目录下的 SQLite；`sync_schema` 收敛到唯一调用点
- [x] tracing 初始化（两壳各自 init subscriber，core 只打点）
- [x] `CoreError` → 稳定 error code 映射（作为 DTO 契约的一部分）
- [x] Tauri commands + 最小 Axum server：渠道 / 凭证 / 路由规则 CRUD，两侧都只做薄适配
- [x] `LlmClient` 补非流式请求总超时（流式仅 connect timeout）

验收：`pnpm tauri dev` 内可调 CRUD；`curl` 对 Axum 可调同一组接口；门禁全绿。

## 阶段 2：契约与前端数据层

- [x] ts-rs：给边界 DTO 挂 `#[derive(TS)]`，生成 `frontend/src/lib/bindings/`；固化生成命令（如 `pnpm gen:bindings`）
- [x] 前端 API 抽象层：运行环境探测，Tauri 走 invoke、浏览器走 fetch；错误按 error code 统一处理

验收：前端能通过抽象层列出并创建渠道（临时页面即可）。

## 阶段 3：渠道管理 UI

目标：第一块真实界面，同时落地欠着的前端基建。

- [x] i18n 引入（Paraglide；中文默认 + fallback），首批词条，AGENTS.md 回填方案
- [x] 主题系统：Tailwind `dark:`、跟随系统 + 手动切换、首屏无闪烁、favicon 亮暗切换
- [x] 渠道 / 凭证 / 路由矩阵管理页面：表单验证、loading/empty/error 状态、移动端可用
- [x] Chrome DevTools 直观验收：三档断点、亮暗两模式、console 干净

验收：能在 UI 里完整配置一个真实渠道（含凭证与路由规则）。

## 阶段 4：LLM 通路端到端

目标：把字节流升格为结构化事件流，打通生成链路。

- [x] core 定义传输无关的流式事件 enum（ChatEvent 一类），替代裸 SSE 字节透传给上层
- [x] Tauri 壳映射 Channel、Axum 壳映射 SSE——同一事件流，两壳只做映射
- [x] 取消传播（客户端断开 → 上游请求中止）与统一超时语义
- [x] `local` 路由实现落地：count_tokens 走 gproxy-tokenize 本地阶梯
- [ ] 真渠道冒烟：非流式、流式、failover 三条路径

验收：从 UI 发一条消息拿到流式回复（临时对话框即可）。

## 阶段 5：Roleplay 领域

目标：应用主体。

详细设计与分步提交以 `dev_docs/roleplay-assets-plan.md` 为准；协议实现缺口见
`dev_docs/protocol-gap-audit.md`。

- [ ] 实体：角色卡（character）、用户 persona、会话（conversation）、消息（message）——entity-first 直接加表
- [ ] 提示词组装：角色卡 + persona + 历史；token 预算（count + compact 能力）
- [ ] 聊天 UI：流式渲染、markdown、消息重试；编辑 / 分支可后置
- [ ] 生图 / 图编辑能力接入 UI（角色头像、场景图）

验收：完整走一轮角色扮演对话并持久化，重启后可继续。

## 阶段 6：打磨与发布

- [ ] 音频能力：协议层已有 speech / transcription / translation 操作，按路由矩阵接入
- [ ] Axum 侧多用户与部署考量：鉴权、多实例约束复查（跨实例状态、并发写）
- [ ] 打包发布：`pnpm tauri build` 桌面产物；server 端部署方式
- [ ] 性能与可观测性复查：span 覆盖关键链路、慢查询、前端 CLS / 首屏
