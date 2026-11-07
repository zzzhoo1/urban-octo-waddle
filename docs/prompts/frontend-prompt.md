# 前端实现提示词（AI 网关管理面板 · 逆向提炼自 Antigravity-Manager 与 workbuddy-manager）

> 使用方式：把本提示词整体喂给大模型，要求按规格生成完整前端实现。
> 推荐技术栈：Next.js 15（App Router）· React 19 · TypeScript · shadcn/ui · Tailwind CSS v4 · motion · recharts · sonner。如需其他框架，仅替换对应库名，页面与交互��[...] 

---

## 角色

你是一名资深前端工程师。请按照本规格实现一个『AI 账号池代理网关』的 Web 管理控制台。控制台通过管理面 API（/admin/api/*）与后端交互，提供账号��[...] 

---

## 一、全局约定

### 1.1 布局
- 左侧固定导航栏（宽约 224px）：顶部产品标识（图标 + 名称），中部导航项（当前页高亮主色填充），底部退出登录按钮。
- 主内容区自适应宽度，页面最大宽度与内边距统一。
- 登录页为独立全屏居中卡片，不���用导航布局。

### 1.2 交互规范（全站一致）
- 所有操作结果用 toast 反馈（成功绿色、错误红色、警告黄色），危险操作（删除、禁用、清空）一律二次确认对话框。
- 页面加载用骨架屏占位（卡片轮廓 + 列表行轮廓），数据返回后淡入。
- 卡片入场动效：透明度 0→1、纵向位移 12px→0，逐个延迟 50ms（使用 motion）。
- 每页右上角提供『刷新』按钮；列表类页面支持 30 秒静默自动刷新（不打扰当前阅读）。
- 空状态：居中文案说明 + 引导操作（如『暂无账号，点击右上角添加』），不显示空白表格。
- 错误状态：请求失败 toast 提示具体原因，页面保持上一次成功数据不清空。

### 1.3 语义化配色
- 状态色统一：运行中/健康（绿）、冷却/临期/警告（琥珀）、禁用/中性（灰）、失败/过期/危险（红）。
- 配额用量按占比分级着色：<60% 绿、60~90% 琥珀、>90% 红。
- 支持明暗双主题，CSS 变量驱动，跟随系统切换。

### 1.4 移动端适配
- 窄屏（<768px）：侧边栏收起为顶部汉堡菜单；表格自动改为纵向卡片布局（关键字段逐行展示）；多列图表单列排列。

---

## 二、登录页（/login）

- 居中卡片：产品图标 + 标题 + 密码输入框（密钥）+ 登录按钮。
- 提交时先用该密钥调用一个轻量管理接口验证（如 /admin/api/status）：401 提示『密钥无效』，成功则写入 Cookie（7 天有效期，path=/，SameSite=Lax）并跳转��[...] 
- 网络异常与密钥错误分别给出不同提示文案。

---

## 三、仪表盘（/）

### 3.1 汇总卡片（4~5 张，一行排列）
账号总数、健康账号（绿）、冷却中（琥珀）、已禁用（灰）、（可选）今日请求数。每卡：指标名 + 大号数值 + 右侧语义色图标。

### 3.2 图表区（两列）
- 账号状态分布：环形饼图（recharts），健康/冷却/禁用三色分段，带图例与悬浮提示。
- 最近请求延迟趋势：面积图，按调用顺序展示最近 60 条记录的耗时曲线，悬浮显示具体时间点的延迟值。

### 3.3 快捷状态提示（可选增强，参考 workbuddy）
- 账号积分卡片：显示最近一笔积分的额度与到期倒计时，按紧迫度三档着色。
- 临期密钥预警：7 天内过期的密钥以琥珀色徽章提示。

---

## 四、账号管理（/accounts）

### 4.1 列表表格
列：名称、状态徽章（运行中/冷却中/已禁用，语义色）、失败计数（>0 时琥珀高亮）、最近失败时间、创建时间、启用开关、删除按钮。

### 4.2 操作
- 手动录入对话框：名称 + 凭据（密码输入框），说明文案『凭据将加密存储，任何接口不返回明文』。
- OAuth 添加按钮：调用 /admin/api/oauth/start，返回 501 时 toast 提示『当前后端未配置 OAuth，请使用手动录入』。
- 启停开关：即时生效并刷新列表，toast 反馈。
- 删除确认对话框：明示账号名与影响范围（不影响上游账号本身）。

### 4.3 分组视图（参考 workbuddy，可选增强）
- 页面顶部分组切换标签；每组独立展示账号列表；添加账号归属当前分组。

---

## 五、密钥分发（/keys）

### 5.1 列表表格
列：名称、密钥掩码（等宽字体）、状态徽章（生效中/已禁用/已过期/配额用尽）、用量/配额（含占比百分比）、模型白名单（空显示『全部模型』）�[...] 

### 5.2 新建密钥对话框
字段：名称、密钥内容（带『生成』按钮，crypto.getRandomValues 生成 sk- 前缀随机串）、过期时间（datetime-local，可选）、Token 配额（数字，0=不限）、模型[...]
醒目提示：『完整密钥仅在创建时可见一次，请立即复制保存』。

### 5.3 操作
- 启停切换、删除二次确认（提示『使用该密钥的调用会立即被拒绝'）。
- 配额编辑（PATCH）：可直接修改配额与过期时间。

---

## 六、模型中心（/models）

### 6.1 两个展示卡片（左右排列）
- 内置档位/别名列表：别名 → 实际模型映射 + 说明文案 + 思考档位徽章。
- 上游可用模型列表：模型 ID + 来源徽章（owned_by），如实标注数据来源。

### 6.2 自定义路由规则编辑器（页面下方卡片）
- 规则列表：每条规则一行——别名输入框（精确匹配）与正则输入框（二选一，互斥提示）、目标模型、思考档位（可选）、删除按钮；『添加规则』[...]
- 校验：保存前检查每条规则『别名与正则必须且只能填一项』『目标模型必填』，不通过则 toast 指明。
- 『保存规则』按钮提交全量配置，成功后 toast『路由规则已保存并即时生效』。

### 6.3 后台任务自动降级卡片
- 总开关 + 展开后两个字段：降级源模型（逗号分隔）、降级目标模型；说明文案解释 X-Background-Task 头的用法。

---

## 七、用量统计（/stats）

### 7.1 汇总卡片（5 张）
请求总量、成功率（百分比）、Token 消耗（K/M 缩写格式化）、平均延迟、平均首字延迟（无数显示『-』）。

### 7.2 图表区
- 按天用量趋势：面积图双序列（请求数 + Token 消耗），X 轴日期。
- 按模型分布：柱状图，各模型请求量。
- 按密钥分布：横向条形图，各密钥请求量与 Token 消耗双序列。

### 7.3 空状态
无任何调用时显示『暂无调用数据，有请求经过网关后这里会出现统计』；页面注明统计基于内存记录、重启后重新累计。

---

## 八、调用日志（/logs）—— 全量审计页

### 8.1 表格列（对齐 workbuddy-manager 全量审计）
时间、模型、密钥、来源 IP（等宽字体）、状态码徽章（<300 绿、4xx 琥珀、5xx 红）、首字延迟、总耗时、Token（千分位）、实际扣费、缓存命中。
关键规则：**未返回的数据（扣费、缓存、首字延迟）一律显示『—』，绝不显示 0 或空白**——这是与『实际为 0』的严格区分。

### 8.2 筛选与刷新
- 时间范围分段切换：最近 1 小时 / 今天 / 全部（按钮组，当前项主色填充）。
- 30 秒静默自动刷新 + 手动刷新按钮。
- 单次最多显示 200 条，超限时标题栏提示『仅显示最近 200 条』。

### 8.3 展示形式
- 固定高度（约 65vh）内部滚动，表头吸顶，页面整体高度恒定。
- 桌面表格、移动端纵向卡片自适应。

---

## 九、任务记录（/tasks，参考 workbuddy，可选增强）

- 集中展示定时任务执行记录：签到、保活、续期、积分变动流水。
- 每条记录：时间、任务类型徽章、结果（成功/失败/跳过）、积分收益（+1000 绿色高亮）、关联账号。
- 类型筛选下拉 + 累计积分汇总行；时间范围筛选与固定高度内滚同日志页。
- 『今日已签到』显示为正常态（幂等成功），不显示为失败。

---

## 十、系统设置（/settings）

### 10.1 运行配置只读卡片
以定义列表展示网关当前生效参数：版本（徽章）、监听地址、上游地址（等宽字体）、熔断阈值/冷却、失败阈值、冷却恢复时长、重试次数与退避、�[...] 

### 10.2 部署说明卡片
编号列表说明升级流程（CI 构建 → 下载产物 → 校验 sha256 → 替换二进制 → 重启）；提示框说明前端已内嵌进二进制、无需 Node 环境、启动参数清单��[...] 

### 10.3 高级设置（可选增强，参考 workbuddy)
- 定时任务开关与执行时刻编辑（输入即校验：时刻 0-23 去重排序、时长格式 30s/10m/2h）。
- 只提交改动项，不全量覆盖；读取失败时锁定保存按钮并��示原因。

---

## 十一、组件清单（shadcn/ui 风格，自建基础组件）

Button（default/destructive/outline/secondary/ghost/link 六种变体，sm/default/lg/icon 四种尺寸）、Card（Header/Title/Description/Content/Footer）、Badge（default/secondary/destructive/[...] 

---

## 十二、数据接口契约（前端依赖）

```
GET    /admin/api/status        → [{id,name,status,healthy,failure_count}]
GET    /admin/api/accounts      → [{id,name,status,failure_count,last_failure_at,created_at}]
POST   /admin/api/accounts      → {name,credential} → 201 账号DTO
PATCH  /admin/api/accounts/:id  → {status: active|disabled}
DELETE /admin/api/accounts/:id  → 204
GET    /admin/api/keys          → [{name,hint,created_at,expires_at,token_quota,tokens_used,model_allowlist,enabled,expired}]
POST   /admin/api/keys          → {name,key,expires_at?,token_quota?,model_allowlist?} → 201（仅此一次返回完整密钥由前端自行保存展示）
PATCH  /admin/api/keys/:name    → {enabled?,token_quota?,expires_at?}
DELETE /admin/api/keys/:name    → 204
GET    /admin/api/stats         → {totals:{requests,errors,success_rate,total_tokens,avg_latency_ms,avg_ttft_ms},by_day:[...],by_model:[...],by_key:[...]}
GET    /admin/api/logs          → [{ts,method,path,model,status,latency_ms,ttft_ms,total_tokens,credit,cached_tokens,source_ip,key_name,session_id}]
GET    /admin/api/router        → {rules:[{alias?,pattern?,target,effort?,note?}],downgrade?}
PUT    /admin/api/router        → 同上结构 → 保存即生效
GET    /admin/api/info          → {version,listen,upstream,各阈值参数,sticky_sessions,api_keys}
GET    /v1/models               → {data:[{id,object,owned_by}]}（数据面，需 Bearer）
```

错误处理：401 统一跳登录页；其他错误取响应 error.message 或 error 字段 toast 展示。

---

## 验收标准（实现完成后逐项自测）

1. 登录页输错密钥提示『密钥无效』，输对进入仪表盘。
2. 仪表盘卡片数值与账号实际状态一致，饼图分段配色符合语义色规范。
3. 创建密钥后完整密钥展示一次，刷新页面后只剩掩码；禁用该密钥后其调用返回 401。
4. 模型中心添加别名规则并保存后，用别名调 /v1/chat/completions 能正确路由。
5. 日志页流式调用记录显示真实首字延迟；扣费列在上游未返回时显示『—』而非 0。
6. 统计页有调用后出现趋势图与分布图；时间范围切换正确过滤。
7. 所有危险操作均有二次确认；所有操作均有 toast 反馈。
8. 手机宽度下表格自动变为卡片布局，无横向撑破。

---

## 已应用现代化变更（2026-10-15）

注：以下变更已在分支 `feature/modernize-prompt-and-scaffold` 中实现，用于保持提示词与仓库实现一致，便于后续将该提示词喂给大模型生成一致代码。

变更概述
- 引入并实现现代化响应式侧边栏（可访问、移动抽屉、折叠持久化、Tooltip）。
- 增加语义化主题 token（surface / sidebar 等）并在 globals.css 中提供暗/亮配色。
- 新增 Playwright E2E 测试覆盖侧边栏（桌面/移动行为）。
- 优化图标导入为按需静态映射以减少运行时打包。

已修改 / 新增的关键文件
- components/Sidebar.tsx
- components/icons.tsx
- app/layout.tsx
- app/globals.css (语义 token 与工具类)
- package.json（增加 Playwright 脚本与依赖）
- playwright.config.ts
- tests/sidebar.spec.ts

本地运行与测试（快速参考）
1. 切换到实现分支：
   - git fetch origin
   - git checkout feature/modernize-prompt-and-scaffold
2. 安装依赖：
   - pnpm install
3. 启动开发服务器（手动预览）：
   - pnpm dev
   - 访问 http://127.0.0.1:3000 并验证侧栏行为
4. 运行 E2E（会在需要时启动 dev server）：
   - pnpm test:e2e
   - 可视化运行： pnpm test:e2e:headed

新增/更新的验收项（补充原有验收标准）
- 侧栏（桌面）支持 collapsed/expanded 两种状态，折叠偏好持久化（localStorage），折叠时显示 tooltip。
- 侧栏（移动）以抽屉/Dialog 形式展示且可通过汉堡按钮打开/关闭。
- Playwright E2E 覆盖：侧栏渲染、折叠/展开、移动抽屉打开、路由高亮、底部退出项可见。
- 全局样式已新增语义 token（--surface / --sidebar 等），组件应使用这些 token 或对应的 Tailwind 工具类（.bg-surface/.bg-sidebar）以保证暗/亮主题一致。

注意事项
- 侧栏使用了 Radix Dialog/Tooltip、zustand（持久化）、lucide-react 图标；请确保在 package.json 中包含对应依赖并通过 pnpm/yarn/npm 安装。
- globals.css 已定义语义 CSS 变量与一些 Tailwind 工具类，建议将这些 token 同步到 tailwind.config.ts 的 theme.extend 中以便完整整合（可以进一步自动化）。
- Playwright webServer 配置假设 `pnpm dev` 在 127.0.0.1:3000 启动；若项目启动命令或端口不同，请调整 playwright.config.ts。

下一步（可选）
- 将此节追加到 docs/prompts/frontend-prompt.md（确认后我可直接提交）。
- 将语义 token 映射入 tailwind.config.ts（我可以代为提交）。
- 在 CI 中添加 Playwright workflow（我可以创建 GitHub Actions 流程）。
