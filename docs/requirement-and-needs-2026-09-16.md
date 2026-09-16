# 需求与所需（推荐版，2026-09-16）

> 触发：用户 `/lyco 你推荐 我的需求 和需要`。本文即**需求基线（requirement of record）**，
> 依据本会话全部实证：v3 held-out 94%、先例调研（Hammer/TinyAgent/Voyager/distil-labs/Unsloth）、
> 修正后的护城河判断。**所有数字都有源，见文末。**

## 一、推荐需求（一行 + 验收标准）

> **在 7 个固定工具上，做一个 0.6B 端侧 agent 控制器：held-out 工具调用 ≥90%、abstention ≥95%，
> 并从同一次训练同源导出 PC(GGUF) 与手机(.pte)，全程可复现。**

| # | 验收标准 | 现状 |
|---|---|---|
| 1 | held-out **≥100 例**（8 工具问法/8 负样本 × 多批） | ❌ 仅 16 例（统计力不足） |
| 2 | 工具选择 ≥90% / abstention ≥95% / 总体 ≥92% | ⚠️ 16 例上 88%/100%/94% |
| 3 | PC：GGUF 跑通并复测不掉分 | ✅ v3 q4km 已产出 |
| 4 | 手机：`.pte` 导出成功并复测，掉分 ≤3pp | ❌ 未做 |
| 5 | 全程命令可复现（脚本 + 日志 + 版本） | ⚠️ 部分（脚本已入库） |

## 二、你**需要**的（按优先级）

| 优先级 | 需要 | 为什么 | 成本 |
|---|---|---|---|
| P0 | **held-out 扩到 100+ 例并重测** | 现在所有结论都建在 16 例上；**度量不准，后面全是自欺**（v1 的"88%"就是教训） | 1 次 A10，~15min |
| P1 | run B：接 `xlam-irrelevance-7.5k`（Hammer） | 唯一可能再涨的**数据变量**，单独验证 | 1 次 A10 |
| P1 | run C：Unsloth QAT + ExecuTorch → `.pte` | 一次拿到"手机也能跑"（你选的是"一切设备"） | 1 次 A10 + 导出 |
| P2 | **窄缝自研**：视频→可执行操作知识包 + 确定性 Verifier | 这是你**唯一**的差异化，其余都已被开源覆盖 | 持续 |
| P2 | 端云 escalation（issue②） | 检索→云端前沿的成本阶梯，需你拍板外部模型接入 | 待决策 |

## 三、你**不需要**的（减法，同样重要）

- ❌ **更大的模型（1.7B）**：0.6B 已验证足够；瓶颈是配方与数据（distil-labs 同款 0.6B 达 100%）。
- ❌ **自研训练算法**：GRPO/冷启动 SFT 是 DeepSeek-R1 公开配方；直接抄。
- ❌ **自研部署栈**：Unsloth QAT + ExecuTorch 是标准答案（0.6B 472MB @ ~40 tok/s）。
- ❌ **自研 ToolRAG / 技能检索**：TinyAgent/Gorilla/Composio 已解决。
- ❌ **为"一切设备"写多端 runtime**：它=一次训练 + 两个导出（.pte / gguf）。

## 四、决策（build-vs-buy）

| 层 | 决策 | 依据（源） |
|---|---|---|
| FC 训练 + 该不调 | **ADOPT** Hammer（function masking + irrelevance） | github.com/MadeAgents/Hammer (Apache-2.0) |
| 训练配方 | **ADOPT** 冷启动 SFT + loss masking + 负样本、GRPO | DeepSeek-R1 Nature 版；本仓 `exp-fc-recipe-holdout-2026-09-16.md` |
| 语料 | **ADOPT** xLAM-60k / hermes-fc-v1 / xlam-irrelevance-7.5k | HF（cc-by-4.0 / apache-2.0） |
| 端侧部署 | **ADOPT** Unsloth QAT(phone-deployment) + ExecuTorch；PC 用 llama.cpp | unsloth.ai/docs（0.6B ~40 tok/s） |
| 经验→技能 | **FORK-EXTEND** 对照 Voyager / GenericAgent | github.com/MineDojo/Voyager (MIT) |
| **视频→可执行知识包 + Verifier** | **BUILD**（窄缝；视频问答已有 VideoRAG，我们不撞） | 本仓 `research-prior-art-fc-2026-09-16.md` |

## 五、ROI 与下一步

- **ROI**：P0（扩 held-out）产出=**可信度量**（后续一切结论的地基），投入≈15 分钟 A10 → **门槛轻松过**。
- **下一步单个动作**：**把 held-out 扩到 ≥100 例（含 50 工具 + 50 负样本），先只重测 v3，不改训练**。
  理由（信条 5）：**先把度量做对，再谈涨分**；否则 run B/C 的涨跌都读不出来。

## 六、源
- v3 实验：`docs/exp-fc-recipe-holdout-2026-09-16.md`（held-out 62%/50%→94%）
- 先例调研：`docs/research-prior-art-fc-2026-09-16.md`（Hammer/TinyAgent/Voyager/distil-labs/Unsloth/VideoRAG）
- 架构总纲：`docs/research-architecture-2026-09-16.md`
