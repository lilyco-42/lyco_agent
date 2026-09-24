# lyco_agent 项目 — Agent 引导（AI 协作风格指南）

Rust 实现的轻量化 Agent 运行时(`lycore`) + 分工模型训练实验平台。

> 本文件是**路标不是闸门**：真正强制力在 CI（`build.yml`）与 `scripts/helpfixtures/`。
> 改约定先改机制，再同步这里。每条规则带「为什么」——都是真实付过学费的。

## 🗺️ Agent 工作区域（2026-09-19 划定，多 agent 并行防踩线）

- **本仓 `lyco_agent`**（`lycore/` `tools/` `docs/` `scripts/` `smoke/` `models/`）=
  **lyco_agent 负责 agent 的工作区域**。
- **`lilyco` 主框架仓**（本机 `D:/Code/lilyco`，21 crate，CLI/Web/MCP/TUI 四端）=
  **另一 agent 维护**——本仓任务不碰其代码，跨仓需求走沟通，勿直接改对方文件。
  其铁律见 §六（跨仓沟通时对齐用）。
- **`lyco-ops`**（本机 `D:/Code/p2-lyco_ops`）= 运维脚本仓（lain42 计费 /
  CloudStudio 训练脚本 / 板端部署），归本 agent。

## 🔴 一、硬禁令（红线，违反=事故重演）

| 禁令 | 为什么 |
|---|---|
| 🔴 **不许本机编译**（cargo build/check/test 都不跑）。构建/测试/clippy 一律走 GitHub Actions；本机只跑 `cargo fmt` | 用户 2026-09-24 明令，**取代本文件旧版「本地允许 cargo build」的过期说法** |
| 🔴 本地**禁止**跑 ASR / 模型训练 / 大批量推理 / 下载 GB 级模型实测；训练/评测走 CloudStudio/Kaggle | 长时间占满本机 CPU/GPU 卡住用户（2026-09-10 令，仍然有效） |
| 🔴 **关键模块（boot/内核/分区表/服务/驱动/sudoers）改动必须显式二次确认**；用户说不动的绝不碰 | A7A 事故：用户方案明写「不动 boot 配置」，我擅自改了 extlinux.conf |
| 🔴 **基线/快照永远人工固化**，绝不许 CI 自动写 | 自动更新基线 = 回归被「自动更新」掩盖，防缺陷机制自废 |
| 🔴 **「解析出垃圾」比「解析出 0 条」更危险**（回退条件是 `out.is_empty()`） | 垃圾命令静默失败；0 条至少可见。宁缺勿垃圾 |
| 🔴 **fixture 必须真机实抓，禁止手写** | hw 的 `!/bin/sh`（sed 剥坏的 shebang）手写永远复制不出 |
| ⛔ 板子（A7A）只做端侧推理与部署，**不做训练/重负载/大批量写入**；写盘前查 dmesg、写后 sync+md5 | 用户：「你搞坏我好几张卡，都是新卡」；SD ext4 损坏=写入静默丢数据，size 对≠文件对 |
| ⛔ 交付 HTML 一律自包含（base64 内嵌），严禁相对路径引用 | 交付过引用相对路径 wav 的页面 = 空壳 |
- **大文件不入库**：`.gitignore` 已整目录忽略 `models/`；产物走 Release/对象存储，
  不要 `git add -f` 强塞。提交前 `git status --short` 自检（曾近 1G 模型差点入库）。

## 二、验证结构（不是多写测试，是改验证方式）

1. **单测过 ≠ 被调用**。新能力必须做**接线断言**：grep 旧名零残留 +
   一条测试证明产品路径真的走到它。
   ——8 个历史缺陷的共同点全是「写了没被用」；没有用户的代码朝「看起来对」演化。
2. **改 `help_parse` 必过 CI 池回归**（job `help-parse 池回归`）：
   `fixtures/*.txt` 全量解析 → **逐字快照 diff**（`snapshots/<cli>.txt`）。
   - 红了：逐条核对 diff，确属改进（垃圾消失/条数增/占位符补齐）才更新快照；
   - 其余 fixture 必须逐字零变化（补盲区不得误伤旧形态）；
   - 新样本必须带来**新版式**（冒号对齐/单横杠/横幅/定义列表…），不是同类第 9 个。
