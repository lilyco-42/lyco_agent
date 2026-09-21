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

### P0 — 把已实现的能力暴露成子命令（当天，无需新代码逻辑）

**P0.1 `lycore vision identify <image>`**
- 内容：把 `cmd_ask` 里调用 `vnn::identify` 的那段逻辑，复制成一个独立子命令
- 验收：`lycore vision identify /tmp/t.png` → `{"kind":"terminal","conf":0.87}`
- 为什么是 P0：`vnn.rs` + executor 分支**都已存在**，只差一个 `fn cmd_vision`
- 预计：~40 行

**P0.2 `lycore npu status` / `lease`**
- 内容：同样，把 `npu_runtime.rs` 已有的租约 API 包一个子命令
- 验收：无板子时 `lycore npu status` → `{"present":false,"reason":"/dev/galcore not found"}`
  （**诚实返回「没板子」也是验收通过** —— 关键是不静默、不 panic）
- 为什么是 P0：409 行死代码零调用，等于没写。接出来才有意义。

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
