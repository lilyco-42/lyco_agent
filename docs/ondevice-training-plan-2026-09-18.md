# lyco_agent 端侧小模型：调研与训练方案

> 🗺️ **路径注记（2026-09-20 tools/ 重组）**：本文写于 tools/ 平铺时代。现按线分组：FC 线 → `tools/training/fc/`，改写线 → `tools/training/rewrite/`，VNN → `tools/training/vnn/`，评测/知识包 → `tools/training/bench/`，`lyv.py` 及 agent 原型 → `tools/runtime/`，板端 NPU/KWS → `tools/native_npu/`，systemd 单元 → `tools/deploy/`；`tools/cgidata/` 原地未动。文中旧路径按此映射，平铺快照见 tag `tools-flat-archive-2026-09-20`。

> 目标：为 lilyco / lyco_agent 框架训练一个**端侧可跑的小模型**，用于"驾驶"agent（规划、工具/技能选择、调用、错误恢复）。要求低功耗、可部署到手机 / 边缘（CPU 或 NPU）。
> 约束：**本机（Win）不编译任何 lyco_agent 模块**；统一在 CloudStudio（腾讯云 GPU）构建与训练。开发在 **A10** 工作空间进行。

---

## 0. 背景与硬约束（来自项目约定）

- **编译/训练只在 CloudStudio**：A10 工作空间 `04e7e16c9cac40dda427befd85ead378`（20核/116G/24G显存）；V100 `b0d0f5fb49b144bdbe9411862d1b3292`（liteApp `38102325662502912`）备用。
- **CloudStudio 平台杀非 kernel 长跑进程** → 只做 session-scoped 训练/评测，非常驻推理服务。
- **现有训练管线**：lyco_agent 仓库 `tools/qwen_grpo_train.py` 等（GRPO / 经验 SFT / 评测），在 CloudStudio 跑 CUDA。`lycore` 纯 Rust，`cargo build/test` 在 CloudStudio 即可验证（不需 CUDA）。
- **架构主张**（`docs/research-architecture-2026-09-16.md`）：Capability 一等公民、Skill 描述符 `{capabilities,risk,verifier,executor}`、Verifier 与模型解耦、Skill-RAG/Tool-RAG、**小模型 + 工具调用 > Scaling Law**、A733 NPU 调度器。
- **不要本地编译**：任何 build / train 走 A10；本机只写代码 + `git push`，CloudStudio 拉取构建。

---

## 0.5 仓库审计结论（2026-09-18 实测，修正方案）

`lilyco-42/lyco_agent` 已自带完整端侧训练栈，**首版直接复用，不重造**：

- **基座统一为 `Qwen/Qwen3-0.6B`**（仓库全部 trainer 的 `MODEL_ID`），已验证可在 A10 + 端侧跑通。
- **工具调用 GRPO**：`tools/qwen_grpo_train.py`（3 工具，自包含生成数据，200 步，`lr=1e-5`）；`tools/fc_grpo_v4.py`（扩到 7 工具，从 `grpo_v3/final` 续训，缺前驱 fail-loud，`lr=5e-6`）。调用格式 `<tool_call>{"name":...,"arguments":{...}}</tool_call>`（正则解析）；reward = 调对工具 +1、参数关键词命中 +0.5、该闲聊不调 +1、乱调/坏 JSON 0。system prompt 含 "running on a Radxa SBC"。
- **意图→CLI 路由器 SFT**：`tools/sft_cli.py`，数据 `tools/cgidata/{train,eval}.jsonl`（真实 `hw cpu` / `hw gpio set 0 370 1` 等板端命令语料），assistant-only loss，5 epoch，`lr=2e-5`，评测=命令精确匹配 + 该不调拒绝。
- **数据生成**：`tools/cli_datagen.py`、`tools/fc_grpo_corpus.py`、`tools/datagen.rs`、`lycore/src/datagen.rs`；现成评测 `tools/fc_eval_holdout.py`、`tools/lyco_bench*.py`。

