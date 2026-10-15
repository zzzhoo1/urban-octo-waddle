# CodeArts Gateway v2

OpenAI 兼容多账号 API 网关 + Web 管理面板。

- **后端**：纯 Rust（axum），从零重写，行为与线上 Go 版对齐
- **前端**：Next.js 15（App Router）· React 19 · TypeScript · shadcn/ui · Tailwind CSS v4 · motion · recharts · sonner
- **交付物**：单一 Linux x86_64 静态二进制（musl），前端静态产物通过 rust-embed 内嵌，无需 Node 运行时
- **构建**：全部在 GitHub Actions 完成，服务器不安装任何编译工具链

> 完整的功能与协议规格见 [SPEC.md](./SPEC.md)。

## 功能

### 数据面（/v1，OpenAI 兼容）

- `POST /v1/chat/completions`：流式 SSE 透传 + 非流式（兼容上游 SSE 响应，三级解析）
- `GET /v1/models`：档位别名（auto / fast-model / balanced-model / primary-model / deep-model）+ 上游动态模型清单（10 分钟缓存，黑名单过滤，兜底列表）
- 账号池轮询、三态熔断（closed/open/half-open）、最多 3 次线性退避重试、会话粘性（10 分钟 TTL）
- 统一 OpenAI 错误格式，请求体上限 8 MiB

### 管理面（/admin）

- 仪表盘：账号健康总览、状态分布图、请求延迟趋势
- 账号管理：列表 / 手动录入（AES-256-GCM 加密入库）/ 启停 / 删除 / OAuth 入口
- 模型中心：档位别名 → 实际模型映射展示
- 调用日志：最近 500 次请求（内存环形缓冲）
- 系统设置：运行配置只读展示
- 登录：网关密钥（gw-key）校验，Cookie 保持会话

### 上游（华为云 CodeArts MaaS）

- SDK-HMAC-SHA256 AK/SK 签名（与官方 CLI v26.9.3 逐字节对齐）
- MaaS 会话自动创建与 50 分钟复用
- 工具调用时自动注入 XML 格式提示
- 账号凭据 AES-256-GCM 加密存储，accounts.json 与旧版格式完全兼容

## 构建（GitHub Actions）

推送 `main` 分支或任意 `v*` tag 即触发流水线：

1. **frontend**：Node 20 安装依赖并 `next build`，产出静态导出 `web/out`
2. **backend**：下载前端产物后 `cargo build --release --target x86_64-unknown-linux-musl`，二进制内嵌前端
3. **release**（仅 tag）：打包 `codearts-gateway-v2-linux-x86_64.tar.gz`（含 sha256）并创建 GitHub Release

## 部署

```bash
# 从 Release 下载并校验
curl -fsSL -o gw.tar.gz <release-url>/codearts-gateway-v2-linux-x86_64.tar.gz
tar -xzf gw.tar.gz
sha256sum -c codearts-gateway.sha256

# 启动（参数与旧版一致）
./codearts-gateway \
  --listen 127.0.0.1:16211 \
  --secret <encryption-secret> \
  --store /opt/codearts-gw/accounts.json \
  --codearts-ak <AK> \
  --codearts-sk <SK> \
  --gw-key <gateway-key>
```

管理面板：`http://<host>:<port>/admin/`（根路径自动重定向）。

## 本地开发

```bash
# 前端（热更新，端口 3000）
cd web && npm install && npm run dev

# 后端（需要预置 web/out 供 rust-embed 内嵌）
cd web && npm run build && cd ..
cargo run -- --listen 127.0.0.1:8080 --secret dev-secret \
  --codearts-ak <AK> --codearts-sk <SK> --gw-key dev-key
```

## 目录结构

```
├── src/                  # Rust 后端
│   ├── main.rs           # 入口与装配
│   ├── gateway.rs        # 调度核心（选号/熔断/重试/粘性）
│   ├── upstream/         # 华为云签名与 MaaS 客户端
│   ├── account.rs        # 账号池
│   ├── circuit.rs        # 熔断器
│   ├── routes.rs         # /v1 数据面
│   ├── admin.rs          # /admin/api 管理面
│   └── ...
├── web/                  # Next.js 15 前端
│   ├── app/              # 仪表盘/账号/模型/日志/设置/登录
│   ├── components/       # shadcn/ui 组件与应用外壳
│   └── lib/              # API 客户端与工具
├── .github/workflows/    # CI 构建与发布
└── SPEC.md               # 逆向提炼的完整规格说明
```
