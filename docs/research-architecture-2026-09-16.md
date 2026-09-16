# lyco_agent 架构演进研究：从「3 日 AI 科技简报」到代码落地

> 调研日期：2026-09-16 | 触发：用户 `/goal` 指令——续做 LYCO agent 架构研究，并把 **CloudStudio 当作云端算力后端**
> 方法：① 精读 3 日「AI 科技每日简报」（09-14/09-15/09-16）；② 对照 `lyco_agent` 现有 Rust 代码（`verify.rs` / `executor.rs` / `learn_cli.rs` / `llamacpp.rs` / `tools_runtime.rs` / `DESIGN.md`）；③ 外部论文与算力规格已通过 WebSearch 核实（ToolGrad / Tool-Star / TinyAgent / CloudStudio GPU 规格）
> 硬约束（沿用）：**不碰 A7A 板子**（用户指令「不要走 板子 了」）；CloudStudio 为 session-scoped 训练/评测，非常驻推理服务

---

## 0. 一页结论（给用户的 TL;DR）

| 简报主张 | lyco 现状 | 差距 / 落地动作 |
|---|---|---|
| **能力层（Capability）应是一等公民** | 无 `enum Capability`，工具能力散落在 `CHAT_TOOLS` 描述里 | 新增 `lycore::capability` 模块，Skill 与工具显式声明能力集 |
| **Skill 描述符 = { capabilities, risk, verifier, executor }** | `verify.rs`（verifier）+ `executor.rs`（executor）已分家，但无结构化 Skill 描述符 | 定义 `Skill` struct，串起 verifier/executor/risk |
| **Verifier 与模型解耦** | `verify.rs` 的 OCR 级联已是确定性代码路径，不依赖 LLM ✓ | 补 VNN Rust 路径（当前降级到 learning_queue）；Verifier 注册表化 |
| **Skill-RAG / Tool-RAG** | `learn_cli.rs` 索引 CLI 手册到 sqlite/FTS5，但**无语义检索** | 加 embedding 召回层（Gorilla / TinyAgent ToolRAG 范式） |
| **小模型 + 工具调用 > Scaling Law** | GRPO 已在 A10 跑通 60%→100%（0.6B）✓ | 接 ToolGrad / Tool-Star 式数据闭环，扩大经验蒸馏（`lernen.rs`） |
| **A733 NPU 调度器** | 板子工作已 halt，NPU 单网络串行限制已知 | 设计 `lyco-npu-runtime` crate（load/unload/infer/priority/lease），代码先写，等板子恢复再联调 |
| **云端算力** | GRPO 实测在 CloudStudio A10 ✓ | 明确 A10=20核/116G/24G显存/抵扣因子3.3，专做训练+评测；端侧 SLM 做推理 |

**一句话**：简报的几乎所有主张，lyco 在「架构意图」上已经对了（verifier/executor 分家、SkillRegistry、工具 schema 单一来源），缺的是**把意图提升为显式类型系统（Capability/Skill/Verifier 注册表）** 和 **把已验证的 GRPO 经验蒸馏接成持续数据闭环**。CloudStudio 是当前唯一可用的重算力，专门喂这条闭环。

---

## 一、简报核心主张 ↔ lyco 代码映射

### 1.1 能力层（Capability Layer）作为一等公民

**简报观点**：agent 不应直接暴露「能执行什么命令」，而应声明一组抽象能力（FileRead / Shell / Network / Camera / DeviceControl …），由运行时把能力映射到具体权限与工具。这样策略层（模型/调度）只看得见能力，看不见危险的原语。

**lyco 现状**：`llamacpp.rs` 的 `CHAT_TOOLS` 是 7 个工具的函数 schema，每个工具隐含一种能力，但**没有显式的能力枚举**。例如 `rembg_remove` 隐含 FileWrite+Shell，`llm_generate` 隐含 Network。权限判断散落在 `tools_runtime.rs` 里。

**落地设计**（新增 `lycore/src/capability.rs`）：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    FileRead, FileWrite, Shell, Network,
    Camera, DeviceControl, GitHub,
}

