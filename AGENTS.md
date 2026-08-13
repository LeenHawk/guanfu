# AGENTS.md — 观复

角色扮演（Role Play）应用。桌面端以 Tauri 发布，Web/服务端以 Axum 发布，两者共享同一套 Rust 业务核心。

## 技术栈

- 前端：SvelteKit（Svelte 5）+ TypeScript + Vite + Tailwind CSS 4，adapter-static
- 后端：Rust（Cargo workspace）
- 数据库：SeaORM 2（实体、迁移、查询均走 SeaORM）
- 桌面壳：Tauri 2
- 服务端壳：Axum
- JS 包管理：pnpm workspace（统一用 pnpm，不用 npm/yarn）

## 架构原则

业务逻辑只写一次，放在共享 core crate 中；Tauri 和 Axum 只是两个薄适配层。

- `backend/core`：领域模型、服务层、SeaORM 实体，不依赖 tauri 或 axum
- `backend/tauri`：Tauri commands，只做参数解析 → 调用 core → 返回结果
- `backend/server`：Axum handlers，同样只做 HTTP 适配
- 交互接口（请求/响应类型、服务方法签名）定义在 core，与传输层无关：不得出现 tauri 或 axum 的类型，序列化/路由由两个壳各自适配
- 前后端交互类型用 ts-rs 从 core 的 DTO 生成（`#[derive(TS)]`）：TypeScript 产物放 `frontend/src/lib/bindings/`，生成文件不手改，前端接口类型只从这里导入
- 前端通过统一的 API 抽象层调用后端：Tauri 环境走 invoke，浏览器环境走 HTTP；新增接口默认两侧适配层都要接上，确属单端的功能（仅本地侧或仅云端侧才有意义）可只实现一端，并在接口定义处注明

## 目录结构

```
frontend/          SvelteKit 前端（pnpm workspace 成员）
backend/           Rust Cargo workspace
backend/tauri/     Tauri 壳（command 适配层）
backend/core/      共享业务核心（SeaORM）
backend/server/    Axum 服务端
```

## 常用命令

pnpm 命令均在仓库根目录执行（脚本代理到 frontend）：

- `pnpm install` — 安装依赖
- `pnpm dev` — 前端开发服务器
- `pnpm tauri dev` — 桌面端开发
- `pnpm check` — svelte-check 类型检查
- `pnpm lint` — prettier 格式检查 + eslint
- `pnpm format` — prettier 自动格式化
- `pnpm build` / `pnpm tauri build` — 构建
- `cargo check` / `cargo clippy` / `cargo test` — Rust 侧，在 `backend/` 下执行

## 前端约束

- 组件化：页面组件只做组合与数据编排，可复用 UI 拆成独立组件（行数上限见「代码约定」）
- 设计 token：颜色 / 间距 / 圆角 / 阴影 / 字级用 Tailwind theme token，组件里不散落裸色值和 magic number
- 状态完整性：异步页面覆盖 loading / empty / error / success；提交中按钮 disabled 防重复；错误要展示给用户，不许只 console
- 表单：统一验证方式；客户端验证只为体验、不可信任；错误提示挂在对应字段且可被辅助技术识别
- 交互一致性：Modal / Toast / Confirm / Dropdown / 分页 / 搜索等同类交互用统一组件，不各页各写一套
- i18n：文案全走词条不硬编码、不拼接句子；日期 / 数字 / 货币按 locale 格式化；定义默认语言与 fallback（方案首次涉及文案时选型引入并更新本文件）
- 亮暗模式：跟随系统 + 可手动切换；主题差异只走 CSS / Tailwind `dark:`，禁止 JS 维护两套颜色；首屏不闪主题
- 响应式：桌面 / 平板 / 手机三档可用（Tailwind 断点）；表格、弹窗、导航要有明确移动端策略，触控目标 ≥ 44px；考虑超长文本 / URL / i18n 文案扩张的溢出，不依赖固定文案长度撑布局
- 无障碍：语义化 HTML、键盘可操作、alt / label 齐全，不压制 Svelte 的 a11y 警告；关键控件保持稳定语义（同时服务测试）；动画只为反馈服务并支持 prefers-reduced-motion
- 数据层：API 调用集中管理，组件不散落 fetch；类型来自 bindings、避免 any，外部数据在边界处校验；可分享 / 刷新的状态（筛选、分页、tab）放 URL 而不是组件 state
- SSR 安全：不在模块顶层无条件访问 window / document / localStorage
- 安全：避免 `{@html}`，确需使用时内容必须经可信 sanitize；token 等敏感信息不进前端日志与持久化存储
- 性能：重组件按需加载，长列表分页或虚拟化，图片标明尺寸防 CLS；优先标准 Web API，不为小功能引入大依赖或 polyfill

## 后端约束

