# lain42 统一身份 + 积分体系（重构设计）

> 目标：**一套账号、一本积分账、一个 USDT 到账入口**，所有微服务共用。
> 现状：4 套账号体系各自为政，同一个人在不同服务里是不同账号（已实测确认）。

---

## 一、现状盘点（2026-09-18 实测）

| 服务 | 位置/端口 | 账号 | 余额/额度 | USDT 订单 |
|---|---|---|---|---|
| **studio-billing** | `/opt/studio-billing` :4700 | `users` 5 行（**有 email**） | `points_accounts.balance` + `points_ledger`（**双式记账，最规范**） | `orders` (amount_usdt, tx_hash) ✓ |
| **proxy-panel** | `/opt/proxy-panel` :8010 | `users` 8 行（uuid + 部分 email） | 流量制 `traffic_limit/used` + `plans` 套餐 | `orders` (amount_usdt, **无 user_id**，靠 card_code) |
| **compute**（算力平台） | `/var/lib/compute` :4601 | `users` 2 行（**无 email**） | `credits` 字段 + `credit_log` | `orders` (credits, cents, provider) |
| **new-api** | `/opt/new-api` | 自带用户/额度（Go 第三方） | quota/token 制 | 自带充值 |

**两个并存的 USDT watcher**：`proxy-panel/usdt_watcher.py`（按金额匹配 pending 订单）+ billing 自己的（`logs/watcher.log`）——都在轮询同一个收款地址。

### 同一人跨服务的真实映射（实测）

| 人 | studio-billing | proxy-panel | compute |
|---|---|---|---|
| A | `123` / liuqingffffff@gmail.com | `wb_3b145848` / 同邮箱 | `lilyco42` |
| B | `admin` / wakeyourselfup@qq.com | `wb_a097332d` / 同邮箱 | `admin01` |

> **email 是唯一跨服务可对齐的字段**；compute 的两个账号没有 email，需要人工绑定。

---

## 二、目标架构

```
                           ┌──────────────────────────────┐
                           │  lain42-id  (统一身份+积分)   │  ← 由 studio-billing 演进
                           │  :4700                       │
   USDT TRC20 ──watcher──► │  users / points_accounts     │
   (唯一收款地址)           │  points_ledger(不可变流水)   │
                           │  usdt_orders / service_keys  │
                           └───────┬──────────┬───────────┘
                     s2s API (内网)│          │s2s API
                        ┌──────────┘          └──────────┐
                        ▼                                ▼
                 ┌─────────────┐                  ┌──────────────┐
                 │ compute     │                  │ proxy-panel  │
                 │ (算力平台)   │                  │ (代理面板)    │
                 │ 只存业务数据 │                  │ 只存业务数据  │
                 └─────────────┘                  └──────────────┘
                        ▲                                ▲
                        └────────────┬───────────────────┘
                                     │  统一登录 (SSO)
                                 new-api / 其他服务
```

**原则**
1. **单一账户源**：`users` 只有一份（lain42-id），其他服务只存 `lain42_uid` 外键
2. **单一积分账本**：所有加减分都写 `points_ledger`（不可变流水），余额是流水的派生值
3. **单一 USDT 入口**：一个 watcher，写统一 `usdt_orders` → 到账积分
4. **服务间鉴权**：`service_keys`（每个服务一个 key），调 s2s API；用户侧用统一 token（SSO）
5. **幂等**：所有扣分/加分带 `idempotency_key`（防重放/防并发双扣）

---

## 三、数据模型（lain42-id）

```sql
-- 统一账户（在现有 users 上扩展）
users(id, username UNIQUE, email UNIQUE, pass_hash, pass_salt, role, founder, created_at)

-- 身份映射：把各服务旧账号挂到统一 uid 上
identity_links(id, uid, service, external_id, external_username, email, linked_at)
--   例: ('proxy-panel','wb_3b145848'), ('compute','u_d1846342ac964fc2')

-- 积分账本（已有，保持）
points_accounts(user_id PK, balance, updated_at)
points_ledger(id, user_id, delta, balance_after, reason, reference, idem_key UNIQUE, created_at, meta)

-- USDT 订单（统一，替代 3 套 orders）
usdt_orders(id, order_no UNIQUE, uid, amount_usdt, credits, purpose, status, tx_hash, created_at, paid_at)

-- 服务密钥
service_keys(service, api_key, enabled, created_at)
```

---

