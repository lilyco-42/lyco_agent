# 热门模型调研：端侧小模型 × 工具调用（2026-09-20）

> 触发：用户指令「去调研一下别人的热门模型」。目的：为 v12 训练与 v13+ 底座换型提供外部依据。
> 结论先行：**v12 底座不动（Qwen3-0.6B）**，配方改单阶段 bash 回放；**v13 对照 Qwen3.5-0.8B**；
> **FunctionGemma-270M 列为 v14 极小底座候选**。外部证据直接印证了我们的两条方法论。

## 1. FunctionGemma 270M（Google）—— 我们最直接的同行

定位（官方原话）：*"intended to be fine-tuned for your specific function-calling task"* ——
**不是拿来就用的产品，是给定向数据配方当底座的**。与 lyco_agent 路由器（v8~v11）完全同赛道。

| 指标 | 数值 |
|---|---|
| BFCL 单轮 Simple / Multiple / Irrelevance（拒绝） | 61.6 / 63.5 / **73.7** |
| 移动操作微调 | 58% → **85%** |
| 端侧（S25 Ultra CPU, dynamic_int8） | 288MB，decode **125.9 t/s**，TTFT 0.3s |

**Distil Labs 微调实验（对 v12 最重要的外部数据）**：
- Shell 命令执行（Gorilla）：9.9% → **96.0%**（5000 条合成数据，GPT-oss-120B 教师蒸馏）
- 智能家居 38.8→96.7；银行语音 23.4→90.9
- **同一条数据配方在 Qwen3-0.6B 上也出了强模型**——原文："The data is doing the work,
  not per-model tuning"。→ **我们 v8 以来的「比成分固定总体积、定向数据」方法论被外部印证**。
- 多轮衰减算术：单轮 80% → 5 轮 33%；95% → 77%。单轮指标对多轮 agent 是误导——
  我们的路由器是单轮，但接入 agent loop 后同算术适用 → 单轮 ≥90% 是实际门槛。

**对我们的差距信号**：Distil 的 shell 配方到 96%，我们 bash_held 8.3%（v10）/0%（v11）。
数字不可直接比（我们评测=模板族+值域互斥+0 泄漏断言，且蒸馏 5k vs 真实语料 8k），
但方向提示：**数据质量（教师过滤/执行验证）可能比语料规模更重要**——v12 之后若 bash 仍弱，
考虑用执行反馈（IC-ALFA 思路）过滤回放集，而非加大 N。

## 2. Qwen3.5 Small 系列（2026-03-02，Apache 2.0）—— v13 底座候选

四款：**0.8B / 2B / 4B / 9B**，全原生多模态（文本+图+视频同权重），262K 上下文。

| 项 | 0.8B | 4B |
|---|---|---|
| 层数 | 24 | 32 |
| BF16 显存 | ~1.6GB | ~8GB |
| 默认模式 | **non-thinking**（适合路由器） | both |
| 官方定位 | 原型/任务微调/轻端侧 | 轻量 agent 基座 |

架构：**Gated DeltaNet 线性注意力 + full attention 3:1 混合**（3×(GDN→FFN)→1×(Gated Attn→FFN)）、
MTP 多 token 预测、248K 词表、201 语言。9B 超 Qwen3-30B 大部分指标、GPQA 超 Qwen3-80B。

**工程可行性（决定性）**：
- **llama.cpp 已支持**（b8200+，PR #19435，2026-02 起；MoE 投机解码 PR #19493 已并）→
  我们的端侧 GGUF 链路无架构阻碍。
- ⚠️ **GDN 坑：KV cache 必须 bf16**（`--cache-type-k/v bf16`，f16 会出错误结果——
  KiCAD-MCP-Qwen3.5-4B 模型卡实锤）。lycore `llamacpp.rs` 启动参数要记死这条。
- ⚠️ 训练侧：transformers 对 Qwen3.5 GDN 支持已就绪（4B LoRA 微调案例：ms-swift + 2×3090），
  0.8B 全参 SFT 显存无压力（A10 富余）。

**生态实证**：已有人把 Qwen3.5-4B LoRA 微调成 159 工具 MCP 选择器（14/14 全对）——
GDN 架构微调 + 工具选择任务已被走通。

## 3. 端侧小模型生态速览（2026-09）

- **Gemma 4 e2b/e4b**（2026-06）：2B-4B 级公开榜第一（原生多模态、Apache 2.0、~1.5GB Q4）。
  但无专用 tool-call token，结构化输出靠 prompt 约束 → 作 agent 路由器不如 Qwen 系可靠。
- **FunctionGemma**（Gemma 3 270M 底座）：专用工具调用，见 §1。
- **LFM2.5 350M**：Distil 实测中起点比 FunctionGemma 高 2-6x（银行语音 95.9%）→ 极小底座第一候选。
- **BFCL 小模型榜**：Llama-3-Groq-8B-Tool-Use 89.06%（8B 级天花板，非端侧优先）。
- Home Assistant 生态验证过工具调用：Qwen3 4B/8B、Phi-4-mini、Llama 3.2 3B。

## 4. 对我们路线的映射（决策记录）

| 轮次 | 决策 | 依据 |
|---|---|---|
| **v12（已备好脚本）** | Qwen3-0.6B 底座不变；单阶段混合 = v9b 配方 4100 + ALFA bash 回放 4000（≈1:1）×4ep | v11 证伪两阶段（遗忘）；变量控制只改「混合 vs 分离」 |
| **v13** | 底座对照 **Qwen3.5-0.8B**（同配方同评测） | 官方定位即微调底座；non-thinking 默认；262K；llama.cpp 就绪；GDN 效率利于端侧 |
| **v14（候选）** | 极小底座 FunctionGemma-270M / LFM2.5-350M，覆盖 hw/gh 窄任务 | 270M@288MB、125 t/s 手机 CPU；若 v12/v13 验证「数据>底座」，270M 级足够窄任务 |
| 配方演进 | bash 回放集可加执行验证过滤（IC-ALFA） | Distil 96% vs 我们 8.3% 的差距信号：质量>规模 |

**风险备忘**：Qwen3.5 训练前先跑 10 步冒烟（GDN 在 transformers 的 loss 数值检查）；
评测若换底座，Qwen3 模板约定（enable_thinking=False / 剥 think 块）需按 Qwen3.5 模板重新核对。
