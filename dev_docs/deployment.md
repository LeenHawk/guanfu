# 部署与打包

观复有两种形态:桌面壳(Tauri)和服务端(Axum)。两者共用同一个 core,
差别只在传输层与运行约束。

## 桌面(Tauri)

```bash
pnpm install
pnpm tauri build          # 产物在 backend/target/release/bundle/
```

- 数据库与资产目录落在系统的应用数据目录(`app_data_dir()`),
  分别是 `guanfu.db` 与 `assets/`。
- 桌面壳是本地单用户进程,不做鉴权;媒体内容以 data URL 交给 webview,
  因为桌面端没有可引用的 HTTP 端点。

## 服务端(Axum)

```bash
cargo build --release -p guanfu-server
GUANFU_TOKEN=... GUANFU_ADDRESS=0.0.0.0:3000 \
  DATABASE_URL='sqlite://guanfu.db?mode=rwc' \
  GUANFU_ASSET_ROOT=/var/lib/guanfu/assets \
  ./target/release/guanfu-server
```

| 环境变量 | 缺省 | 说明 |
| --- | --- | --- |
| `DATABASE_URL` | `sqlite://guanfu.db?mode=rwc` | 数据库连接串 |
| `GUANFU_ADDRESS` | `127.0.0.1:3000` | 监听地址 |
| `GUANFU_ASSET_ROOT` | `guanfu-assets` | 二进制资产目录 |
| `GUANFU_TOKEN` | 无 | 共享访问令牌 |
| `RUST_LOG` | 无 | 日志过滤(如 `guanfu_core=debug`) |

前端静态产物由 `pnpm --filter frontend build` 生成到 `frontend/build/`,
交给任意静态服务器,并把 `/api` 反向代理到 guanfu-server。

反向代理**必须放行 WebSocket 升级**:`/api/realtime` 是双工通道,
只转发普通 HTTP 会让实时语音停在"连接中"而不报错(nginx 需要
`proxy_set_header Upgrade $http_upgrade;` 与 `proxy_set_header Connection "upgrade";`)。

**令牌不是可选项**:API 能读写渠道凭证,拿到 API 就等于拿到上游密钥的
使用权。未设置 `GUANFU_TOKEN` 时进程拒绝绑定非回环地址。令牌是共享秘密,
不是用户身份——持有者共用同一份资产与渠道,逐用户隔离需要真正的账号体系。

## 多实例约束

数据库侧可以多实例并行:

- Asset 修订不可改写,推进头指针走 `(id, head_revision)` CAS,
  抢输的一方拿到结构化冲突后重读重试。
- chunk 按内容哈希幂等写入,重复写是无操作。
- 凭证的失败计数与冷却存在数据库里,实例之间自然共享。
- `LlmService` 的轮换计数器是进程内的,只影响每个实例从哪个凭证开始试,
  不影响正确性。

**存储是唯一的硬约束**:`LocalAssetStore` 写本机目录,多实例各写各的,
一个实例存的图另一个读不到。多实例部署要么共用网络文件系统,要么等
`AssetStore` 的 S3 实现(trait 已就位,实现待补)。

SQLite 同样是单机方案;多实例应换成独立数据库服务。

## 可观测性

- 关键链路有 span:`LlmService::execute`(带 operation)、
  `RunnerService::run_chat`(带 pipeline / channel / run_id)、
  每个图节点(带 node id)、媒体操作与 Asset 提交。
- SQL 语句日志在 trace 级(默认不出);**超过 200ms 的语句按 warn 单独报**,
  所以慢查询不需要临时打开全量 SQL 日志就能在正常日志里看到。
