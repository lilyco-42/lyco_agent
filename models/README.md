# 端侧模型产物（Qwen3-0.6B 派生，GGUF Q4_K_M）

由 `cloudstudio/` 下的管线在 CloudStudio A10 上训练 + 量化得到。**不要把这些 .gguf 提交进 git**（已在 .gitignore 内）。

| 文件 | 角色 | 大小 | 参数量 | 来源 |
|---|---|---|---|---|
| `grpo-Q4_K_M.gguf` | **工具调用 / 驾驶模型** | 396.7 MB | 596.05 M（lm_head 绑定） | `tools/qwen_grpo_train.py`，200 步 GRPO，FC 遵循度 **80%**（基线 60%） |
| `router_v4-Q4_K_M.gguf` | **意图 → CLI 路由器** | 484.2 MB | 751.63 M（含 lm_head） | `cloudstudio/a10_cli_router_v4.py`，拒绝率 **100%**，heldA **90%** / heldB **80.3%** |

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

```bash
llama-cli    -m router_v4-Q4_K_M.gguf --jinja -sys "<上面的 router system prompt>" -p "现在多少主频"
llama-server -m router_v4-Q4_K_M.gguf --jinja -c 2048 --port 8080   # OpenAI 兼容接口
```

## 实测吞吐（llama-bench，**服务器 CPU** / 8 线程 / Q4_K_M）

| 模型 | pp64（预填充） | tg32（解码） |
|---|---|---|
| grpo | 569.2 t/s | 132.1 t/s |
| router_v4 | 575.3 t/s | 130.9 t/s |

手机 CPU 会明显低于此，但量级说明 0.6B Q4_K_M 在**纯 CPU** 上可跑。若要进一步压功耗，
见 `docs/cli-router-experiments-2026-09-19.md` 第 5 节的路线讨论（BitNet 1.58-bit 需重训，
不是事后转换）。
