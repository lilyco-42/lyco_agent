# 同类工作横向对比：本地「自然语言 → 命令」小模型（2026-09-19 lyco 预研）

**需求（一行）**：找出做过同类工作的项目（小参数端侧模型 + NL→命令/工具调用 + 量化 GGUF），并横向对比。
**检索面**：GitHub（gh CLI 同义词循环 8 组）+ HuggingFace（模型/数据集 API）。

> ⚠️ **对比纪律**：三者任务域、评测集、指标定义都不同，**绝对准确率不可直接比**。
> 可比的只有：产物体积、延迟、评测方法与严谨度、社区采用度。下述表格已标注不可比项。

## 1. 候选清单（全部来自真实检索结果）

| 项目 | 采用度 | 许可 | 体积/基座 | 任务域 | 评测方式 |
|---|---|---|---|---|---|
| **`ThorOdinson246/whatisit-nl2sh`**<br>（HF: `nl2sh-1.5b-Q4_K_M`） | **601★ / 53,807 下载** | Apache-2.0 | **941 MB** Q4_K_M / Qwen2.5-Coder-1.5B-Instruct | 英文 → 通用 shell（开放域） | **执行式**（InterCode-ALFA，容器执行后比对文件系统/stdout） |
| **`ValerieLuo2003/RobustTool-SLM`** | 1★（新） | 无 | LoRA / Qwen2.5-1.5B-Instruct | 英文 → Calendar 工具调用（多步状态依赖） | **分层 + 环境重放**，含失败分类 |
| `TellinaTool/nl2bash` | 519★ | **GPL-3.0 ⚠️** | TF seq2seq（2018，arXiv 1802.08979） | 英文 → bash | 学术基线数据集 |
| `BuilderIO/ai-shell` | 5295★ | MIT | 无本地模型（调云 API） | 英文 → shell | 无（非模型项目，仅作对照） |
| `mradermacher/Qwen2.5-Coder-0.5B-Instruct-NL2SH-GGUF` | 427 下载 | — | **0.5B**（与我们体量最接近） | 同 nl2sh | 派生量化 |
| `tiiuae/Falcon-H1-Tiny-Tool-Calling-90M-GGUF` | 1955 下载 | — | **90M** | 工具调用 | — |
| `SmolQwen/functiongemma-270m-it-simple-tool-calling` | 674 下载 | — | **270M** | 工具调用 | — |
| `spinozans/emender-e97-1.3b-cli-agent` / `samueljohn/Llama-3.2-1B-CLI-Agent-GGUF` | 189 / 131 下载 | — | 1.3B / 1B | CLI agent | — |
| **我们** `lyco42/lyco-agent-qwen3-0.6b-ondevice` | 刚发布 | Apache-2.0 | **484 MB** Q4_K_M / Qwen3-0.6B | **中文 → `hw` 板端命令**（13 类）+ **拒绝语义** | **字符串精确匹配** + 2 值拒绝检查（**弱**） |

## 2. nl2sh 的硬指标（唯一有可比口径的数字）

评测：InterCode-ALFA，300 任务，temp 0，64-token 预算，**逐任务配对比较**。

| 模型 | 体积 | pass rate |
|---|---|---|
| GPT-4o（云端，官方公布） | — | 0.73 |
| **nl2sh-1.5b（微调后）** | **941 MB** | **0.620** |
| Qwen2.5-Coder-7B（未微调） | 4.4 GB | 0.613 |
| Qwen2.5-Coder-1.5B（同基座未微调） | 941 MB | 0.540 |

- **微调增益**：0.540 → 0.620，配对 **+0.080，p = 0.004**（exact McNemar）→ 显著。
- **与 5× 大模型无统计差异**：0.620 vs 0.613，差 0.007，95% CI [−0.050, +0.063]，p = 0.91
  （作者自己注明：300 题只能排除 >5 点的差距，是**上界而非等价证明**）。
- 训练：**LoRA r=32, α=64**，**125,770** 条 NL/shell 对，合并后量化 Q4_K_M。

## 3. RobustTool-SLM 的等规模对照 —— 直接解释了我们踩的坑

| 模型 | Clean Task Success (1000) | Robust Task Success (500) | Robustness Gap (配对) | Recovery Success |
|---|---|---|---|---|
| Base | 63.00% | 38.40% | 29.20pp | 4.00% |
| **Recovery-aware v2（定向失败数据）** | 90.10% | **85.00%** | **9.20pp** | **92.00%** |
| **Random Augmentation v2（等规模 12000 条）** | **92.80%** | 65.80% | 29.00pp | **0.00%** |

