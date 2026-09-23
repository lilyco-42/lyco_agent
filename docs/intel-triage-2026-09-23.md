# 情报分诊：2026-09-19 ~ 09-23 AI 简报 → LYCO 动作清单

> 输入：用户提供的 5 天科技简报。本文只做一件事：**判断哪些条目会改变我们的动作**，
> 并标注**我已核实到什么程度**（简报里的厂商数字一律不进结论）。

## 0. 结论速览（四条改动作）

| # | 情报 | 对 LYCO 的动作 | 状态 |
|---|---|---|---|
| A | **Cactus Needle 3**（8–29MB tool-calling 模型） | 加入 A7A 基准名单；用 5/20/50/100 tools 测路由 | **已核实可用**（见 §A） |
| B | **Closed-World Resolution Against Tool Hallucination** | 在 lycore 落地确定性 resolver：registry→schema→capability→execute | 待实现（成本低） |
| C | **数据库 Agent 论文**：75.7% 的模型归因失败发生在**成功调用工具之后** | 工程重心从「换更大模型」移到 runtime/transport/observability | 方向确认 |
| D | **When Better Turns…**：单步 SFT 变好，完整 workflow 成功率 ≤10.4% | 评测从「单题对错」升级为四层 + `Tokens per Successful Step` | 改评测设计 |

## A. Needle 3 —— 已核实，且正好卡在我们架构的空位上

**核实结果**（HF API 实查，非转述）：

| 项 | 值 |
|---|---|
| 仓库 | `Cactus-Compute/needle3`（Apache-2.0，30 天 6.2 万下载；另有 needle2 2.99 万） |
| 参数 | **121,021,910**（hidden 768 / 20 层 / GQA / vocab 8192 / 滑窗 1024 / max_pos 8192） |
| 权重可变形 | 2 层 → 20 层按设备/任务选深度（简报口径，未独立验证） |
| 官方二进制 | **linux-arm64 / android-arm64 / android-riscv64 / ios / macos / windows-arm64 / wasm / wasm-component** |
| 运行时 | **自定义引擎**（`NeedleForToolCalling`, engine 3.0.0），**不是 GGUF/llama.cpp** |
| 生态 | `needle-rs`（600KB WASM 运行时，⭐95）、`.NET` 封装、`n8n` 节点、Android 端侧 app、OpenAI 兼容 HTTP 层 |

**为什么它是我们的「Tier 0」候选**：
- 121M 参数在低比特下 ≈ 24–30MB —— 与简报「8–29MB」相符（数量级合理）；
- 它专门做 **tool routing / structured extraction / semantic matching**，不做闲聊 —— 正是我们要它做的那件事；
- 🔴 **「没有匹配的 tool 时输出空列表，而不是猜一个」** —— 这直接对着我们最烂的一个指标：
  路由域实测 **拒绝率 0%**（base 与所有 SFT 臂都是 0）。Needle 把这能力做进模型层。

**行动（按序）**：
1. 先取 `linux-arm64` 二进制 + 权重，在 A7A 上跑通「能否运行 + RAM + 单次延迟」；
2. 用我们现成的 CLI 池（docker/git/cargo/npm/kubectl/jq + L1 的 14 个）做 **5/20/50/100 tools** 扩展实验，
   只测六项：tool accuracy / argument accuracy / **hallucinated tool rate** / TTFT / RSS / tokens-per-success；
3. 与我们 0.6B 路由器（v13）同池对照 —— 如果 121M 能覆盖 60–80% 的简单路由，
   架构就从「0.6B 处理每个请求」改成 **Needle(Tier0) → 0.6B/1.7B(Tier1 Planner) → 云端**。

## B. 工具幻觉：runtime 才是边界（论文要点）

论文实测：10 个托管模型 × 2 种调用接口 → **322 个真实 tool hallucination**；
**675B 模型也不会因规模而消失**；MCP 多 server 合并后还有 namespace collision / shadowing。

**结论与我们的既有判断完全同向**：我们之前定「护城河 = 注入协议 + 能力表 + T1 门，不在权重」
（路由域 Δ_SFT = −43.3pp，base 零样本 85.0% 反而最高）。这篇论文给了第三方证据。

**行动**：在 lycore 把解析顺序固化成确定性闸门（模型只提供候选，不决定是否存在）：

```
LLM ──► Tool Resolver ──┬─ registry.contains(name)?
                        ├─ schema.validate(args)?      ← 我们已有 lfiles --schema 的 required/safety
                        └─ arg type check
                        └─ capability.check()           ← 我们已有能力表
                        ──► Execution
```
验收判据：**不存在的 tool 永远执行不了**（当前靠 prompt 约束，属于概率保证）。

## C. 数据库 Agent 论文 —— 最反直觉、也最省钱的一条

39 个本地模型 / 11 天 / **8,199 次真实运行** / 14,008 次被拒 tool call：
在 2,100 个「归因于模型」的失败里，**1,590 个（75.7%）发生在模型已经成功调用至少一个工具之后**
（最大类别是 transport）；纯「不会调工具」只占小部分。
**只修 5 个 server/runtime 问题、完全不动模型/prompt/采样，就让 6 个模型在 30 个测试单元中改善 6–21 个。**

