# 后端实现提示词（AI 网关服务端 · 逆向提炼自 Antigravity-Manager 与 workbuddy-manager）

> 使用方式：把本提示词整体喂给大模型，要求按规格生成完整后端实现。
> 技术栈匹配：后端实现应自动匹配或建议现代化技术栈与框架，示例包含但不限于：
> - Rust（axum + tokio + reqwest + serde）
> - Go（Gin/Fiber + goroutines + net/http + encoding/json）
> - Node.js（Fastify/Express + Node 18+ + undici / fetch）
> - Python（FastAPI + asyncio + httpx）
> 如需替换为其他语言或框架，仅替换对应框架名称，功能规格不变。
>
> 技术栈自动匹配策略（可被生成器/实现器使用）：
> - 目标维度：按部署目标、性能需求、二进制体积、开发速度、团队熟悉度与生态要求综合评分。
> - 指标与权重（示例，生成器可按需求调整）：
>   - 部署约束（静态二进制、musl、容器体积优先）: 25%
>   - 延迟敏感度（P95 延迟目标）: 20%
>   - 吞吐与并发（并发连接/请求处理）: 15%
>   - 开发/交付速度（语言生态与第三方库）: 15%
>   - 可观测性与运维（Prometheus、日志、健康检查）: 10%
>   - 团队熟悉度（由用户输入偏好）: 15%
> - 简单决策规则示例：
>   1. 若“部署约束”高（需要单静态二进制、无外部运行时），优先推荐 Rust 或 Go（Rust 更小体积与性能，Go 开发更快）。
>   2. 若“延迟敏感度”高且需极限性能/资源受限，优先推荐 Rust。
>   3. 若“开发速度”或“生态”权重高（快速原型、丰富中间件、动态扩展），优先推荐 Node.js 或 Python（FastAPI）。
>   4. 若团队熟悉度对某语言偏好高，则在分值接近时以团队熟悉度为决策杆。
>   5. 若需轻量运维与快速交付（容器化、K8s），Go/Node.js 通常是折中优选。
> - 输出格式：生成器在选择栈时应返回 rationale（得分明细）与最终推荐（主/备），以及需要的框架/库清单与运行/编译建议���
>
> 固定评分表与优先级表（建议写入生成器以降低随机性）：
> - 输入：用户向生成器提供六项评分（每项 0-10）：deployment_constraint, latency_sensitivity, concurrency_need, dev_speed, observability, team_familiarity。
> - 计算：最终得分 = sum(weight_i * normalized_input_i)，权重按上表。生成器应输出每个候选栈的得分并排序。
> - 候选栈基线能力（示例评分 0-10）：
>   - Rust: deployment=9, latency=10, concurrency=9, dev_speed=4, observability=8, ecosystem=7
>   - Go: deployment=8, latency=8, concurrency=8, dev_speed=7, observability=8, ecosystem=7
>   - Node.js: deployment=4, latency=6, concurrency=7, dev_speed=9, observability=7, ecosystem=9
>   - Python(FastAPI): deployment=4, latency=5, concurrency=6, dev_speed=9, observability=7, ecosystem=8
> - 决策阈值示例：
>   - 若最高得分 >= 8.0 → 强烈推荐（primary）。
>   - 若最高得分在 [6.0, 8.0) → 推荐（primary），并给出备选（secondary）。
>   - 若最高得分 < 6.0 → 推荐两种用于实验的栈（secondary 为可选生产候选）。
> - 平局与靠近情况处理：若前两名分差 < 0.2，使用 team_familiarity 作为 tiebreaker；若仍然平局，输出两种主/备并标注“用户选择优先”。
>
> 输出例子（生成器应返回 JSON 对象）：
> {
>   "primary": "Rust",
>   "secondary": "Go",
>   "scores": {"Rust": 8.6, "Go": 8.2, "Node.js": 6.9, "Python": 6.7},
>   "rationale": "Rust 得分高因部署约束与延迟敏感度权重大，团队熟悉度适中。",
>   "compile_run_notes": "musl 编译、静态链接、交叉编译建议 (x86_64 -> ARM64)、建议使用 Tokio + axum + reqwest。"
> }
>
> 最终推荐写法：生成器必须同时输出（1）数值型得分，（2）选择理由短文，（3）主/备推荐与依赖列表，以及（4）编译/运行/部署要点。

---

## 角色

你是一名资深后端架构师。请按照本规格实现一个生产可用的『AI 账号池代理网关』后端服务。该网关把多个上游 AI 账号（OAuth Token 或 AK/SK 凭据）��[...]

---

## 一、核心数据面（/v1，OpenAI 兼容）

