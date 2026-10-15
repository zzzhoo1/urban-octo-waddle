# CodeArts 网关重构规格说明（非代码）

> 本规格从线上运行的 Go 版网关逆向提炼，并参考 workbuddy-manager 与 Antigravity-Manager 的功能设计。
> 目标实现：纯 Rust 后端（axum）+ Next.js 15 前端（静态导出，由后端内嵌托管）。
> 构建方式：全部在 GitHub Actions 完成，服务器不安装任何编译工具链。

---

## 1. 系统定位

OpenAI 兼容 API 网关 + Web 管理面板。对下游客户端暴露标准 OpenAI 接口；对上游接入华为云 CodeArts MaaS（码道）生产链路，使用 AK/SK 签名直连。

- 数据面：`/v1/*`（OpenAI 兼容）
- 管理面：`/admin/api/*`（管理接口）+ `/admin/*`（静态前端页面）
- 根路径 `/` 302 重定向到 `/admin/`
- 管理面与数据面共用同一把网关密钥（gw-key）

## 2. 数据面 API 规格

### 2.1 POST /v1/chat/completions
- 请求体：标准 OpenAI ChatCompletionRequest（model/messages/stream/temperature/max_tokens/user/tools/tool_choice/reasoning_effort）
- messages[].content 须兼容字符串与多模态数组两种格式（数组时提取 type=text 片段拼接）
- 鉴权：`Authorization: Bearer <gw-key>` 或 `X-Api-Key`
- 会话粘性：优先取请求头 `X-Session-ID`，缺省取 body 的 `user` 字段；同一会话绑定同一账号
- stream=true：返回 `text/event-stream`，上游 SSE 原样透传
- stream=false：返回完整 JSON；上游即便在非流式时也可能返回 SSE，需按『直接 JSON → 剥离 data: 前缀 → SSE 聚合』三级解析
- 请求体上限 8 MiB，超限返回 413 + OpenAI 错误格式

### 2.2 GET /v1/models
- 返回 = 档位别名列表（owned_by=codearts-gateway）+ 上游动态模型列表（owned_by=huaweicloud-maas）

### 2.3 POST /v1/embeddings
- 上游不支持，返回 502 错误（codearts upstream does not support embeddings）

### 2.4 错误格式
统一 OpenAI 错误信封：`{"error":{"message","type","code"}}`
状态码映射：unauthorized=401 / bad_request=400 / rate_limited=429 / unavailable=503 / bad_gateway=502 / internal=500

## 3. 管理面 API 规格

### 3.1 鉴权
- `Authorization: Bearer <gw-key>` 或 Cookie `gw_key=<key>`，二者任一
- 静态页面不鉴权（登录页可访问）；仅 `/admin/api/*` 鉴权（与 Go 版的差异点，属有意改进）

### 3.2 接口清单
| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /admin/api/accounts | 账号列表（凭据脱敏，不返回 credential 明文） |
| POST | /admin/api/accounts | 新建账号 {name, credential}，凭据 AES-256-GCM 加密后入库，返回 201 |
| PATCH | /admin/api/accounts/{id} | {status: active\|disabled} 启用/禁用 |
| DELETE | /admin/api/accounts/{id} | 删除，返回 204 |
| GET | /admin/api/status | 运行状态 [{id,name,status,healthy,failure_count}] |
| POST | /admin/api/oauth/start | 返回 {auth_url, state}；未配置 OAuth 返回 501 |
| GET | /admin/api/oauth/callback | code+state 换 token，加密入库建账号，返回 201 |
| GET | /admin/api/logs | 【v2 新增】最近请求日志环形缓冲（最多 500 条）：{ts,method,path,model,status,latency_ms} |
| GET | /admin/api/info | 【v2 新增】网关信息：版本、监听地址、熔断/重试/会话等配置值 |

### 3.3 账号 DTO
{id, name, status(active|disabled|cooling), failure_count, last_failure_at, created_at}

## 4. 核心运行时行为

### 4.1 账号池
- 状态机：active / disabled / cooling
- 选号：仅在健康子集（active 且非冷却）内轮询（round-robin 游标）
- 每次选号前先执行 RecoverCooling：冷却到期（默认 1 分钟）自动恢复 active
- MarkFailure：连续失败达阈值（默认 20）转入 cooling；成功则清零失败计数

### 4.2 熔断器（每账号一个）
- 三态：closed → open → half_open
- closed：连续失败达阈值（默认 20）切 open
- open：拒绝请求；冷却期（默认 30 秒）过后转 half_open 并放行唯一探测请求
- half_open：探测成功回 closed；探测失败立即回 open；探测进行中拒绝其余请求

### 4.3 重试
- 最多重试 3 次；线性退避 = (attempt+1) × 500ms；等待期间响应客户端取消
- 仅对可重试错误重试（5xx/网络错误）；不可重试错误直接返回

### 4.4 会话粘性
- 内存会话表：sessionID → accountID，TTL 10 分钟，容量 1000，超限逐出最旧
- 流式请求成功后绑定会话

### 4.5 请求日志
- 每个 /v1 请求记录：时间、方法、路径、模型、状态码、耗时；环形缓冲 500 条
- 终端日志对 Authorization 头脱敏（Bearer ***）

## 5. 上游协议规格（华为云 CodeArts MaaS）

