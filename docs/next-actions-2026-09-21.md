# 下一步：停止研究，开始接线（2026-09-21 12:xx）

> 用户质问：「所以呢？你打算怎么做，是研究别人怎么做还是？」
> 这份文档是对该质问的直接回答。**它不引入任何新调研**，只做三件事：
> ① 诚实清点「研究」与「交付」的差距；② 给出可验收的动作清单；③ 明确什么不再做。

---

## 0. 承认问题：产出结构已经失衡

| 类别 | 数量 | 位置 |
|---|---|---|
| 研究/设计文档 | **29 个文件 / 3918 行** | `docs/*.md` |
| 已实现的库代码 | ~1800 行（capability/executor/agent/vnn/npu_runtime） | `lycore/src/*.rs` |
| **可从 CLI 触达的能力** | **13 个子命令**，但**没有一个是视觉/NPU** | `bin/lycore.rs` |

**核心差距**：`capability-roadmap-2026-09-21.md` 承诺的
`lycore vision identify` / `lycore yolo detect` / `lycore npu lease`
**三个子命令，一个都没注册**。文档里写着「当天可交付」，代码里没有入口。

而 `npu_runtime.rs`（409 行）**零处被调用** —— 是纯死代码。
（`vnn.rs` 情况不同，见下。）

这不是「调研不够」，是**接线没做**。用户问得对。

---

## 1. 先分清「已经真的通了」和「只是写了」

必须精确，不能笼统地说「都没接」。

### ✅ 真的通了的：`vnn_identify` 全链路已接线

```
capability.rs:45   声明 ("vnn_identify", &[FileRead, Camera])
executor.rs:177    分发 → crate::vnn::identify(ffmpeg, image)
tests/vnn_integration.rs  集成测试（需 ffmpeg，本地标 ignored）
bin/lycore.rs cmd_tools   已把 vnn_identify 输出进工具 schema（远端实测确认）
```

**这条链只差最后一步**：没有一个 CLI 子命令能让用户直接用手调它。
`lycore ask --llama <url>` 能触发（走 agent loop），但那需要先起 llama-server。

### ❌ 真的没通的

| 项 | 状态 | 证据 |
|---|---|---|
| `npu_runtime.rs` | **死代码，零调用** | `grep -rn "npu_runtime::"` 无结果 |
| `lycore vision identify` | 无此子命令 | `match cmd` 里只有 13 个分支，无 vision |
| `lycore yolo detect` | **yolo.rs 不存在** | 文件缺失 |
| 三端（CLI/TUI/Web）一致性 | 未验证 | lycore 有 serve.rs 但无 TUI/MCP 四端验收 |

---

## 2. 动作清单（按「能否当天验收」排序）

原则：**每条都要有一个可以贴出来的命令 + 输出**。凡是不满足这条的，不进清单。

### P0 — 把已实现的能力暴露成子命令

**P0.1 `lycore vision identify <image>` — ✅ 已完成（`df9695f`）**
- 远端实测：`t_pattern.png` → GUI 窗口界面 (0.95) / `t_dark.png` → 终端界面 (0.50) /
  `t_white.png` → `unknown` + `needs_training:true`（诚实标记）+ 入学习队列 3 条
- 错误路径：不存在的图 → exit=1 不 panic；用法错 → exit=2

**P0.2 `lycore npu status` — ✅ 已完成（`23e0f2c`）**
- 远端实测：无板子 → `present:false` + **exit 0**（正常状态，非报错）
- JSON 含 `serial_constraint` 与 `fallback` 说明
- 把 409 行零调用的 `npu_runtime.rs` 接出；其单测现已被真实跑到

**P0 结项**：132 lib 单测全绿。`capability-roadmap` 三块能力中，
**vision 与 npu 已从纸面变成用户可敲的命令**，剩余 yolo 见 P1。

### P1 — 补上真正缺的实现（需要新逻辑）

**P1.1 `lycore yolo detect` 的 CPU 兜底**
- 内容：`tract`（纯 Rust ONNX，零外部依赖）跑一个 YOLOX-nano / PicoDet
- 为什么用 tract：`research-cross-device-vision` 已定 Tier-0 兜底就是它，不赌用户有 libonnxruntime.so
- 验收：`lycore yolo detect --image x.jpg --backend cpu` → JSON 框列表
- 前置：需要真模型文件（可先用一个 5MB 的测试模型）

**P1.2 T1 门补「必需参数校验」**
- 内容：`t1gate.rs` 除拒危险命令外，校验必需参数是否存在
- 依据：v18 实测 F4 失败模式 `npm install` 漏掉 `express`（空参调用是**静默失效**）
- 验收：新单测 + `lycore t1 check "npm install"` → `{"verdict":"needs_param","missing":["<pkg>"]}`

### P2 — 只能靠训练解决，先不动（明确挂起）

- **出域 exec 从 41.7% 往上提**：Δ_SFT = −43.3pp，说明当前 SFT 是负的。
  下一步是**少训练**（LoRA / 少 epoch / 先验域 replay）对照实验，**不是加数据、不是换底座**。
- 但这条**优先级低于 P0/P1**：因为 P0/P1 是不占 GPU、当天可验收的纯增量。

---

## 3. 明确不再做的事