/// 工具 → 能力集（单一来源，与 CHAT_TOOLS 同文件维护）
pub const TOOL_CAPS: &[(&str, &[Capability])] = &[
    ("lyv_knowledge",      &[Capability::FileRead]),
    ("vnn_identify",       &[Capability::FileRead, Capability::Camera]),
    ("rembg_remove",       &[Capability::FileRead, Capability::FileWrite, Capability::Shell]),
    ("html_gen",           &[Capability::FileWrite]),
    ("llm_generate",       &[Capability::Network]),
    ("html_render_video",  &[Capability::FileWrite, Capability::Shell]),
    ("video_info",         &[Capability::FileRead]),
];
```

**价值**：模型-escalation（0.6B→1.7B→8B→云端前沿）时，能力层是「这个档位能不能跑这个工具」的唯一裁决点——小模型可声明只拥有 `FileRead` 子集，越权工具直接不进它的 schema（这正是 ToolGrad「answer-first」想降低的失败面）。

### 1.2 Skill 描述符 = { capabilities, risk, verifier, executor }

**简报观点**：把「技能」从一段 prompt 升级为一个结构化对象——能力、风险等级、验证器、执行器四元组。Verifier 决定「这个技能算不算做成了」，Executor 决定「怎么做」。

**lyco 现状对照**：

| 字段 | 简报 | lyco 现有代码 | 位置 |
|---|---|---|---|
| `capabilities` | 声明能力集 | 隐含在工具 schema，无枚举 | 见 1.1 |
| `risk` | 风险等级 | 无 | 待加 |
| `verifier` | 验证器 | `verify.rs` 的 `Ocr::recognize` + `verify()` 级联 | `lycore/src/verify.rs` |
| `executor` | 执行器 | `executor.rs` 的 `Executor::dispatch` + `tools_runtime.rs` | `lycore/src/executor.rs` |

`verify.rs` 当前逻辑（已读，201 行）：OCR 级联 → `ocr_pass()`（conf≥min_conf 或 lev≤2 模糊）→ `verify()` 返回 `Verdict{route, pass, ocr_conf, vnn_hint}`。VNN 的 Rust 路径**未实现**，因此非 OCR 技能一律降级到 `learning_queue`（诚实降级，这是 lyco 差异化卖点）。

**落地设计**（合并到 `SkillRegistry`，DESIGN.md 已规划但未落地为类型）：

```rust
pub struct Skill {
    pub name: String,
    pub capabilities: &'static [Capability],
    pub risk: RiskLevel,                       // Low / Medium / High
    pub verifier: VerifierId,                  // 指向 verify.rs 的某个级联
    pub executor: ExecutorId,                  // 指向 executor.rs 的某个 tool
}

pub enum RiskLevel { Low, Medium, High }
```

> 注：DESIGN.md 已描述 SkillRegistry，但 `lib.rs` 当前模块列表里**没有** `skill` / `skill_registry` 模块——这是「意图已写、类型未建」的典型 gap，本研究的直接产出就是把它落成代码（见第五节任务清单）。

### 1.3 Verifier 与模型解耦（已部分达成，需注册表化）

**简报观点**：验证「任务是否完成」应该是确定性程序（OCR 比对、文件存在、退出码），不是再问一次 LLM。模型只知道「想做什么」，不知道「做没做成」。

**lyco 现状**：✅ 已对。
- `verify.rs`：`ocr_pass()` 是纯确定性算法（tesseract conf + 编辑距离），不调 LLM。
- `executor.rs`：执行失败 → `LearningQueue::push()` 写 `learning_queue.jsonl`，而不是假装成功。

**缺口**：Verifier 目前只有「OCR 一种」。简报要的是「每个技能挂一个 verifier」。落地动作是把 `verify.rs` 的级联抽象成 `Verifier` trait + 注册表，让新增技能（如 `html_render_video` 可验证「视频时长>0 且能 ffprobe 出流」）即插即用。

### 1.4 Skill-RAG / Tool-RAG（语义检索，当前只有关键词）

**简报观点**：工具/技能太多时，不能全塞进 prompt。先检索「相关工具子集」再喂模型（Gorilla 检索增强 FC；TinyAgent 的 ToolRAG 把 100+ 工具检索到 top-k，1.1B 模型超过 GPT-4-Turbo）。

**lyco 现状**：`learn_cli.rs` 的 `index_and_build()` 已把 CLI 手册索引进 sqlite + FTS5（`parse_commands()` 三态解析 clap/adb/scrcpy help）。这是**全文检索**，不是语义检索。

**缺口（DESIGN.md 已记）**：`index_lilyco_schema()`（learn_cli.rs:175）是**孤儿函数——零调用者**。本应把 lyco 自身子命令 schema 也索引进同一张表，但没有接线。

**落地设计**（ToolRAG 范式）：在 FTS5 之上加一层 embedding 召回——
```
query → embed → 取 Top-K 工具 schema（来自 CHAT_TOOLS + learn_cli 索引）
      → 拼成「裁剪后的 tools_openai.json」→ 喂给 0.6B/1.7B
