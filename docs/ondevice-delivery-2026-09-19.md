# 端侧模型交付说明（2026-09-19）

## 1. 交付物

| 产物 | 角色 | 大小 | 参数量 | 关键指标 |
|---|---|---|---|---|
| `models/grpo-Q4_K_M.gguf` | 工具调用 / **驾驶模型** | 396.7 MB | 596.05 M（lm_head 绑定） | FC 遵循度 **80%**（基线 60%） |
| `models/router_v4-Q4_K_M.gguf` | **意图 → CLI 路由器** | 484.2 MB | 751.63 M（含 lm_head） | 拒绝率 **100%**、heldA **90.0%**、heldB **80.3%** |

链路：CloudStudio A10（NVIDIA A10 24G）→ 训练 → HF checkpoint → llama.cpp `convert_hf_to_gguf.py` → `llama-quantize Q4_K_M` → 拉回本机。

## 2. 实测吞吐（llama-bench，服务器 CPU / 8 线程 / Q4_K_M）

| 模型 | pp64 | tg32 |
|---|---|---|
| grpo | 569.2 t/s | 132.1 t/s |
| router_v4 | 575.3 t/s | 130.9 t/s |

说明 0.6B Q4_K_M 在**纯 CPU** 上完全可跑（手机 CPU 会低不少，但量级成立）。这是端侧部署的可行性下限证据。

## 3. 两个已定位的问题（都已给出成因，未静默绕过）

### 3.1 router_v4 转换失败：`tokenizer_config.json` 的 `extra_special_tokens` 是 list
`convert_hf_to_gguf.py` 走 `_set_vocab_gpt2 → AutoTokenizer.from_pretrained(dir)`，
在 `_set_model_specific_special_tokens` 里对 **list** 调 `.keys()` → `AttributeError`。
之后抛出的 protobuf `ImportError` 只是异常处理链里的**连带噪声**，会误导排查方向。
**修法**：把该字段规范化为 `{}`，转换即成功（`cloudstudio/a10_quantize_fix.py`）。

### 3.2 router_v4 比 grpo 大 ~83 MB：存了未绑定的 `lm_head`
- 张量数：grpo **310**（无 lm_head）vs router_v4 **311**（含 lm_head）
- `lm_head.weight` 与 `embed_tokens.weight` **不相等**（maxdiff ≈ 0.0087）→ 训练中确实解绑了
- 结论：**保留未绑定版本是忠实的**；不要为了省 83 MB 事后强行绑定（会改变输出）。
  要省体积应重做带 tie 约束的训练并重新验证。

## 4. 已知局限 / 未解决

- **命令准确率无实质突破**：三版路由器在弱线索困难集（heldB）都是 78–81%，**评测噪声 ±3pp**，
  单次差异不构成显著改进。0.6B + 合成改写数据的语义天花板约 80%。
- **BitNet 1.58-bit 不是事后转换**：它需要量化感知训练（从零训练或渐进微调），
  无法把已训好的 Qwen3 直接转成 1.58-bit 而不掉质量。若要极致低功耗需另立训练任务。
- 评测仍需加强：应加大评测集或多种子重复，抵消 ±3pp 噪声后再比较。

## 5. 复现

```bash
export CS_COOKIE='cloudstudio-session=<值>; cloudstudio-session-team=gh'
export CS_JPS=$(node cloudstudio/cs_auth.mjs <spaceKey> | head -1 | awk '{print $2}')

node cloudstudio/cs_exec_long.mjs  --file cloudstudio/a10_stage1.py        # clone + venv + torch
node cloudstudio/cs_exec_long.mjs  --file cloudstudio/a10_stage2.py        # 锁定版训练栈 + API 自检
node cloudstudio/cs_exec_train.mjs --file cloudstudio/a10_train_grpo.py    # 工具调用 GRPO
node cloudstudio/cs_exec_train.mjs --file cloudstudio/a10_cli_router_v4.py # CLI 路由器 v4
node cloudstudio/cs_exec_long.mjs  --file cloudstudio/a10_quantize.py      # GGUF Q4_K_M
node cloudstudio/cs_exec_long.mjs  --file cloudstudio/a10_quantize_fix.py  # 修 tokenizer 后补量化
```
