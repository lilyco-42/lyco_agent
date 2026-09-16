# 实验：端到端跑通 + 定位「Q4 量化崩塌」根因（2026-09-16）

> 触发：用户 `/lyco 反正一定要能用, 随便你抄别人怎么做` → 需求锁定「必须能用」。
> 决策：**抄标准做法** —— `llama.cpp llama-server`（OpenAI 兼容 + 原生 `tools`）当服务端，
> `lycore` 自己的 agent loop 当编排器，工具 schema 用 `lycore tools --out`（单一真源）。
> 平台：CloudStudio A10。

## 一、端到端结果（llama-server + lycore agent loop）

| 任务 | 结果 |
|---|---|
| A「怎么启动 paper 服务器」 | ✅ rounds=2，tool_calls=[lyv_knowledge]，答案带证据（`java -Xmx2048M -jar paper.jar nogui`）—— **完整闭环** |
| B「帮我写一个 8:00-22:00 自动启动 Minecraft 的脚本」 | ❌ `answer="None"`（initial） |
| C「你好呀」 | ✅ 不调工具，正常回答 |

→ A 证明**链路是通的**；B 暴露一个真 bug。

## 二、根因定位（过程中差点误判 —— 控变量救场）

排查顺序与证据：

| 假设 | 实验 | 结论 |
|---|---|---|
| ① SFT 的 assistant-only 掩码错位 | dump 训练样本，比较 `ids[:len(pids)] == pids` | ❌ 0/6 错位 → 掩码正确 |
| ② 语料里含 "None" | 扫 `reply_for()` 全部输出 | ❌ 0 条 → 不是记忆 |
| ③ 服务端模板不一致 | **同权重 HF(训练同款模板) vs llama-server** | HF 输出**正常**（"…可以用 Python 的 subprocess 模块…"），server 输出 `None` → **疑似模板** ⚠️ |
| ④ **量化档** | **同模板同端点，只换 Q4_K_M ↔ f16** | **Q4_K_M → `None`；f16 → 正常** ✅ **根因** |

**关键教训（`lyco` 信条 5 第 3 次命中）**：假设③ 的对照里 HF 跑 bf16、server 跑 Q4_K_M —— **变量没控住**，
差点把"量化"误判成"模板"。补一次**同对象跨配置**对照（Q4 vs f16，其余全同）才锁定根因。

## 三、根因结论

> **Q4_K_M 量化把 0.6B 在"分布外意图"上的「不确定时好好说话」行为压塌成了字面量 `"None"`。**
> 同权重 f16 完全正常 —— 不是模型问题、不是数据问题、不是模板问题。

| 用例 | Q4_K_M | f16 |
|---|---|---|
| 写脚本（分布外意图） | ❌ `None` | ✅ 正常解释 |
| paper 启动（分布内） | ✅ tool_call | ✅ tool_call |
| 你好呀 | ✅ 正常 | ✅ 正常 |

## 四、修复（两处，均已落地）

1. **`lycore` 退化输出守卫**（`agent.rs`，commit 见下）：
   `is_degenerate()` 判定 空 / `None` / `null` / `undefined` / `nan` → **绝不外泄**，
   走 lyco「诚实降级」语义：回答"抱歉，我没能理解这个请求，已加入学习队列。" + 入学习队列。
   - 2 条新单测（`degenerate_detection` / `degenerate_output_becomes_honest_degrade`）；`cargo test` **78 passed / 0 failed**。
   - **端到端复验**：任务 B 现在返回诚实降级文本（不再 `"None"`）。
2. **多档量化产物**（同一 f16 源，避免跨源偏差）：

| 档 | 体积 | BPW | 定位 |
|---|---|---|---|
| `qwen3_lyco_v3_q4km.gguf` | 484 MB | 5.09 | 体积优先（**已知分布外会崩，需配合守卫**） |
| `qwen3_lyco_v3_q6k.gguf` | 594 MB | 6.56 | **质量/体积平衡（推荐默认）** |
| `qwen3_lyco_v3_q8.gguf` | 768 MB | 8.50 | 质量优先（PC/有余量设备） |

均已下载到 `lyco_agent/models/`。

## 五、结论与下一步

- **"能用"的定义已被端到端验证过一次**（任务 A 真闭环），且暴露并修掉了产品级 bug（退化输出外泄）。
- **量化档是行为变量，不只是体积变量** —— 产品必须定档：建议 **q6_K 默认**，q4_K_M 仅在"必须省体积"时用且依赖守卫。
- **下一步（手机档）**：走 **QAT**（Unsloth `phone-deployment` + ExecuTorch `.pte`）—— 简报与 Unsloth 文档均指出
  **QAT 能恢复 ~70% 被量化损失掉的精度**，正是为避免上面这类 PTQ 崩塌。
- 待补：`llama-server` 的 system prompt 与训练不一致（server 用默认 system）→ 需统一；以及 argparse 参数质量（模型自造 `pack:"paper"`）。

## 六、源
- 本实验脚本：`D:/Code/cute_box/cs_e2e_serve.py` / `cs_e2e_ask.py` / `cs_hf_vs_llama.py` / `cs_f16_test.py` / `cs_multi_quant.py`
- 相关：`docs/exp-fc-recipe-holdout-2026-09-16.md`（v3 配方与 held-out）、`docs/requirement-and-needs-2026-09-16.md`（需求基线）
- Unsloth QAT/ExecuTorch：https://unsloth.ai/docs/basics/deploy-llms-phone