## 四、服务间 API（s2s，仅内网 / HMAC）

| 方法 | 路径 | 用途 |
|---|---|---|
| POST | `/api/s2s/verify` | 校验用户 token → 返回 uid/username/role |
| GET | `/api/s2s/user/{uid}` | 查用户基本信息 |
| GET | `/api/s2s/points/{uid}` | 查余额 |
| POST | `/api/s2s/points/debit` | 扣分（`{uid, amount, reason, idem_key}`）→ 409 若余额不足 |
| POST | `/api/s2s/points/credit` | 加分（退款/奖励） |
| POST | `/api/s2s/users/ensure` | 按 email 找到或创建用户（服务首次接入时用） |

鉴权：`X-Service-Key: <service_keys.api_key>`；**只绑定 127.0.0.1**，不对外暴露。

---

## 五、各服务改造点

### compute（算力平台，:4601）
- `users` 表 → 只保留 `billing_uid`（其余字段删/迁移）
- `credits` 字段 → 删除，扣分改调 `POST /api/s2s/points/debit`（`reason='compute:'+service`）
- `credit_log` → 迁移进 `points_ledger`（保留原 id 到 `meta`）
- `devices.owner_id` → 存统一 uid
- `tasks.cost` 不变，但结算走统一账本

### proxy-panel（:8010）
- `users` → 保留业务字段（uuid/traffic_*/expires_at），加 `billing_uid`
- **决策点**：套餐制保留（用积分**购买套餐**，套餐再管流量）还是改成纯积分计量？
- `orders` → 迁移到 `usdt_orders`（purpose='proxy-plan'），保留 `card_code` 逻辑
- 删除自己的 `usdt_watcher.py`（由统一 watcher 接管）

### new-api（Go 第三方）
- **不改其代码**（改造成本高）。方案：**额度同步**——
  - 定时（或充值触发）用统一余额换算 new-api 的用户额度（调其管理 API）
  - 或让 new-api 的 token 由 lain42-id 分发（每用户一个 key，额度上限=余额 × 系数）
- **决策点**：选"额度同步"还是"token 分发"？

### studio-billing → **lain42-id**
- 新增上述 s2s API + `identity_links` + `service_keys`
- watcher 升级为唯一 USDT 入口（合并 proxy-panel 的匹配逻辑：按金额匹配 pending 订单 + 卡片生成）
- 前端：统一登录页（各服务跳转到此登录，回跳带 token）

---

## 六、迁移步骤

| 阶段 | 内容 | 风险 | 回滚 |
|---|---|---|---|
| **P0 设计** | 本文档 + 决策拍板 | 无 | — |
| **P1 增强** | lain42-id 加 s2s API / identity_links / service_keys / 统一 watcher（**不改动现有表**） | 低 | 停新接口即可 |
| **P2 迁移（dry-run）** | 生成映射表：按 email 对齐 + 人工确认；输出"迁移预览"（谁合并到谁、余额加总） | 无（只读） | — |
| **P3 迁移（执行）** | 备份 3 个库 → 写入 identity_links → 旧账号标记只读 → compute/proxy 切到 s2s 扣分 | 中 | 恢复备份 |
| **P4 统一入口** | 统一登录页 + 各服务回跳；proxy-panel 去掉独立 watcher | 中 | 保留旧登录路由 |

**对账铁律**：迁移前后 `sum(points_ledger.delta)` 必须等于旧系统余额之和；不一致就停下查。

---

## 七、待拍板（5 个决策）

1. **汇率**：1 USDT = **?** 积分（现有 USDT 订单金额 14.5~30 USDT，billing 余额 580 分；compute 用 credits）
2. **账号主键/合并规则**：email 为主键？同名不同邮箱如何处理？compute 无 email 的两个账号（`admin01`/`lilyco42`）人工绑到谁？
3. **proxy-panel 套餐**：保留"套餐+流量"（积分购买套餐），还是改成纯积分计量？
4. **new-api**：额度同步 vs token 分发？
5. **compute 的 credits 折算**：1 credits = 1 积分？

---

## 八、收益

- **用户侧**：一个账号通行所有服务；充一次 USDT 全站通用；积分不会"充了但另一个站用不了"
- **运维侧**：一份用户表、一本账、一个 watcher；对账/审计/风控只需看一处
- **商业侧**：积分成为全站统一计价单位（算力/代理/API 都能定价），便于打包与促销