```
这直接复用了 `llamacpp.rs` 的 `chat_tools()` 单一来源，且让小模型在「工具多」时不爆 context（TinyAgent 已证：1.1B + ToolRAG = 80.06% > GPT-4-Turbo 79.08%）。

### 1.5 小模型 + 工具调用 > Scaling Law（已实证，需接数据闭环）

**简报佐证（WebSearch 核实）**：

| 论文 |  venues / 链接 | 核心结论 | 对 lyco 的映射 |
|---|---|---|---|
| **ToolGrad** | arXiv 2508.04086, ACL 2026, Google XR | answer-first 反向数据生成；ToolGrad-500（仅 500 样本）训 Gemma-3 1B/4B/12B；BFCL **83.1**（12B，逼近 Gemini 2.5 Pro 83.2 / Claude 4.5 Opus 82.8 / GPT-5 74.4）；ToolBench 16k+ API 通过率 99.8% | 「少量高质量工具数据」即可拉平大模型——印证 lyco 0.6B GRPO 路线 |
| **Tool-Star** | arXiv 2505.16410, SIGIR 2026, RUC-NLPIR | Qwen-0.5/1.5/3/7B；SFT-54K + Multi-Tool-RL-10K；冷启动 SFT + 多工具自批判 RL（层次奖励），ARPO/AEPO 变体 | 直接对应 lyco 的「SFT 预热 → 环境反馈 RL」两段式 |
| **TinyAgent** | arXiv 2409.00608, Berkeley | 1.1B/7B + LLMCompiler DAG + ToolRAG；7B 84.95% / 1.1B 80.06% > GPT-4-Turbo 79.08% | ToolRAG 是让小模型不吃亏的关键，见 1.4 |

**lyco 已实证**（research-base-model-2026-09-10.md）：GRPO 在 CloudStudio A10 上 200 步、11 分钟，FC 遵循度 **60%→100%**（0.6B）。结论：「工具调用能力靠环境反馈 RL，不靠模型规模」——与三篇论文完全一致。

**缺口 / 下一步**：现有 GRPO 数据是程序化生成的 118 条三类 prompt。简报三篇都强调「**真实轨迹 / answer-first 反向生成**」质量 > 数量。落地动作：把 `lernen.rs` 的 Experience→Skill 蒸馏接到训练数据生成器，用真实调用轨迹替代程序化数据（ToolGrad 式）。

### 1.6 A733 NPU 调度器（设计先行，板子 halt 不阻塞设计）

**简报观点 / 板子现实**（来自长期记忆）：Allwinner A733 的 Vivante VIP9000（3 TOPS INT8）**同一时刻只能跑一个网络**，多消费者必须自己串行排队。官方 YOLO26→ONNX→Pegasus→NBG 管线；Radxa 离线语音助手（KWS/ASR/TTS）已在 NPU 跑通。

**落地设计**（`lyco-npu-runtime` crate，Rust，纯设计+联调位，不依赖板子通电）：

```rust
pub struct NpuRuntime {
    lease: Mutex<Option<Lease>>,        // 全局单网络锁
}
pub struct Lease { pub priority: u8, pub owner: String }
impl NpuRuntime {
    pub fn acquire(&self, owner: &str, prio: u8) -> Result<Lease, Busy>;
    pub fn load(&self, nbg: &[u8]) -> Result<Network>;   // 加载 .nbg
    pub fn infer(&self, net: &Network, input: &[u8]) -> Result<Vec<u8>>;
    pub fn release(&self, lease: Lease);                 // 释放→下一个排队者
}
```

调度语义：优先级队列 + 租约超时（防止某技能霸占 NPU）。`vnn_identify` 的 executor 在调用 NPU 前 `acquire()`，用完 `release()`。这样 CPU 侧（Qwen 类 LLM）与 NPU 侧（视觉/语音 encoder）按 petayyyy 定论「hybrid」分工：NPU 吃视觉，空出 6 个 A55 核给 LLM。

> ⚠️ 板子工作已按用户指令 halt。此 crate 代码可先写、单测可先跑（用 mock backend），等用户物理断电重启 A7A 恢复后，再接真实 `/dev/galcore`。当前 6.6.98-4-aw2511 上「替换 galcore」路线已证伪（insmod 即 hard hang），真实联调应走 TIM-VX 用户态或 OrangePi 6.6.98-sun60iw2 内核——不在本次研究范围。

---

## 二、CloudStudio 作为云端算力后端（已核实规格）

**平台性质**（WebSearch 核实）：腾讯云 GPU 平台，session-scoped——**平台会杀掉非 kernel 的长跑进程**（DESIGN.md 已记）。结论：CloudStudio = 训练 / 评测专用，不做常驻推理服务。

**规格与抵扣因子**（首次绑定腾讯账号送 50 免费机时）：

| 机型 | 核 / 内存 / 显存 | 抵扣因子 | lyco 用途 |
|---|---|---|---|
| T4 | 8核 / 32G / 16G | 1.2 | 轻量 SFT |
| **A10** ⭐ | **20核 / 116G / 24G** | **3.3** | **主力：GRPO / 经验 SFT / BFCL+τ²-Bench 评测** |
| V100 | 8核 / 40G / 32G | 3.6 | 备选 |
| L40 | 48核 / 192G / 48G | 8 | 大数据闭环 |
| A800 | 124核 / 1929G / 80G | 14 | 全量预训练（不现实，仅记录） |

**分工架构（最终形态）**：

```
┌───────────────── 云端：CloudStudio A10 (session) ─────────────────┐
│  GRPO 训练 (tools/qwen_grpo_train.py)                            │
│  经验 SFT (lernen.rs 蒸馏出的 Experience → Skill corpus)          │
│  ToolGrad/Tool-Star 式数据闭环 (answer-first 反向生成)            │
│  评测: BFCL + τ²-Bench + 自建 lyv-knowledge-qa                    │
│        ↓ 产出量化 SLM (0.6B/1.7B Q4, ~0.3-1.1GB)                 │
└──────────────────────────┬───────────────────────────────────────┘
                           │ 下载到端侧
                           ▼
