# 端侧模型产物（Qwen3-0.6B 派生，GGUF Q4_K_M）

由 `cloudstudio/` 下的管线在 CloudStudio A10 上训练 + 量化得到。**不要把这些 .gguf 提交进 git**（已在 .gitignore 内）。

| 文件 | 角色 | 大小 | 参数量 | 来源 |
|---|---|---|---|---|
| `grpo-Q4_K_M.gguf` | **工具调用 / 驾驶模型** | 396.7 MB | 596.05 M（lm_head 绑定） | `tools/qwen_grpo_train.py`，200 步 GRPO，FC 遵循度 **80%**（基线 60%） |
| `router_v4-Q4_K_M.gguf` | 意图 → CLI 路由器（已被合并版取代） | 484.2 MB | 751.63 M（含 lm_head） | `cloudstudio/a10_cli_router_v4.py`，拒绝率 **100%**，heldA **90%** / heldB **80.3%** |
| **`router_merged-Q4_K_M.gguf`** | **意图 → CLI 路由器（推荐用这个）** | 484.2 MB | 751.63 M（含 lm_head） | **v3 ⊕ v4 权重平均（模型合并）**，heldA **98.5%** / heldB **79.7%** / 拒绝 **100%** |

> **为什么推荐 `router_merged`**：把 v3 与 v4 两条优化路径的 checkpoint 逐张量平均（各 0.5），
> **零训练成本**却同时超过两者（heldA 91.7%→98.5%，heldB 74.8%→79.7%），两次独立评测命中数完全一致。
> 依据 DeepSeek-V4.1-Flash 报告 §5.1.2「model merging reinitializes successive RL runs」。
> 详见 `docs/cli-router-experiments-2026-09-19.md` 第 7 节。
> 实测吞吐：pp64 556.7 t/s、tg32 128.2 t/s（CPU/8 线程）。

> ⚠️ `router_v4` 比 `grpo` 大约 83 MB，原因是它**存了未绑定的 `lm_head`**（训练中 `lm_head.weight` 与
> `embed_tokens.weight` 已解绑，maxdiff ≈ 0.0087）。这是**忠实产物**，不要事后强行绑定（会改变模型输出）。
> 若确实要省这 83 MB，需要重新做带 tie 约束的训练，并重新验证质量。

## system prompt（推理时必须一致，否则行为会漂）

- **grpo（工具调用）**
  ```
  You are lyco, a helpful assistant. You can call tools.
  ```
  并按 `tools/qwen_grpo_train.py` 里的 `TOOLS` 传 function schema；模型输出
  `<tool_call>{"name": "...", "arguments": {...}}</tool_call>`。

- **router_v4（CLI 路由）**
  ```
  你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。
  ```
  输出形如 `hw cpu` / `hw gpio set 0 370 1`；与硬件无关时输出 `(无需调用硬件命令)`。

## 跑法（llama.cpp）

⚠️ **两个必须注意的点（实测踩过）**：
1. **必须关掉 thinking**。训练用的是 `enable_thinking=False`；若推理时开着思考，模型会先长篇"思考"
   （`好的，用户问的是…`）而在有限 token 内不输出命令 → 看起来"模型不会用"。
2. **新版 llama.cpp（本机 0.4.1-dev, commit 60081bb）已移除 `-no-cnv`**（用会直接
   `error: invalid argument`）。要单轮退出用 **`-st` / `--single-turn`**。

```bash
# 路由器（无需 tools，最简写法）
llama-cli -m router_v4-Q4_K_M.gguf \
  -sys "你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。" \
  -p "现在多少主频" -n 48 --temp 0 -st \
  --chat-template-kwargs '{"enable_thinking": false}'

# 路由器（服务化，OpenAI 兼容）
llama-server -m router_v4-Q4_K_M.gguf -c 2048 --port 8080 \
  --chat-template-kwargs '{"enable_thinking": false}'

# grpo 工具调用：tools schema 无法从命令行传，需先用 transformers 预渲染
#   tok.apply_chat_template(msgs, tools=TOOLS, add_generation_prompt=True, enable_thinking=False)
# 再：
llama-cli -m grpo-Q4_K_M.gguf -f prompt.txt -n 96 --temp 0 -st
```

## 已验证（llama.cpp 端到端，服务器 CPU）

- **router_v4：9/9**（两轮共 9 个用例全中）—— 含弱线索困难样本与"该不调"拒绝：
  `现在多少主频→hw cpu`、`板子烫不烫→hw temp`、`读一下 gpio0 的 97 号脚→hw gpio get 0 97`、
  `gpio line 12 什么电平→hw gpio get 0 12`、`把风扇调到 200→hw fan 200`、`板子什么型号→hw info`、
  `讲个笑话→(无需调用硬件命令)`
- **grpo：工具调用可用**（`怎么新建 rust 项目` 产出 `lyv_knowledge` 调用；`你好呀` 不调工具）。
  权威质量指标是训练期评测：FC 遵循度 **80%**（基线 60%）。
- 吞吐：Prompt ~670–700 t/s，Generation ~121 t/s（CPU，8 线程）。

## 实测吞吐（llama-bench，**服务器 CPU** / 8 线程 / Q4_K_M）

| 模型 | pp64（预填充） | tg32（解码） |
|---|---|---|
| grpo | 569.2 t/s | 132.1 t/s |
| router_v4 | 575.3 t/s | 130.9 t/s |

手机 CPU 会明显低于此，但量级说明 0.6B Q4_K_M 在**纯 CPU** 上可跑。若要进一步压功耗，
见 `docs/cli-router-experiments-2026-09-19.md` 第 5 节的路线讨论（BitNet 1.58-bit 需重训，
不是事后转换）。
