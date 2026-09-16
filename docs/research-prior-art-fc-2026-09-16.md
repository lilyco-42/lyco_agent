# 先例调研：我们想要的（端侧小模型 + 工具调用 + 该不调 + 经验复用）别人做过没有？

> 日期：2026-09-16 | 触发：用户指令「先查相关论文或 github 项目，一定有人会有这种我们想要的东西，解决了」
> 方法：`gh search repos`（近义词循环 + topic）+ HF dataset API + WebSearch 论文核实。**每条候选均有源**。
> 结论：**大部分已被解决，且是开源的**。我们的架构 = 若干成熟组件的组合 + 一小块无人区。见「决策」。

---

## 一、需求（PREPARE，一行）
在端侧（手机/PC/边缘）用 0.6~1.7B 小模型做**可信工具调用**：该调才调、不该调不调（相关性/abstention）、
结果由**确定性 Verifier** 验收、成功经验**沉淀复用**（experience→skill），且**不出网/隐私优先**。

---

## 二、候选清单（均实测，非臆造）

### A. 端侧 Function Calling（核心）
| 仓库/资源 | ★ | 许可 | 最近推送 | 覆盖我们的什么 |
|---|---|---|---|---|
| **MadeAgents/Hammer** | 123 | Apache-2.0 | 2025-06-13 | ⭐ **on-device FC + 相关性/该不调（irrelevance）+ function masking** —— 正治"闲聊误调" |
| `MadeAgents/Hammer2.0-0.5b/1.5b/3b/7b` (HF) | — | cc-by-4.0 | — | 现成 0.5B 端侧 FC 模型，可直接对照/蒸馏 |
| **SqueezeAILab/TinyAgent** | 498 | MIT | 2024-09-04 | 边缘 FC + **ToolRAG**（我们 ToolRAG 的原始论文） |
| **SalesforceAIResearch/xLAM** | 638 | Apache-2.0 | 2026-06-02 | action model 训练法 + 数据（60k） |
| `Salesforce/xlam-function-calling-60k` (HF) | — | cc-by-4.0 | — | 多样性主料（Hammer 就在它上面训） |
| `MadeAgents/xlam-irrelevance-7.5k` (HF) | — | cc-by-4.0 | — | ⭐ **7.5k"该不调"样本**（工具集无关，可迁移） |
| `NousResearch/hermes-function-calling-v1` (HF) | — | apache-2.0 | 2026-01-03 | 格式与我们一致（`<tool_call>{…}</tool_call>`） |
| **QwenLM/Qwen-Agent** | 17100 | Apache-2.0 | 2026-03-04 | Qwen 官方 FC 框架/模板约定 |

### B. 经验→技能复用（我们 lernen/datagen 的同类）
| 仓库 | ★ | 许可 | 最近推送 | 覆盖 |
|---|---|---|---|---|
| **MineDojo/Voyager** | 7202 | MIT | 2024-04-03 | ⭐ **Minecraft 里**的开放式 agent：**经验→可复用技能库**（我们的飞轮已在 Minecraft 被验证过） |
| **lsdefine/GenericAgent** | 14198 | MIT | 2026-09-14 | 自演化 **skill tree**（从 3.3K 行种子长成） |
| **titanwings/distilly** | 24786 | MIT | 2026-09-16 | 把"怎么想的"蒸馏成**可复用 Skill** |

### C. 评测/度量（我们缺的 abstention 口径）
| 资源 | ★ | 许可 | 说明 |
|---|---|---|---|
| **facebookresearch/AbstentionBench** | 91 | NOASSERTION(查) | LLM **abstention 基准** —— 正是"该不调"的量化口径 |
| lhannnn/agentic-abstention | 43 | — | agentic abstention |

### D. 工具检索/规模（若 Skill 数上千）
| 仓库 | ★ | 许可 | 说明 |
|---|---|---|---|
| **ComposioHQ/composio** | 30197 | MIT | 1000+ toolkits + **tool search** + context 管理（我们 ToolRAG 的工业化版本） |

---

## 三、无人区（我们的护城河，无先例）
1. **视频 → LVK 知识包**（切片+关键帧+OCR 证据）—— 竞品扫描（research-base-model §六）确认无同赛道。
2. **确定性 Verifier 级联**接**领域产物**（ffprobe/文件存在/exit code，`verify.rs`）。
3. **A733 NPU 多后端调度**（`npu_runtime`，租约+优先级）。
4. **跨设备同源能力**（一次训练→多档量化）+ 端云 escalation 成本阶梯。

---

## 四、决策（build-vs-buy）
| 层 | 决策 | 依据 |
|---|---|---|
| FC 训练/相关性 | **ADOPT（克隆研究，不自造）** | Hammer 覆盖 ≥80%：已有 on-device FC + **irrelevance 数据** + function masking；Apache-2.0 可商用 |
| 训练数据 | **ADOPT** xLAM-60k(多样性) + xlam-irrelevance-7.5k(负样本) | cc-by-4.0，需署名 |
| 经验→技能 | **FORK-EXTEND**：对照 Voyager/GenericAgent 补齐 `lernen.rs` | 我们已有同类实现，属改进而非新建 |
| 评测口径 | **ADOPT AbstentionBench 思路**（held-out + abstention 指标） | 修"评测污染"必须 |
| 视频知识/Verifier/NPU | **BUILD（无人区）** | 无先例 |

**ROI**：产出=直接解决闲聊误调 + 拿到可信评测；投入=克隆抄实现 + 数据适配（远低于自造数据集/自研算法）。

---

## 五、下一步（单个动作）
`git clone` **Hammer**（只读研究其 function masking + irrelevance 训练流程）与 **Voyager**（技能库设计），
把 `xlam-irrelevance-7.5k` 抽样接入 `datagen` 负样本，先做**评测隔离**再训。

## 六、引用
- Hammer — arXiv:2410.04587（ICLR 2025）, https://github.com/MadeAgents/Hammer
- TinyAgent — arXiv:2409.00608（EMNLP 2024 Demo）, https://github.com/SqueezeAILab/TinyAgent
- xLAM — https://github.com/SalesforceAIResearch/xLAM
- Voyager — https://github.com/MineDojo/Voyager
- GenericAgent — https://github.com/lsdefine/GenericAgent
- AbstentionBench — https://github.com/facebookresearch/AbstentionBench
- Composio — https://github.com/ComposioHQ/composio
- HF datasets — `MadeAgents/xlam-irrelevance-7.5k`、`Salesforce/xlam-function-calling-60k`、`NousResearch/hermes-function-calling-v1`