┌──────────── 端侧：手机 / PC (llama.cpp GGUF) ────────────┐
│  0.6B/1.7B Q4 SLM → lycore 路由 (Capability 裁决)        │
│  + ToolRAG 裁剪工具 schema → CHAT_TOOLS                  │
│  + Verifier 级联 (OCR/VNN) 确定性验收                     │
│  + 不会 → LearningQueue (诚实降级 + 回传云端下一轮训练)    │
│  + 升级档: Qwen3.8-27B Q4 (~16GB) 本地/PC 重问题          │
│  + 越权/超难 → escalation 到云端前沿模型                    │
└──────────────────────────────────────────────────────────┘
```

**端云协同闭环**：端侧每次 `LearningQueue::push()` 的真实失败轨迹，是云端下一轮 GRPO/经验 SFT 的**最高质量训练数据**（answer-first 反向生成的天然来源）。这正是简报「小模型靠工具调用 + 持续数据闭环追上大模型」在工程上的闭环实现。

---

## 三、DESIGN.md 两个已知 open issue 的处理建议

| # | Issue | 本研究建议 |
|---|---|---|
| ① | `index_lilyco_schema()`（learn_cli.rs:175）孤儿，零调用者 | 在 `index_and_build()` 末尾接一行 `index_lilyco_schema(pack)`，把 lyco 自身子命令也进同一 FTS5 表；同时作为 1.4 ToolRAG 的候选召回源 |
| ② | `lyco_chat` 不消费 `tools_openai.json`（无外部模型路径） | 产品决策点：是否让端侧小模型也走 `chat_tools()` 单一来源导出 schema？建议**是**——这样才能让 Capability 层 + ToolRAG 统一作用于所有档位。需用户拍板是否开放外部模型接入 |

---

## 四、与既有研究的衔接（不重复造轮子）

- `research-base-model-2026-09-10.md`：底座选型（R2 蒸馏路线）、GRPO 实测、竞品扫描——本研究**沿用**其结论，不推翻。
- `deepseek-training-guide-2026-09-10.md`：RL 方法论细节——本研究 GRPO/经验 SFT 部分与之对齐。
- `rewrite-grpo-v4-eval-2026-09-11.md`：GRPO 评测口径——本研究「训练闭环 + 评测」沿用其口径。
- 本研究**新增**的是：把上述单点验证提升为**类型系统（Capability/Skill/Verifier）+ 检索层（ToolRAG）+ 云端算力定位（CloudStudio A10）**。

---

## 五、落地任务清单（按优先级）

1. **P0 — Capability 枚举**（`lycore/src/capability.rs`）：定义 `enum Capability` + `TOOL_CAPS` 表，并入 `lib.rs`。
2. **P0 — Skill 描述符**（`lycore/src/skill.rs`）：`struct Skill { capabilities, risk, verifier, executor }`，把 `verify.rs`/`executor.rs` 接进来；补 `RiskLevel`。
3. **P1 — Verifier 注册表**（`verify.rs` 改造）：`Verifier` trait + 注册表；补 `html_render_video` 的 ffprobe 验证器；VNN Rust 路径占位（降级保持）。
4. **P1 — ToolRAG 召回层**（`learn_cli.rs` 改造）：FTS5 之上加 embedding Top-K；接 `index_lilyco_schema` 孤儿（fix issue①）；产出裁剪版 `tools_openai.json`。
5. **P1 — 数据闭环**（`lernen.rs` + `tools/qwen_grpo_train.py`）：LearningQueue 失败轨迹 → answer-first 反向生成 → GRPO/经验 SFT 输入；在 CloudStudio A10 跑。
6. **P2 — NPU 调度器**（`lyco-npu-runtime` crate）：纯设计+单测（mock backend），等板子恢复再联调真实 `/dev/galcore`（走 TIM-VX 用户态，避开 6.6 galcore hard hang）。
7. **P2 — escalation 接线**：Capability 层裁决越权/超难时升级到云端前沿模型（需确认外部模型接入策略，见 issue②）。

---

## 六、引用（已核实）

- ToolGrad — arXiv:2508.04086, ACL 2026, Google XR (Zhongyi Zhou, Ruofei Du)
- Tool-Star — arXiv:2505.16410, SIGIR 2026, RUC-NLPIR
- TinyAgent — arXiv:2409.00608, Berkeley SqueezeAILab
- CloudStudio GPU 规格 — T4/V100/A10/L40/A800 + 抵扣因子表（首次绑账号送 50 免费机时）
- 内部代码：`D:/Code/rust/lyco_agent/lycore/src/{verify,executor,learn_cli,llamacpp,tools_runtime}.rs`、`DESIGN.md`、`lib.rs`
- 内部既有研究：`docs/research-base-model-2026-09-10.md`、`docs/deepseek-training-guide-2026-09-10.md`、`docs/rewrite-grpo-v4-eval-2026-09-11.md`