- 数据库层用 SeaORM 2，按 2.0 的写法而不是 1.x 老风格，优先利用新特性：
  - dense 实体格式（`sea-orm-cli generate entity --entity-format dense`），关系作为类型化字段写在 Model 上（如 `BelongsTo<Entity>`，可空性编码进类型）
  - 嵌套 ActiveModel：对象树一次事务保存，外键顺序交给 SeaORM
  - 唯一键自动生成的类型安全捷径 `find_by_xxx()` / `filter_by_xxx()`，少手写 filter 链
  - 需要裸 SQL 时用 `raw_sql!` 宏（参数插值防注入），不手拼 SQL 字符串
  - 批量插入用 2.0 重做的 `insert_many` API
- schema 演进：entity-first，Model 是 schema 的事实来源；开发期新增表 / 字段 / 索引走 schema sync（只增不删）；数据搬迁或 sync 无法安全表达的破坏性变更允许显式 migration，禁止在运行时业务代码里偷偷修数据库；sync 的调用位置统一管理，业务模块不得自行执行
- 多实例：从一开始按「多个观复实例共享同一数据库」设计——跨请求 / 跨实例的状态存库不存进程内存；并发写用原子更新、唯一约束、乐观锁，避免 read-modify-write 竞态；不假设单写者
- 事务：一个业务操作的多次相关写入放同一事务；数据层函数接收 `&impl ConnectionTrait`，同一份代码兼容普通连接与事务；事务内不等待 LLM / 文件等长外部 IO，跨外部调用的流程状态显式建模（pending / running / failed）
- 错误处理：core 用结构化错误（thiserror），业务代码不返回 Axum / Tauri 错误类型；对前端只暴露稳定 error code（+ 可选 details），DbErr / reqwest / anyhow 的原始字符串只进日志；用户可见文案由前端按 code 走 i18n；anyhow 仅限应用入口与启动流程
- LLM 协议层用 gproxy-protocol / gproxy-transform / gproxy-tokenize，出站 HTTP 用 reqwest
- 遇到疑似 gproxy 协议层 bug：不要在本仓库悄悄绕过，向用户提出，并到 https://github.com/LeenHawk/gproxy 提 issue
- LLM 流式：生成、流式解析、工具调用编排只在 core 实现；core 对外暴露传输无关的流式事件 enum（如 ChatEvent），Tauri 壳映射为 Channel、Axum 壳映射为 SSE——不许两个壳各写一套生成逻辑，层间不传未定义的 JSON Value / 字符串协议
- 取消与超时：客户端断开 / 取消要向下传播，不让上游模型调用继续空跑；所有外部网络请求必须有明确 timeout，禁止无限等待
- 共享资源：DatabaseConnection、reqwest::Client、注册表等长生命周期资源在启动时创建，经 AppState 显式注入（Tauri State 与 Axum State 共用同一结构）；不在每次调用重建 reqwest::Client，core 不通过全局变量取资源
- 并发：async 代码不直接做明显阻塞的 CPU / 同步 IO（确需则 spawn_blocking）；锁 guard 不跨 `.await` 长持；锁落在最小共享对象上，不把整个 AppState 包进大 Mutex
- DTO 与实体：SeaORM Model 是持久化模型，不直接当 API DTO；ts-rs 只生成跨前后端边界的 DTO，不把数据库内部类型全暴露给前端；简单 CRUD 直接转换即可，不为形式建 repository / domain 层
- 配置：启动阶段集中读取并以结构化 Config 注入，core 不自行读环境变量；两个壳可有不同配置来源，进入 core 后统一类型
- 日志：统一 tracing（正式日志不用 println!/eprintln!）；请求、LLM 生成、tool call 等关键流程建 span 并携带 request_id / conversation_id 等关联字段；不记录 API key 等凭证，不默认记录完整 prompt / response（调试内容走显式开关）

## 提交前检查

- Rust 改动：`cargo fmt`、`cargo check`、`cargo clippy` 必须全部通过，clippy 不留警告
- 前端改动：`pnpm check`、`pnpm lint` 必须全部通过
- 前端 UI 改动：还需用浏览器（Chrome DevTools MCP）打开实际页面直观评估——布局与交互正常、三档断点可用、亮暗两种模式正常、console 无报错

## 代码约定

- 单个源文件最好不超过 200 行，超过 500 行必须拆分（生成代码如 SeaORM entity 除外）
- 前后端都尽可能模块化：按功能域拆分，模块边界清晰、低耦合
- 需要较大规模重构时，先和用户沟通确认再动手
- 不过度设计、不过度防御、不过度测试：只为当前需求服务
- Rust：遵循 rustfmt 与 clippy 默认规则
- Svelte：使用 Svelte 5 runes（`$state`、`$derived`、`$props`）
- 各子目录可有自己的 AGENTS.md 细则，改动对应区域时先读它