另一发现（对 A7A 直接相关）：一个 7.1GB 的模型因为开了 262,144 context，
**在 64GB 主机上吃掉了 51GB RAM** —— 表面日志只写「timeout」。

**行动**：
1. 建 **Failure Ledger**，把失败强制分桶：`MODEL / ROUTING / SCHEMA / TRANSPORT / EXECUTION / VERIFY / RECOVERY / RESOURCE`；
2. 记录资源三元组 **权重 + KV cache + 实际 RSS**（以后我们的端侧报告不许只写「模型只有 1GB」）；
3. 短期先修 runtime 侧：单步成功率之后必然遇到的 transport/超时/状态残留。

## D. 评测方法必须换（两篇论文的共同含义）

- 单步「给 gold state → 预测下一步」变好，**不代表**自主多轮 workflow 变好（严格完成率 ≤10.4%）；
- 语音 Agent 的 MTVA-Bench：7 个模型里 6 个「选对工具」只差 6.4pp，但总分差 24.4pp —— 差在
  **参数值、动作顺序、规则遵循、工具调用前后的自然语言行为**。

**行动**：我们的评测分四层，并新增两个端侧专属指标：

| 层 | 测什么 |
|---|---|
| L1 Tool Selection | 选没选对（我们已有：acc_exec） |
| L2 Tool Execution | 参数/执行是否成功 |
| L3 Recovery | 错了能不能自己恢复 |
| L4 Workflow Success | 最终任务真的完成了吗（**唯一真正的成绩**） |

新增：`Tokens per Successful Step`（成功一次要烧多少 token —— 端侧比准确率更值钱）、
`RAM / Successful Turn`。

## E. 其它有价值的、但优先级较低

| 情报 | 我们的处置 |
|---|---|
| **SemKV**（KV 极低比特，~2.322 bit 是质量悬崖，混合精度 6–7.9× 压缩） | 端侧 Agent 长上下文时 **KV 比权重更吃内存** → benchmark 必须记 RSS+KV；暂不动手 |
| **蒸馏 EOS 不一致导致长度膨胀**（Qwen3/Llama/Gemma 全家族可见） | 与我们实测「写诗重复退化」「base 思考吃光预算」同源 → 下轮训练加 length 监控 + 统一 stopping set |
| **Hailo-10H**：6.9 tok/s @ <2W，20 轮 sustained 几乎无衰减；iPhone 第二轮掉一半、三星触发频率地板 | 端侧「峰值 tok/s 严重误导」→ A7A 该做 **20 轮 sustained + tokens/joule**（⚠️ 但与我们「板子不做重负载」的约定冲突，**需你点头**） |
| **Rust：Miri + GitHub Actions cache 泄露 CI secrets** | **已自查：不适用**（我们只缓存 cargo registry，无 miri、workflow 里无 secrets）。记住通用规则：**有 secrets 的 job 不要缓存 `target/`** |
| OpenObserve v1.0 把 Agent tracing 并入 observability | 印证 Failure Ledger 方向；`Trace → Verified Experience → Training Asset` 与我们 lbrush 记录器同构 |
| Snorkel 估值 $3.5B（收入 2000 万 → 3.5 亿，卖「经过验证的经验数据」） | 我们的 `lbrush`（NL→cmd→exit_code）正好是这类资产的雏形，值得继续埋点 |

## F. 明确不采纳（厂商自述、缺独立验证）

- **Ternary Bonsai 2 27B**「保留 98.2% 性能」——项目方口径，等 llama.cpp/BitNet.cpp 真机 benchmark 与 Q4 同机对照；
- **ASUS Ascent QN10 80 TOPS** / **Intel Core Ultra 3「180 platform TOPS」** / **MediaTek 9600 Pro NPU +51%** ——
  全部等第三方 `tokens/s / TTFT / RSS / sustained power`；TOPS ≠ LLM 可用吞吐；
- **27B ternary ≈ 5.9GB** 对 A7A 无意义（权重放得下 ≠ 带宽够）。

## G. 与我们当前进度的接口

v2（COIG-CQIA 换掉 moss-003）训练刚结束：`train_loss 2.23`（**跨数据集不可比**，不能据此判定变差），
554 步 × batch2 × acc16 = 17,728 条 = 1 epoch 全覆盖。24 题对照正在跑。

**如果 v2 仍然打不过未微调的原版底座**（很可能，因为底座没换），那么依据本文 A/B/C/D 四条，
下一步**不是换更大的模型**，而是：
1. Tier-0 控制器（Needle 3）替掉大部分路由调用；
2. runtime 确定性 resolver + Failure Ledger；
3. 评测换四层 + 两个端侧指标；
4. 只有在 1–3 做完后仍不够，才考虑换底座 —— 而那时的瓶颈也会被数据说清楚（是 MODEL 桶还是 TRANSPORT 桶）。