→ 首版"驾驶模型"= `qwen_grpo_train.py`（工具选择能力，自包含）+ `sft_cli.py`（板端 CLI 路由，用现成 cgidata）。两者产物都是 Qwen3-0.6B，量化后即可端侧闭环。`fc_grpo_v4.py` 扩工具集作为第二阶段。

## 1. 端侧低功耗路线对比（2026 现状，已调研）

### 路线 A：Qwen 小模型（成熟、推荐作基座）
- 候选：Qwen3-0.6B / 1.7B / 4B；Qwen2.5-0.5B / 1.5B / 3B。
- 优点：原生 tool-calling、多语言、生态成熟；GGUF INT4 经 `llama.cpp` 在手机 CPU 已验证（树莓派5 纯 CPU 5+ t/s；高通 AI Hub 有骁龙 8 Elite NPU 版 Qwen3-0.6B Q4_0，27.7 t/s 解码）。
- 训练：LoRA / 全参 SFT / GRPO 都成熟，lyco_agent 现有 `qwen_grpo_train.py` 直接复用。

### 路线 B：BitNet 1.58-bit（最低功耗）
- 微软 BitNet b1.58（三值 -1,0,1），`bitnet.cpp` 官方推理引擎：CPU 上**省电 55–82%、提速 1.37–6.17x**（ARM/x86 都覆盖），NPU 支持"coming soon"。
- 训练：from-scratch，或把现成模型（含 LLaMA/Qwen）经 **gradual fine-tuning** 转成 1.58-bit。
- 缺点：训练/转换工具链较新、模型生态小、NPU 暂不支持；agent/tool-calling 行为在 BitNet 基座上微调经验少。

### 推荐：混合路线（满足"qwen 小模型可以 + bitnet 低功耗也要"）
1. 在 A10 上以 **Qwen3-0.6B** 为基座（**仓库已验证的端侧基座**，手机可跑、BitNet 友好、训练快），做 agent 驾驶能力的 SFT + GRPO（复用现有管线）。
   - *容量升级可选*：若 0.6B 驾驶能力不够，再上 **Qwen2.5-1.5B / Qwen3-1.7B** 做 ablation 对比，但首版端侧交付先用 0.6B。
2. 部署时按需量化：
   - **成熟路径**：GGUF `Q4_K_M`（~400MB–1GB）→ `llama.cpp` 手机 CPU（功耗可控、生态成熟）。
   - **极致低功耗**：转 **BitNet 1.58-bit** → `bitnet.cpp`（省电 55–82%，CPU 无浮点矩阵乘）。
3. 先交付 Q4_K_M 版本跑通端侧闭环，再追加 BitNet 转换拿最低功耗做功耗对比。

---

## 2. 训练数据

- **Agent 驾驶轨迹**：规划 / 工具选择 / Skill 调用 / 错误恢复的 (state, action, reward) 样本，来自 lyco_agent 现有 trace 或合成。
- **工具调用格式数据**：function-calling schema 对齐 lyco_agent Skill 描述符 `{capabilities,risk,verifier,executor}`。
- **评测集**：agent 成功率、工具调用格式正确率、端侧延迟/功耗。
- 数据准备放在 A10（大文件不下本地）。

---

## 3. 训练管线（A10，session-scoped，复用仓库脚本）

- **环境**：CloudStudio A10（GPU=NVIDIA A10 24G，workspace `b0d0f5fb49b144bdbe9411862d1b3292`），Python venv，`.venv/bin/pip install torch transformers trl peft accelerate datasets bitsandbytes`（torch wheel 自带 CUDA 12.x）。脚本经 JPS python3 内核跑，平台只杀非 kernel 进程 → 训练必须在 kernel 执行内完成。
- **首版两条训练（均自包含，无需外部数据）**：
  1. `tools/qwen_grpo_train.py` — 工具选择 GRPO（3 工具，自生成 90+ 条 prompt，200 步，`lr=1e-5`，`num_generations=4`，`max_completion_length=256`，`bf16`，`use_vllm=False`）。产物 `/workspace/qwen3_lyco_grpo`。
  2. `tools/sft_cli.py [data_dir]` — 意图→CLI 路由器 SFT，吃 `tools/cgidata/{train,eval}.jsonl`（板端真实命令），5 epoch，`lr=2e-5`，assistant-only loss。产物 `/workspace/qwen3_router_v1`。
