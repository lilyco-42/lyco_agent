import os, sys, io

REPO_ID = "lyco42/lyco-agent-qwen3-0.6b-ondevice"
MODELS_DIR = r"D:\Code\lyco_agent\models"

CARD = """---
license: apache-2.0
base_model: Qwen/Qwen3-0.6B
language:
- zh
- en
library_name: llama.cpp
tags:
- gguf
- qwen3
- tool-calling
- agent
- on-device
- grpo
- model-merging
---

# lyco-agent Qwen3-0.6B (端侧 / on-device)

给 [lyco_agent](https://github.com/lilyco-42/lyco_agent) 用的**端侧小模型**，基于 `Qwen/Qwen3-0.6B` 微调，
已量化为 **GGUF Q4_K_M**，可在手机 / 单板上用纯 CPU 跑（llama.cpp）。

## 文件

| 文件 | 角色 | 大小 | 指标 |
|---|---|---|---|
| `router_merged-Q4_K_M.gguf` | **意图 → CLI 路由器**（推荐） | 484 MB | heldA(未见说法) **98.5%**、heldB(弱线索) **79.7%**、拒绝率 **100%** |
| `grpo-Q4_K_M.gguf` | **工具调用 / 驾驶模型** | 397 MB | FC 遵循度 **80%**（基线 60%） |

### router（把自然语言翻译成一条板端命令）

system prompt：

```
你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。
```

覆盖 13 类能力：`hw led <color> on|off|status|blink N`、`hw temp`、`hw cpu`、`hw mem`、`hw disk`、
`hw fan [V]`、`hw gpio get <chip> <line>`、`hw gpio set <chip> <line> <0|1>`、`hw info`；
与硬件无关时输出 `(无需调用硬件命令)`。

> `router_merged` 是 **v3 与 v4 两个微调 checkpoint 的权重平均（模型合并）**：
> 零额外训练成本，却同时超过两者（heldA 91.7%→98.5%，heldB 74.8%→79.7%），两次独立评测命中数完全一致。
> 做法参考 DeepSeek-V4.1-Flash 技术报告 §5.1.2「model merging reinitializes successive RL runs」。

### grpo（工具调用）

system prompt：`You are lyco, a helpful assistant. You can call tools.`
配合 function schema 使用，输出 `<tool_call>{"name": "...", "arguments": {...}}</tool_call>`。

## 用法（llama.cpp）

⚠️ **必须关掉 thinking**：训练用的是 `enable_thinking=False`；若推理时开着思考，模型会先长篇推理而**不输出命令**。

```bash
# 路由器
llama-cli -m router_merged-Q4_K_M.gguf \\
  -sys "你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。" \\
  -p "现在多少主频" -n 48 --temp 0 -st \\
  --chat-template-kwargs '{"enable_thinking": false}'

# 服务化（OpenAI 兼容）
llama-server -m router_merged-Q4_K_M.gguf -c 2048 --port 8080 \\
  --chat-template-kwargs '{"enable_thinking": false}'
```

> 新版 llama.cpp（0.4.1-dev）**已移除 `-no-cnv`**，单轮退出用 **`-st`**。
> grpo 模型的 tools schema 需要先用 `transformers` 的 `apply_chat_template(..., tools=..., enable_thinking=False)` 预渲染。

## 实测吞吐（llama-bench，**服务器 CPU** / 8 线程 / Q4_K_M）

| 模型 | pp64 | tg32 |
|---|---|---|
| router_merged | 556.7 t/s | 128.2 t/s |
| grpo | 569.2 t/s | 132.1 t/s |

手机 CPU 会明显更低，但量级说明 0.6B Q4_K_M 在纯 CPU 上可跑。

## 训练与数据

- 训练管线与实验记录：仓库 `cloudstudio/`、`docs/cli-router-experiments-2026-09-19.md`、`docs/ondevice-delivery-2026-09-19.md`
- router 用 SFT（意图→命令）+ 可编程 verifier 的 GRPO；训练数据由 `hw` CLI 语料扩写与失败回放构成
- **已知局限**：弱线索困难集（heldB）约 80% 是 0.6B + 合成改写数据的语义天花板；
  评测噪声约 ±3pp，单次差异不构成显著改进（见 docs 中的方法学说明）

## 许可

继承基座 `Qwen/Qwen3-0.6B`，同样为 **Apache-2.0**。
"""


def main():
    # 优先用 HF_WRITE_TOKEN（当前 HF_TOKEN 是只读 role=read，建仓会 403）
    wt = os.environ.get("HF_WRITE_TOKEN")
    if wt:
        os.environ["HF_TOKEN"] = wt
        print("using HF_WRITE_TOKEN", flush=True)
    from huggingface_hub import HfApi, create_repo
    api = HfApi()
    who = api.whoami()
    role = (who.get("auth", {}).get("accessToken", {}) or {}).get("role")
    print("auth as:", who.get("name"), "| token role:", role, flush=True)
    if role not in ("write", "fineGrained"):
        print("[!] 当前 token 非写权限(role=%s)，建仓/上传会被 403 拒绝。" % role, flush=True)
        print("    请提供写权限 token，或直接在网页 hf.co/new 建仓后手动拖拽上传。", flush=True)
        print("    （仍会尝试一次，便于确认）", flush=True)

    try:
        create_repo(REPO_ID, repo_type="model", exist_ok=True)
        print("repo ready:", REPO_ID, flush=True)
    except Exception as e:
        print("CREATE_REPO_FAILED:", str(e)[:300], flush=True)
        print("HF_UPLOAD_BLOCKED", flush=True)
        return

    # 模型卡
    card_path = os.path.join(MODELS_DIR, "HF_README.md")
    io.open(card_path, "w", encoding="utf-8").write(CARD)
    api.upload_file(path_or_fileobj=card_path, path_in_repo="README.md",
                    repo_id=REPO_ID, repo_type="model")
    print("uploaded README.md", flush=True)

    for fn in ("router_merged-Q4_K_M.gguf", "grpo-Q4_K_M.gguf"):
        p = os.path.join(MODELS_DIR, fn)
        if not os.path.exists(p):
            print("skip (missing):", fn, flush=True)
            continue
        sz = os.path.getsize(p) / 1e6
        print(f"uploading {fn} ({sz:.1f} MB) ...", flush=True)
        api.upload_file(path_or_fileobj=p, path_in_repo=fn, repo_id=REPO_ID, repo_type="model")
        print(f"  done: {fn}", flush=True)

    print("HF_UPLOAD_DONE", flush=True)
    print("https://huggingface.co/" + REPO_ID, flush=True)


main()