用户质问的另一半意思是「别老研究别人」。据此，以下行为**暂停**：

| 停止 | 原因 |
|---|---|
| 新增 `docs/research-*.md` | 已有 29 篇 / 3918 行，边际价值已为负 |
| 横向对比第三方模型榜 | BFCL / AI-Radar 已看过，结论已入档，再看不加分 |
| 「调研一下 X 怎么做」式任务 | 除非对应一个 P0/P1 动作，否则不做 |
| 为「能力拓宽」再写设计文档 | 设计已定（子命令 + capability 表），只缺代码 |

**唯一的例外**：如果某个 P0/P1 动作被证明确实卡在「不知道怎么实现」，
才允许做**限定范围的**调研（只查这一个问题，不写文档，直接产出代码）。

---

## 4. 立刻开始的那一条

从 **P0.1** 开始：`lycore vision identify <image>`。

理由：
- 逻辑已存在（`vnn::identify` + executor 分支），不存在「不会写」的风险
- 当天可在远端编译 + 实测出可贴的输出
- 它把「29 篇文档」里的第一条能力真正变成用户能敲的命令
- 做完后 `capability-roadmap` 的三块能力里就有 1 块从纸面变成现实

**下一步动作（不写文档，直接改代码）**：
1. `bin/lycore.rs` 加 `fn cmd_vision` + `match` 分支
2. 远端 `sync_build.py --bin-name lycore2 --test`
3. 造一张测试图，跑 `lycore2 vision identify`，把输出贴出来

---

## 5. 执行结果（2026-09-21 12:4x 收尾）

**P0 两项当天完成并远端实测**，不写新文档，直接改代码：

| 项 | commit | 实测证据 |
|---|---|---|
| `lycore vision identify` | `df9695f` | 三种测试图分别 → GUI 0.95 / 终端 0.50 / unknown+needs_training；错误路径 exit 1/2 |
| `lycore npu status` | `23e0f2c` | 无板子 → `present:false` **exit 0**；JSON 带 serial_constraint |

**验证方式**：`sync_build.py --bin-name lycore2 --test`（远端编译 52s + 132 单测全绿）
→ `cs_exec_long.mjs` 跑真实命令取输出。

**结论**：`capability-roadmap` 的三块能力，**vision 与 npu 已可被用户直接调用**；
yolo 是唯一真正缺实现的（P1.1）。研究→交付的欠账还清两项。

---

## 6. P1.2 执行结果（2026-09-21 13:0x 收尾）

**P1.2 必需参数门已完成并本机实测**（`ba73068` 实现 + `44dc60e` 修复）：

| 场景 | 实测输出 | 验收 |
|---|---|---|
| `lycore t1 check "npm install"` | `verdict: needs_param` + `missing: ["<pkg>"]` + exit 1 | ✅ 与本文档 §2 P1.2 口径**逐字一致** |
| `lycore t1 check "npm install express"` | `verdict: confirm` / `param.status: ok` | ✅ 补齐后回到正常写确认流 |
| `lycore t1 check "rm -rf /"` | `verdict: block` / `param.status: unchecked` | ✅ 危险优先，未被缺参降级 |
| 只读四连 `gh issue list` / `git status` / `cargo check` / `docker ps` | `run` + **exit 0** | ✅ 零回归 |
| `npm run build` / `git add .` / `git log` / `docker build -t x .` | 各自正确 | ✅ 无误伤 |

**新增能力**：`paramcheck` 模块（动词表驱动，覆盖包管理/git/容器/网络/进程/ffmpeg），
三态返回 `Ok` / `NeedsParam` / `Unchecked` —— **未覆盖的程序诚实报 Unchecked，不假装 Ok**。

**单测**：132 → **148**（+16，0 failed）。本机 `env -u BASH_ENV cargo test --lib` 实测 0.34s。

**设计要点**（写进代码注释）：
- 分诊链顺序 **Noop → Danger(优先) → NeedsParam → Confirm → Run** —— 缺参不得覆盖危险
- `may_execute` 对 `NeedsParam` 恒 false（空跑连 override 也救不了）
- 只报「确定缺」的，宁漏勿错 —— 误报会拦掉合法命令，比放过坏命令更伤产品

**踩到的 3 个真实缺陷**（本机单测暴露，都不是测试写错）：
1. 实参起点偏移一位（`sudo` 分支 `cmd_idx` 算成 2 应为 1）
2. 两级动词 `git remote add` 的 `add` 被当成实参 → 新增多词动词匹配
3. 豁免边界过宽 → 收窄为仅「动词为空」的程序（curl/wget/ffmpeg）

**跳过的一步**：远端 `sync_build.py` 因 CloudStudio cookie 失效（`leak_auth_params`）
未跑成 —— 但**本机 cargo 验证已覆盖同等范围**（单测 + release 二进制实测），
产物构建待 cookie 恢复后补跑。

---

## 7. 剩余动作

- **P1.1 `lycore yolo detect`**：唯一真正缺实现的（需 ~5MB 测试模型 + `tract` 依赖）
- **P2 出域 exec 提升**：少训练对照实验（LoRA / 少 epoch / 先验域 replay），需 GPU
- 上轮遗留：git 裸 help(100.0) > hparse(87.5) 原因排查；`agent.rs` 身份描述泛化