- **扩工具（第二阶段）**：`tools/fc_grpo_v4.py`（7 工具，需先有 `grpo_v3/final`；可改 `BASE_MODEL` 指向 `qwen3_lyco_grpo/final` 续训）。
- **超参已验证适配 24G**：Qwen3-0.6B bf16 ≈ 1.2GB；GRPO `per_device_train_batch_size=8 × num_generations=4 = 32` completions/step，单卡 24G 宽松；`lr=1e-5~5e-6`。
- **产物**：HF 格式 checkpoint（`save_model`/`save_pretrained`）→ 量化。
- **注意**：仓库脚本写死 `/workspace/...` 输出路径，训练在 CloudStudio 容器内跑（HOME/workspace 即容器），本机不落模型文件。

---

## 4. 量化与部署

- **Q4_K_M**：`llama.cpp/convert_hf_to_gguf.py` + `llama-quantize` → GGUF → 手机 `llama.cpp` / Ollama / `llama-cpp-python`。
- **BitNet 1.58-bit**：`microsoft/BitNet` 仓库 `setup_env.py -q i2_s` 转换 → `bitnet.cpp` 推理（CPU 极致省电）。
- **NPU（可选）**：骁龙 8 Elite 用 Qualcomm AI Hub 的 Qwen3-0.6B Q4_0 已验证；A7A NPU（A733/VIP9000）见 `petayyyy/a733_npu_driver`（vision/CNN 已跑通，LLM 类受限，通用 batched MatMul 不可用）。

---

## 5. 评测

- **功能**：agent 驾驶成功率、工具调用格式正确率、Verifier 对齐率。
- **端侧**：手机 / 树莓派5 上 `llama.cpp` 的 t/s、首 token 延迟、内存峰值、功耗（**Q4_K_M vs BitNet 1.58-bit 对比**）。

---

## 6. 执行步骤（分阶段，2026-09-18 推进中）

- **阶段 0**：确认路线与基座（Qwen3-0.6B）。✅ 方案已定。
- **阶段 1（进行中）**：工作空间 `b0d0f5fb49b144bdbe9411862d1b3292`（GPU=NVIDIA A10 24G）已在控制台启动；已 clone `lyco_agent`；环境分两段装：stage1=torch（后台跑中），stage2=transformers/trl/peft/accelerate/datasets/bitsandbytes。
- **阶段 2**：数据集**已有**——`tools/cgidata/{train,eval}.jsonl`（板端 CLI 语料）+ `qwen_grpo_train.py`/`fc_grpo_v4.py` 自生成 prompt。无需额外准备；如需扩工具集再跑 `fc_grpo_corpus.py`/`cli_datagen.py`。
- **阶段 3（待环境就绪）**：在 A10 上跑（均经 `.venv/bin/python`）：
  - `python tools/qwen_grpo_train.py` → `/workspace/qwen3_lyco_grpo`
  - `python tools/sft_cli.py` → `/workspace/qwen3_router_v1`
  - （扩工具）`python tools/fc_grpo_v4.py`（设 `BASE_MODEL=/workspace/qwen3_lyco_grpo/final`）
- **阶段 4**：量化 `Q4_K_M`（`llama.cpp/convert_hf_to_gguf.py` + `llama-quantize`）→ 端侧 `llama.cpp` 闭环验证（t/s、首 token、内存峰值）。
- **阶段 5（可选）**：转 BitNet 1.58-bit（`microsoft/BitNet` `setup_env.py -q i2_s`）→ 与 Q4_K_M 做功耗对比；决定是否默认走 BitNet。

---

## 7. 风险与开放问题

- A10 工作空间需用户**手动启动**（CloudStudio API 无启动端点）。
- BitNet 转 agent 模型的成熟度待验证（阶段 5 后再定）。
- 训练数据需从 lyco_agent trace 抽取/合成（阶段 2 重点）。
- 基座最终选 Qwen2.5-1.5B vs Qwen3-1.7B 待阶段 1 后在 A10 上做小规模 ablation 决定。
