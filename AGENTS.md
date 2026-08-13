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

- i18n：所有用户可见文案走 i18n 词条，不硬编码（i18n 方案在首次涉及文案时选型引入，并更新本文件）
- 亮暗模式：全站支持亮/暗主题（默认跟随系统，可手动切换），用 Tailwind 的 dark variant 实现；新 UI 两种模式都要可用
- 响应式：桌面 / 平板 / 手机三档都要可用，优先用 Tailwind 断点实现
- 无障碍：优先语义化 HTML；交互元素可键盘操作，图片有 alt，表单控件有 label；不要压制 Svelte 编译器的 a11y 警告

## 后端约束

- 数据库层用 SeaORM 2，按 2.0 的写法而不是 1.x 老风格，优先利用新特性：
  - dense 实体格式（`sea-orm-cli generate entity --entity-format dense`），关系作为类型化字段写在 Model 上（如 `BelongsTo<Entity>`，可空性编码进类型）
  - 嵌套 ActiveModel：对象树一次事务保存，外键顺序交给 SeaORM
  - 唯一键自动生成的类型安全捷径 `find_by_xxx()` / `filter_by_xxx()`，少手写 filter 链
  - 需要裸 SQL 时用 `raw_sql!` 宏（参数插值防注入），不手拼 SQL 字符串
  - 批量插入用 2.0 重做的 `insert_many` API
- schema 演进：entity-first——写 Model → sync 生成 schema，不手写 migration 文件
- LLM 协议层用 gproxy-protocol / gproxy-transform / gproxy-tokenize（本项目用户是其作者），出站 HTTP 用 reqwest
- 遇到疑似 gproxy 协议层 bug：不要在本仓库悄悄绕过，向用户提出，并到 https://github.com/LeenHawk/gproxy 提 issue

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