### 5.1 端点
- 生产网关：`https://snap-access.cn-north-4.myhuaweicloud.com`
- 聊天：`POST {gateway}/api/v2/chat/completions`
- 会话：`POST {gateway}/v1/sessions`
- 模型统计：`GET {gateway}/snap-manager/v1/statistics/plugin`

### 5.2 签名（SDK-HMAC-SHA256，逆向自官方 CLI v26.9.3）
- 日期头：`X-Sdk-Date`（UTC，格式 YYYYMMDDTHHMMSSZ）
- 有 body 时设置 `X-Sdk-Content-Sha256` = sha256hex(body)；空 body 用固定哈希 e3b0c442...52b855
- SignedHeaders：除 authorization 外全部请求头小写排序，分号连接
- CanonicalURI：按 / 分段，逐段 urlEncode（保留 A-Za-z0-9-_.~+），末尾强制补 /
- CanonicalQuery：key 排序后 key=value 逐个 QueryEscape 用 & 连接
- CanonicalRequest = method\nuri\nquery\nheaders\nsignedHeaders\npayloadHash
- StringToSign = SDK-HMAC-SHA256\ndate\nsha256hex(CanonicalRequest)
- Signature = hex(hmacSHA256(SK, StringToSign))
- Authorization = `SDK-HMAC-SHA256 Access=<AK>, SignedHeaders=<...>, Signature=<...>`
- 注意：Host 头必须显式参与签名

### 5.3 MaaS 会话
- 聊天前必须持有会话：POST /v1/sessions，请求头 Agent-Type: PromptCenter、X-Language: zh-cn，body 含 client_type/device_name/device_type/ip_address/client_version
- 响应取 data.id；缓存复用 50 分钟（互斥保护，并发上限 3 个会话）
- 聊天请求携带 x-ot-session-id 与 user-session-id 两个头

### 5.4 模型清单
- 权威来源：statistics/plugin 的 model_metrics；仅收 enable=true 且 status=normal 的模型
- 缓存 10 分钟
- 黑名单（实测不可用，必须过滤）：Qwen3-VL-235B（静默降级成 kimi-k2.6）、openpangu-2.0-pro（502）
- 兜底列表：GLM-5.2、glm-5.2-sft-harmony、openpangu-2.0-flash、kimi-k2.6-vl

### 5.5 档位别名
| 档位名 | 实际模型 | 默认 reasoning_effort |
|---|---|---|
| auto | GLM-5.2 | - |
| fast-model | openpangu-2.0-flash | - |
| balanced-model | openpangu-2.0-pro | - |
| primary-model | GLM-5.2 | - |
| deep-model | GLM-5.2 | max |
客户端显式指定 reasoning_effort 时优先客户端值。

### 5.6 工具调用格式注入
请求包含 tools 时，向 system 消息追加 XML 格式提示（无 system 则在开头插入），要求模型以 `<tool name><param name>` XML 输出工具调用，禁止 JSON 与 markdown 代码块。

## 6. 安全与存储

- 凭据加密：AES-256-GCM；密钥 = SHA256(secret)；随机 nonce 前置拼接密文后 base64
- 账号存储：JSON 文件；写入采用 tmp 文件 + rename 原子替换；文件权限 0600
- 密钥管理：网关密钥支持运行时增删（内存集合）

## 7. 前端页面规格（Next.js 15，静态导出，basePath=/admin）

技术栈：App Router · React 19 · TypeScript · shadcn/ui · Tailwind CSS v4 · motion · recharts · sonner

| 页面 | 路由 | 功能 |
|---|---|---|
| 登录 | /login | 输入网关密钥，写 gw_key Cookie 后跳转首页 |
| 仪表盘 | / | 账号健康总览卡片（总数/健康/冷却/禁用）、账号状态分布图（recharts）、最近请求趋势 |
| 账号管理 | /accounts | 列表展示（状态徽章、失败计数、创建时间）、手动录入（名称+凭据对话框）、启用/禁用开关、删除确认、OAuth 扫码入口（501 时 toast 提示） |
| 模型中心 | /models | 档位别名与上游模型分组展示、别名→实际模型映射说明 |
| 调用日志 | /logs | 最近请求表格（时间/方法/模型/状态码着色/耗时），手动刷新 |
| 设置 | /settings | 网关配置只读展示（版本、熔断阈值、重试、会话 TTL 等）、部署说明 |

交互约定：全部操作结果用 sonner toast 反馈；页面加载用骨架屏；动效用 motion（卡片入场淡入上移）。

## 8. 构建与部署

- GitHub Actions：前端（Node 20，`npm install && npm run build`，静态导出 web/out）→ 后端（Rust stable，target x86_64-unknown-linux-musl，rust-embed 内嵌 web/out）→ 产物打包 codearts-gateway-v2-linux-x86_64.tar.gz（含 sha256）→ tag 推送时自动创建 Release
- 部署：下载产物 → 解压替换服务器二进制 → systemctl 重启；启动参数与 Go 版保持一致（--listen/--secret/--store/--codearts-ak/--codearts-sk/--gw-key）
- 默认运行参数：CircuitThreshold=20, CircuitCooldown=30s, SessionTTL=10min, SessionMaxSize=1000, FailureThreshold=20, RetryMax=3, RetryBackoff=500ms, AccountRecoverAfter=1min, DefaultModel=codearts-pro