**结论（他们的原话）**：提升主要来自**针对恢复行为的定向数据增强，而不是单纯增加 3000 条训练样本**。
Random-v2 在 Clean 上甚至更高（92.8% > 90.1%），但 Robust 只有 65.8%、恢复率 **0%**。

**这几乎逐字复现了我们的经历**：我们的"扩说法/加语气词"= 他们的 Random Augmentation ——
clean 上漂亮、换说就崩（我们 heldB 卡在 ~80%）。而我们的 `T_FAIL`（失败回放）
本应对应他们的 Recovery 数据，但**我做得太容易**（起始 reward 0.886、65–80% 采样组零方差），
没构成 hard case。**他们是从 SFT 验证集里挑 Top-3 高频失败再针对性造 3000 条**——有失败分类法驱动。

## 4. 他们比我们严谨的地方（可直接拿来）

1. **执行式评分，而非字符串匹配**：nl2sh 用容器执行后比对文件系统/stdout；RobustTool-SLM
   **在全新环境重放轨迹**，且明确"`Trajectory` 里模型自报的 `final_state` 不可信"。
   → 我们目前是字符串精确匹配，且**没有防伪机制**。
2. **失败分类法（15 个确定性多标签）**：`wrong_call_decision` / `wrong_tool` /
   `missing_argument` / `wrong_argument_value` / `invalid_json` / `hallucinated_tool` /
   **`unnecessary_tool_call`** / `ignore_tool_result` / `tool_error_recovery_failure` /
   `clarification_failure` … → 我们只有"对/错 + 该不调"二分。
3. **统计严谨**：配对比较 + exact McNemar + 置信区间；**多种子入口**。
   → 我们的结论还停留在"±3pp 噪声"的自觉，没做显著性检验。
4. **等规模对照组**：证明"定向数据 > 加量"必须**匹配训练规模**才有说服力。
5. **不漏样本**：Task 与 Trajectory 必须一一对应，缺失/多余/重复 `task_id` 直接报错，防指标虚高。

## 5. 独立复现的一个坑（我们踩过同款）

nl2sh 模型卡原文：
> Earlier versions of this card used `llama-cli -no-cnv` … Upstream split raw completion out of
> `llama-cli` into a separate `llama-completion` binary in **December 2025**, and **`-no-cnv` is now
> accepted but ignored, so that command returns nothing at all**. Use `-sys`/`-st` as above.

→ 与我们今早的遭遇同源（我们的版本直接报 `invalid argument`）。**这是上游 2025-12 的破坏性变更**，
不是我们的环境问题。正确姿势：`-sys` + `-st`（或改调 `llama-completion`）。
他们还给了可直接抄的采样参数：`--temp 0 -n 64 --repeat-penalty 1.08 --repeat-last-n 64`。

## 6. 决策（lyco REASON/ACT）

| 维度 | 判断 |
|---|---|
| 同类先例覆盖度 | **概念层 ≥80% 已有人做过**（本地小模型 NL→命令 + GGUF + CPU）→ **不自研评测/训练框架** |
| 我们的差异化（真实的硬约束） | ①**中文 → `hw` 板端命令**（13 类固定能力，非开放域 shell）；②**0.6B / 484 MB**（比 nl2sh 小一半）；③**拒绝语义**「该不调就不调」是一等公民 |
| 决策 | **adopt / fork-extend 方法论，保留差异化域**：把 RobustTool-SLM 的**分层评测 + 失败分类法 + 等规模对照**移植到我们的 `hw` 域；nl2sh 只作**产品化参考**（pip/nix/CI/doctor + 危险命令告警） |
| 许可风险 | `TellinaTool/nl2bash` 是 **GPL-3.0** → **不可拷代码**，只能当论文基线引用；nl2sh / RobustTool-SLM 为 Apache-2.0 / 无许可（后者需谨慎，只借鉴方法不拷实现） |

## 7. 下一步（单一动作）

**克隆 `RobustTool-SLM`（已拉至 `D:/Code/_priorart/RobustTool-SLM`，407 KB）并读 `docs/evaluation.md`
+ `docs/failure_taxonomy.md` + `docs/final_test_report.md`，把它的 15 标签失败分类法映射到我们的
`hw` 域**，据此重做两件事：
1. 用**失败分类法**跑一次我们 router 的失败分桶（现在不知道错在哪几类）；
2. 设计**等规模对照**：`定向失败数据` vs `随机扩充`，用配对检验（McNemar）而非看 ±3pp 差值下结论。

> 注意：这是**方法论移植**，不是拷代码；我们的域和体量是差异化的，且他们的 GRPO 尚未跑（"待 GPU 运行"），
> 我们在这一点上反而先行了一步。