### 1.1 接口清单
| 方法 | 路径 | 说明 |
|---|---|---|
| POST | /v1/chat/completions | 聊天补全，支持 stream=true（SSE 透传）与非流式 |
| GET | /v1/models | 模型清单 = 别名层 + 上游动态模型 |
| POST | /v1/embeddings | 嵌入（上游不支持时返回 502 并说明） |
| POST | /v1/messages | 【扩展】Anthropic Messages 协议适配，让 Claude Code CLI 可直连 |

### 1.2 请求处理流水线（严格按序）
1. **鉴权**：从 `Authorization: Bearer <key>` 或 `X-Api-Key` 提取密钥；密钥以 SHA-256 哈希落库，校验时比对哈希。每把密钥独立校验：启用状态、过期时间、T[...]
2. **路由解析**：把客户端请求的模型名解析为实际上游模型：自定义别名（精确匹配优先）→ 正则规则 → 内置档位表 → 原样透传。命中后台任务�[...]
3. **账号选择**：在健康账号子集内轮询（round-robin）。选择前先执行冷却恢复（冷却到期的账号自动恢复 active）。支持会话粘性：`X-Session-ID` 头或 bo[...]
4. **熔断检查**：每账号一个三态熔断器（closed/open/half-open）。closed 连续失败达阈值（默认 20）切 open；open 冷却期（默认 30 秒）过后放行唯一探测请[...]
5. **上游调用 + 重试**：失败时按『可重试错误』（5xx/429/网络错误/401）换账号重试，最多 3 次，线性退避 (attempt+1)×500ms，等待期间响应客户端取消��[...]
6. **用量记账**：请求完成后按密钥累计 token 消耗（用于配额判定）。

### 1.3 流式响应（SSE）处理
- 上游 SSE 原样透传给客户端，不缓冲、不改写。
- **旁路审计**（不干扰数据流）：记录首字延迟（从请求发起到第一帧到达的毫秒数）；从结束帧提取 token 用量、实际扣费（usage.credit）、提示词缓[...]
- 非流式请求：上游即便在非流式也可能返回 SSE 体，需按『直接 JSON → 剥离 data: 前缀 → SSE 聚合』三级解析。

### 1.4 错误格式
统一 OpenAI 错误信封 `{"error":{"message","type","code"}}`；状态码映射：unauthorized=401 / bad_request=400 / rate_limited=429 / unavailable=503 / bad_gateway=502 / internal=500。请�[...]

---

## 二、账号池管理

- 账号字段：id、名称、凭据（AES-256-GCM 加密存储，密钥=SHA256(secret)，nonce 前置拼接密文后 base64）、状态（active/disabled/cooling）、连续失败计数、最近[...]
- 状态机：active 正常参与调度；disabled 不参与调度但签到/保活等任务照常；cooling 由失败阈值触发，冷却到期自动恢复。
- 账号 CRUD 后原子化持久化（临时文件 + rename），文件权限 0600。
- 支持扫码/OAuth 纳管入口（未配置时返回 501 而非报错）。
- 【参考 workbuddy】账号分组（多账号池）：每组可绑定独立���游实例；密钥可绑定分组，请求只走该组账号池；未绑定分组只读不做账号管理。

---

## 三、多密钥分发

每把密钥独立字段：名称、SHA-256 哈希（明文仅创建时展示一次）、掩码提示（前4...后4）、创建时间、过期时间（可空=永不过期）、Token 配额（0=不[...]
可选增强：IP 白名单（CIDR）、最大并发 IP 数、限定版本（国内版/国际版标签）。
密钥持久化到独立 JSON 文件；主密钥（启动参数传入）在密钥文件为空时自动作为 master 保留。

---

## 四、全量审计日志

每次数据面调用记录：时间、方法、路径、模型（客户端请求名）、密钥名称、来源 IP（X-Forwarded-For 优先，回退 X-Real-IP，再回退连接地址）、状态��[...]
存储：默认内存环形缓冲（容量 500，可配），并提供按天/按模型/按密钥的聚合统计接口；可选扩展为 JSONL 按天落盘或 SQLite 持久化。

---

## 五、定时任务框架

- 任务注册表：每类任务（每日签到领积分、Token 保活、自动续期、积分快照采集）独立开关与执行时刻（cron 或整点数���）。
- 任务执行结果与积分收益留痕（如『签到 +1000（30588 → 31588）』），支持按类型筛选与累计汇总。
- 任务失败触发告警钩子（OnFailure 执行通知命令）。
- 实现建议：优先用 systemd timer 或独立调度线程，幂等设计（今日已领取 = 正常跳过）。

---