3. **先测再断言**；推理是 greedy 确定性（sd=0.00），跑分波动先重复实验再归因。
4. clippy 是 `-D warnings --all-targets`：**lib 挂了 bin 不报**，必须全绿才算过。
   新 rustc 拒绝「闭包返回 &str」→ 用 fn item。

## 三、代码判据风格（parse_help 系，可迁移）

- 判据**确定性、逐行独立、刻意窄**；宁少收，不猜。
- **不可用绝对缩进/区块状态机做收录判据**；节上下文只允许用于**排除**
  （如 HELP TOPICS），永远不用于收录。
- **白名单是 token 精确匹配**：`log`/`logs` 漏一个就退化（t1gate 血训）。
- **值转写纪律**：help 写了值就逐字带上（`-cmd COMMAND`），丢值=静默失效命令；
  没写值绝不编。
- **条数会骗人**：ffmpeg 7 条看着合理、内容全是横幅垃圾。验收看逐字内容不看计数。
- 占位符 `<x>`/`[x]` 不计词数；`...` 不是句号。

## 四、工作模式

- 遇阻**不许提问阻塞**：多数派决策 / 查文档与记忆，做完汇报，不留待办给用户拍板
  （除非涉及不可逆破坏）。
- push：`git -c credential.helper='!gh auth git-credential' push origin main`。
- push 后盯 CI 到全绿；红了按 job 注释指引处理，不绕过。
- 长任务先建 Task 清单（P0-P3 分级），做完逐个销项。

## 五、lilyco（D:/Code/lilyco）铁律（跨仓沟通对齐用）

- CLI+WebUI+MCP+TUI **四端齐开**，一域一二进制，只共享 CommandSchema；
  验收 = 四端结果 JSON **逐字一致**。
- 安全策略**必须在 register 前按调用面注入**（MCP→DenyElevated）；
  删除/覆盖/外呼 ≥T1；参数不拼 shell；Web 只绑 127.0.0.1。
- **NPU 只吃 CNN**，路由器永不上 NPU；视觉主运行时 ONNX Runtime（`ort` pin），
  普惠=厂商 EP（RKNN/QNN/CANN）；⛔ ultralytics 全家 AGPL。
- 普惠不变量：`backend::available()` 恒非空（CPU 兜底）；**降级必须可见**。

## 六、文档与记忆

- 盲区/教训记 `scripts/helpfixtures/BLINDSPOTS.md`（修掉的要销项）。
- 事故全档 `docs/incident-*.md`；主题档 `docs/<主题>-<日期>.md`。
- 评测口径：域内指标不携带泛化信息，**排路线只看跨域 acc_exec**；
  `acc_exec` 是产品指标（acc 只是研究，差 46.6pp）；brush 是唯一工具出口；
  取数按显式 arm key（多 outer key 陷阱曾算出反向结论）。
- 教训全文索引：`docs/incident-a7a-sdcard-2026-09-22.md`、
  `docs/apk-ondevice-agent-2026-09-23.md`、`docs/eval-chat-slm-a7a-2026-09-23.md`。

## 七、当前战略判决（2026-09-24，取代旧「底座决策」）

- **训练线已关**：跨域 acc_exec base 零样本 85.0%（天花板）> 全部 SFT 臂
  （41.7~3.3）⇒ Δ_SFT=−43.3pp；**护城河 = 注入协议 + 能力表（parse_help）+ T1 门，不在权重**。
- reject 全臂 0% ⇒ T1 门是唯一防线；聊天 SFT v1/v2 全负，chat 线已关
  （SFT 唯一收益=快 4×+预算内直答）。
- 判定式脊（`choices.rs`）：CLI=天然选项集，模型只回编号；判定式 100% vs 生成式 41.7%。
  `router::plan*` 有意休眠——`lycore do`（人当模型）是判定式脊的第一个真实调用者，
  `plan_choice` 是将来接 Qwen/Needle 的唯一替换点。
- 底座调研旧档：`docs/research-base-model-2026-09-10.md`、
  `docs/deepseek-training-guide-2026-09-10.md`。

## 八、押注方向（10 年尺度）

- 押「**模型越强，确定性验证越值钱**」；具体判据是耗材，用
  「真实用户暴露 → 真样本固化 → 逐字回归」的循环持续重写。
- 任何能力落地前先问：**谁今天就用它？** 没有真实用户，缺陷是隐形的。