## 六、上游协议适配层（以华为云 CodeArts MaaS 为例，可替换）

- 签名：SDK-HMAC-SHA256。日期头 X-Sdk-Date（UTC，YYYYMMDDTHHMMSSZ）；body 非空时设 X-Sdk-Content-Sha256；CanonicalURI 按 / 分段逐段 urlEncode（保留 A-Za-z0-9-_.~+）末尾�[...]
- 会话管理：聊天前持有 MaaS 会话（POST /v1/sessions，Agent-Type: PromptCenter），缓存复用 50 分钟，互斥保护；聊天请求携带 x-ot-session-id 与 user-session-id。
- 模型清单：从统计接口动态拉取（enable=true 且 status=normal），10 分钟缓存，内置黑名单过滤实测不可用模型，失败回落静态兜底列表。
- 档位别名：对外暴露 auto/fast/balanced/primary/deep 等档位名，网关侧解析为具体模型并注入默认思考档位（reasoning_effort），客户端显式指定时优先。
- 工具调用：请求含 tools 时向 system 消息追加 XML 工具调用格式提示，保证下游客户端可解析。

---

## 七、管理面 API（/admin/api，与数据面共用密钥体系）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET/POST | /admin/api/accounts | 账号列表（凭据脱敏）/ 新建账号 |
| PATCH/DELETE | /admin/api/accounts/{id} | 启停 / 删除账号 |
| GET | /admin/api/status | 账号运行状态（健康、失败计数） |
| GET | /admin/api/logs | 审计日志（新→旧） |
| GET | /admin/api/stats | 用量聚合（总量、成功率、按天、按模型、按密钥） |
| GET/POST | /admin/api/keys | 密钥列表 / 新建密钥（返回完整明文仅一次） |
| PATCH/DELETE | /admin/api/keys/{name} | 启停/改配额/过期时间 / 删除 |
| GET/PUT | /admin/api/router | 路由规则读写（别名、正则、降级规则） |
| GET | /admin/api/info | 网关运行配置只读展示 |
| POST/GET | /admin/api/oauth/start / callback | OAuth 纳管（未配置 501） |

管理面鉴权：Bearer 头或 gw_key Cookie 二选一；静态页面不鉴权，仅 API 鉴权。

---

## 八、非功能要求

1. **配置**：全部运行参数可经启动参数或配置文件注入（监听地址、加密密���、存储路径、上游 AK/SK、主密钥、熔断/重试/会话参数，默认值见[...]
2. **存储**：默认 JSON 文件（账号/密钥/路由），原子写入；可选 SQLite/PostgreSQL ���配层，接口抽象便于替换。
3. **日志**：终端日志对 Authorization 头脱敏（Bearer ***）；结构化请求日志。
4. **部署**：单静态二进制（musl 编译），前端静态产物内嵌，无外部运行时依赖；systemd 托管，Restart=on-failure。
5. **安全**：凭据 AES-256-GCM 加密；密钥 SHA-256 哈希；管理面与数据面共用密钥但仅放行已启用未过期密钥；HSTS/安全响应头（经反代层）。
6. **可观测**：/admin/api/stats 聚合统计；审计环形缓冲；关键状态变更（账号冷却/恢复、密钥创建/删除、任务执行）有结构化日志。

---

## 附：默认运行参数

| 参数 | 默认值 |
|---|---|
| 熔断阈值 | 连续 20 次失败 |
| 熔断冷却 | 30 秒 |
| 账号失败阈值 | 20 次转 cooling |
| 账号冷却恢复 | 60 秒自动恢复 |
| 重试 | 最多 3 次，线性退避 500ms 起 |
| 会话粘性 | TTL 10 分钟，容量 1000 |
| 审计缓冲 | 500 条环形 |
| 模型缓存 | 10 分钟 |
| 上游会话复用 | 50 分钟 |
| 请求体上限 | 8 MiB |

---

## 验收标准（实现完成后逐项自测）

1. 携带有效密钥调用 /v1/chat/completions（流式与非流式）返回正常，错误密钥返回 401 标准错误信封。
2. 创建别名规则后，用别名请求被正确路由到目标模型，/v1/models 同步出现别名。
3. 新建密钥可独立设置配额，配额用尽后��密钥返回 429，其他密钥不受影响。
4. 流式调用后审计日志包含首字延迟、token 用量；上游未返回扣费时该字段显示为『—』而非 0。
5. 账号连续失败触发冷却，冷却期后自动恢复；熔断器三态切换正确。
6. 定时任务到点执行且幂等，结果留痕可查。
7. 重启服务后账号、密钥、路由规则完整恢复（审计缓冲可清空）。
