# 聊天小模型调研：A7A (VIP9000) × 安卓手机 NPU 选型
日期：2026-09-21 · 底座：CloudStudio L40 46GB

> **一句话结论**：底座选 **Qwen3-0.6B（Apache-2.0）**——
> 中文最强小模型、**手机 NPU 实测 115 tok/s**、且与现有 `router_v13`
> **同架构**（训练管线 / tokenizer / 模板 / 评测全部复用）。
> NPU 走 **ORT QNN EP（手机）+ galcore/TIM-VX（A7A）** 两条腿。

---

## 一、🔴 先修正两条写进 memory 的错误结论

这两条必须改，否则后续所有决策都会被带偏。

### 1.1 「Qwen 系 LLM 上 NPU 是范式级死路」❌ **只成立于 A7A**

实测反证（Qualcomm AI Hub 官方数据）：

| 平台 | 模型 | Prefill | **Decode** | License |
|---|---|---|---|---|
| **Snapdragon 8 Elite Mobile NPU** | Qwen3-0.6B | 7574 tok/s | **115 tok/s** | Apache-2.0 |
| **Snapdragon X2 Elite NPU** | Qwen3-0.6B | 8561 tok/s | **122 tok/s** | Apache-2.0 |

需要 QNN SDK ≥ 2.45.0，Context 4096。**decoder Transformer 在手机 NPU 上跑得好好的。**

⇒ 原结论应改写为：**Qwen 系 LLM 上「A7A 当前的 TIM-VX 软件栈」是死路**。

### 1.2 「NPU 只吃 CNN 类」❌ **硬件支持，软件栈不支持**

芯原官方资料（VIP9000）：

- 「**从传统 CNN 架构转向 Transformer 架构**」，做了三项优化：
  GEMM/GEMV 优化、**矩阵转置引擎**优化、流处理器优化
- 「完成了基于 **Multi-head Self Attention 的图优化**，减少转置操作并降低 **10% 带宽**」
- 支持 **FP8**（E4M3 / E5M2，指数位与英伟达一致）、INT8/INT16/FP16/BF16、**混合量化**
- **4-bit 量化 + 压缩**技术解决带宽问题
- 官方宣称可在嵌入式设备部署 **Stable Diffusion 与 Llama 2**
- ViT / BERT / DETR 上 **50%~70% 算力利用率**；单核 50 TOPS，多核至多 400 TOPS

⇒ **限制在软件栈（TIM-VX 缺 Attention/LN/RMSNorm/RoPE/KV-cache 算子），不在硅。**
petayyyy 已用自编译驱动在 A733/VIP9000 上跑通 **SmolLM2-135M 20.7 t/s**，佐证了这点。

> **方法论**：判定「某硬件能不能跑某类模型」时，必须区分
> **IP/硅的能力** 与 **当前软件栈的算子覆盖**。两者常常差一到两个版本。
> 尤其不能拿「某个开源驱动版本跑不通」去证伪硬件能力。

---

## 二、底座选型

### 2.1 候选对比

| 模型 | License | 中文 | 手机 NPU 验证 | 判 |
|---|---|---|---|---|
| **Qwen3-0.6B** | **Apache-2.0** | ⭐⭐⭐⭐ | ✅ **115 tok/s**（AI Hub 官方） | **✅ 选它** |
| Qwen3-1.7B | Apache-2.0 | ⭐⭐⭐⭐⭐ | ✅ ~50 tok/s（Q4） | 质量优先备选 |
| SmolLM2-135M/360M | Apache-2.0 | ⭐（**英文为主**） | ✅ A7A NPU 20.7 t/s | 中文不行 |
| MiniCPM 4 | Apache-2.0 | ⭐⭐⭐⭐ | ✅ | 国产端侧首选，生态较小 |
| Phi-4-mini 3.8B | MIT | ⭐⭐⭐ | ✅ ORT ONNX 官方发布 | **A7A 太大**（4.9GB） |
| Gemma 3 1B | Gemma 条款（非 OSI） | ⭐⭐⭐ | ✅ LiteRT | ⚠️ 许可证非标准，不用 |

### 2.2 为什么是 Qwen3-0.6B（四条理由，按重要性）

1. **中文**：SmolLM2 作者自述「primarily understands and generates **English**」。
   聊天场景中文是硬需求，这条一票否决 SmolLM2。
2. **NPU 有官方适配**：Qualcomm AI Hub 已上架 Qwen3-0.6B（含 ORT/QNN 链路），
   意味着手机 NPU 这条路**不用自己做算子搬运**。
3. **与现有资产同构**：`router_v13` 就是 Qwen3-0.6B。
   tokenizer / chat template / 评测脚本 / L40 训练管线 **全部现成复用**，
   训练完可直接发布到已有的 `lyco42/lyco-agent-qwen3-0.6b-ondevice`。
4. **License 干净**：Apache-2.0，可商用。（对比：Gemma 非 OSI 条款；ultralytics/YOLOv10
   实测是 **AGPL-3.0**，踩过坑）

### 2.3 尺寸取舍：0.6B vs 1.7B

| | Qwen3-0.6B | Qwen3-1.7B |
|---|---|---|
| A7A CPU（已实测 Zan + taskset 6,7） | **22.4 t/s** | ~8 t/s（估） |
| 手机 NPU（Snapdragon） | 115 tok/s | ~50 tok/s |
| 中文质量 | ⭐⭐⭐ | ⭐⭐⭐⭐ |
| L40 46G 全量微调显存 | ~7GB（极宽裕） | ~27GB（紧但可行） |

**建议 0.6B 起步**：两端都能跑、复用度最高、把「先跑起来」的风险降到最低。
若验证后嫌质量不够，1.7B 作为第二阶段（同一管线换个 base 即可）。

---

## 三、NPU 落地路径（两条腿）

### 3.1 安卓手机：ORT + QNN EP 是唯一成熟路

实测结论（多来源一致）：**只有 ONNX Runtime 能在手机上真用 NPU 跑 LLM。**

| 运行时 | 手机 NPU | 适用芯片 |
|---|---|---|
| **ONNX Runtime + QNN EP** | ✅ **Hexagon NPU** | **Snapdragon** |
| ONNX Runtime + NNAPI EP | ⚠️ 已 deprecated | 通用 Android |
| **LiteRT-LM（ex-TFLite）** | ✅ | 通用含 **MediaTek** |
| llama.cpp | ❌ 仅 CPU / GPU(Vulkan) | — |
| MLC-LLM | ❌ 仅 GPU(Metal/Vulkan) | — |

**新增发现：Google 的 `litert-community/Qwen3-0.6B` 已提供三种 artifact：**

```
Qwen3-0.6B.litertlm                         INT8 权重, float KV   4096  586 MB
Qwen3-0.6B.mediatek.mt6993.litertlm   a16w8 NPU-targeted   4096  992 MB  ← MediaTek NPU
qwen3_0_6b_mixed_int4.litertlm              TorchAO 混合 INT4     2048  475 MB
```

⇒ **MediaTek 也有路了**（不必只绑 Snapdragon，呼应「普惠」不变量）。

**开发注意事项（来自 Unity+Android 实战踩坑）**：
- **minSdk ≥ 31**：Android 12 起才能用 `uses-native-library` 声明 vendor 分区库；
  QNN HTP 依赖 src 在 vendor 分区的 `libcdsprpc.so`，低于 31 **NPU 完全用不了**。
- 运行时切换后端只是改一行 providers 列表，但 **QNN 要模型已按 QNN 量化**（走 AI Hub 转换）。

### 3.2 A7A (VIP9000)：软件栈是关键变量

| 方案 | 状态 |
|---|---|
| ORT 官方 Verisilicon EP | ❌ **不存在**（官方只有 CUDA/TensorRT/OpenVINO/CoreML/NNAPI/QNN） |
| **galcore + TIM-VX** | ✅ **可行**（`MaverickLong/Radxa-A733-NPU-Unified-Driver-Support-Package`，有 6.6 内核 API drift 补丁） |
| Verisilicon 官方栈 | 需 ACUITY Docker **约 11GB**（上次因体积搁置；TIM-VX 可跳过） |
| petayyyy/a733_npu_driver | ✅ 已跑通 SmolLM2-135M 20.7 t/s / MobileCLIP-S0 22.6ms / Zipformer ASR |

**务实建议：A7A 上先别赌 NPU 跑 LLM。**
已验证的舒适区是 **CPU + Qwen3-0.6B-Q4_K_M = 22.4 t/s**（`taskset -c 6,7 -t 2`）。
NPU 留给 **视觉/语音**（这正是 petayyyy 的「hybrid」结论，也与
`research-cross-device-vision` 的分工一致）——NPU 吃掉视觉后，8 核 CPU 全留给聊天。

### 3.3 与现有 ORT 选型的关系

`research-cross-device-vision-2026-09-21.md` 定的
「**主运行时 = ONNX Runtime（Rust 侧 `ort`），Tier-0 兜底 = tract**」
**依然成立且被本次调研强化**：手机 NPU 只有 ORT 有生产级路径。

需要补充一条：**ORT 没有 Verisilicon EP** ⇒ A7A 走
`backend.rs` 的 **Tier-1 专用后端**（galcore/TIM-VX），而不是指望 ORT EP 覆盖。
这恰好检验了当初「抽象 `Engine` trait」这个决定的价值——
如果当时直接绑死 ORT EP，现在 A7A 就是死路一条。

---

## 四、训练实录（2026-09-22）

### 4.1 铁律（来自 memory，勿违反）

- **优化器必须 `transformers.Adafactor`**，lr=2e-5
  （torch≥2.10 原生实现步长小 **200 倍**，用了等于没训）
- Qwen3 模板：训练/评测/推理**全部 `enable_thinking=False`**
- 🔴 **但 `enable_thinking` 并不控制 think 块的注入**（2026-09-22 实测推翻字面理解）
  ：transformers **5.1.0** 里，`enable_thinking=False` 与 `True` 渲染出的 assistant 段
  **完全相同**，都带空 block：

  ```
  <|im_start|>assistant\n<think>\n\n</think>\n\n我很好<|im_end|>\n
  ```

  即**训练数据会天然带上空的 think 前缀**。若不处理，模型学会「先吐 5 个废 token 再回答」，
  端侧（尤其 A7A CPU 22.4 t/s）每次对话白白多花 ~0.2s。
  ⇒ 管线里渲染后统一 `.replace("<think>\n\n</think>\n\n", "")`，
  让 assistant 段直接开答；推理侧同样不要期待 think 块。
  ⚠️ 这条要随 transformers 版本复核：不同版本的模板 jinja 行为不同。

### 4.2 🔴 显卡三处口径互相矛盾 —— 唯一可信源是远端 `nvidia-smi`

标题里的「L40 46GB」是**错的**。三个来源给的型号都不一样：

| 来源 | 声称 | 是否可信 |
|---|---|---|
| 本机 memory（09-21 记的「定稿」） | NVIDIA L40 46GB | ❌ |
| CloudStudio 控制台界面 | GPU A10 | ❌ |
| `/api/workspace` 返回 | 不含型号字段 | — |
| **远端 `nvidia-smi`** | **Tesla V100-SXM2-32GB (sm_70, Volta)** | ✅ 实测 |

⇒ **开训前必跑探针**（`_p2_probe_gpu.py`）。V100 32GB 做 0.6B 全量微调仍绰绰有余，
但它是 **Volta，没有 bf16 张量核**，直接改写了精度方案（见 4.3）。

### 4.3 精度方案：fp16 autocast + fp32 主权重

| 做法 | 结果 |
|---|---|
| 模型加载成 **bf16** | ❌ V100 无 bf16 张量核（matmul 走 FP32 模拟，能跑但慢） |
| 模型加载成 **fp16** + `fp16=True` | ❌ 梯度也是 fp16 → `ValueError: Attempting to unscale FP16 gradients` |
| **模型 fp32 + `fp16=True` autocast** | ✅ 唯一可行，compute 仍落 fp16 张量核 |

### 4.4 吞吐实测：为什么最终只跑 ~1.8 万条

V100 上 0.6B 全量微调的**显存主要被 CE 的 fp32 logits 吃掉**（`BS×SEQ×151936×4` 字节）：

| 配置（每条满 1024 token 的最坏情况） | s/step | tok/s | peak |
|---|---|---|---|
| bs=4 acc=2 无 liger | **2.85** | **2876** | 28.4GB |
| bs=4 acc=2 liger | 4.66 | 1758 | 22.9GB |
| bs≥8（含 liger） | OOM | — | — |

- ⛔ **liger-kernel 反而慢一倍**（本来指望 fused linear CE 省显存，实测既没省到也没提速）
- ⇒ 定 `bs=4 / acc=4`

> 注意基准是「每条满 1024 token」的最坏情况；**真实数据平均 ~520 token**，
> 实际吞吐 **5528 tok/s**（约 2 倍），整个 32 分钟训完。

### 4.5 最终读数（2026-09-22 跑完）

| 项 | 值 |
|---|---|
| 样本 | **17,896 条 / 9.30M tokens** |
| 耗时 | **32m19s**（1119 步，1.50 s/step，5528 tok/s） |
| train_loss | **1.2300** |
| HF 产出 | `/workspace/chat_slm_qwen3_0p6b`（fp32 权重 + HF config/tokenizer） |
| GGUF f16 | `/root/chat_slm_qwen3_0p6b-f16.gguf` — **1509.3 MB** |
| **GGUF Q4_K_M** | **`/root/chat_slm_qwen3_0p6b-Q4_K_M.gguf` — 484.2 MB**（`ftype: Q4_K - Medium`） |
| 可用性闸门 | `llama-cli --single-turn` **rc=0**，吐出中文回答（见 4.5.1） |

#### 4.5.1 GGUF 闸门实测

```console
$ llama-cli -m /root/chat_slm_qwen3_0p6b-Q4_K_M.gguf --single-turn ...
model      : /root/chat_slm_qwen3_0p6b-Q4_K_M.gguf
ftype      : Q4_K - Medium
> <|im_start|>user
你好，介绍一下你自己<|im_end|>
<|im_start|>assistant
你好，我是AI语言模型，我可以通过对话来回答你的问题。
[ Prompt: 28.3 t/s | Generation: 3.9 t/s ]
```

两点必须记牢：

1. **`llama-cli` 无参数会进交互模式导致 TIMEOUT**，`-no-cnv` 是无效 flag（rc=1），
   正确的是 **`--single-turn`**。
2. 上面的 `3.9 t/s` 是**远端 V100 容器 + CPU 受限环境**的读数，**不代表 A7A 性能**；
   A7A 实测仍待补（4.8 第 4 条）。

导出链路沿用 v13：`/root/llama.cpp/convert_hf_to_gguf.py` → `build/bin/llama-quantize Q4_K_M`，
并照旧做了 `tokenizer_config` 里 `extra_special_tokens` **list→dict 归一化**（v4/v13 同款坑）。

数据源实测分布（token 数占比更能反映真实算力去向）：

| 源 | 条数 | tokens | 占比 |
|---|---|---|---|
| moss-003（中文多轮） | 7000 | 5.76M | **62%**（平均 820 tok/条，最重） |
| wizard-zh（中文指令） | 5972 | 1.88M | 20% |
| tulu-3（英文通用） | 4924 | 1.66M | 18% |

### 4.6 🔴 loss 呈稳定双群 —— 不是噪声，是「两个数据群体」

每 25 步一个日志点，**奇偶严格交替**，一直到最后一步都不消失：

```
step  350  loss 1.435  grad_norm 17.2     ← 长样本群（moss 中文多轮）
step  375  loss 1.110  grad_norm  3.2     ← 短样本群（wizard / tulu）
step 1075  loss 0.995  grad_norm  3.2
step 1100  loss 1.341  grad_norm 18.7
```

- 短样本群 **1.27 → 0.99**（学得很扎实）
- 长样本群 **1.43 → 1.34**（几乎没学好），且 grad_norm 常年 12~21（短群只有 ~3.3），
  step 650 还出现过一次 **`grad_norm: Infinity`**
- 成因推测：`group_by_length` 把 ~1000 token 的 moss 样本聚成同批，
  长中文多轮的每 token 预测难度本就更高；加上 moss-003 是 2023 年数据，
  回答风格模板化，可压缩性低于 wizard/tulu。
- ⚠️ **这解释了为什么这个模型「聊天能聊，但共情和代码偏弱」**：
  算力有 **62% 花在了最难学、收益最低的那一半数据上。

### 4.7 质量实测（贪心 vs 采样，各 6 题）

| 题 | 表现 |
|---|---|
| 自我介绍 | ✅ 中文流畅（会虚构「语音/图像」能力 —— SFT 数据年纪大，典型症状） |
| Python 异步下载器 | ⚠️ 结构完整（代码块+解释+`<|im_end|>` 正常收尾），**但把同步 `requests` 塞进 `async with`** —— 0.6B 的合理上限 |
| 「今天心情不太好」 | ❌ 最弱：答「你好，我很好，我今天心情不错，你呢？」——**没接住用户情绪** |
| Transformer 自注意力 | ⚠️ 大意对，细节有错（「预测下一个词的词性」） |
| 英文俳句 | ✅ 通 |
| 1+1 | ✅ |

> ⚠️ **别拿短 max_new_tokens 下的输出判定「模型不会」**：训练脚本里 `max_new_tokens=128`
> 一度只吐出「以下是一个简单的异步下载器的 Python 实现：」就断，看着像只会说半句；
> 实际是**截断**，放开到 160 后完整代码+解释都出来了。

### 4.8 下一步（按性价比排序）

1. **换掉 moss-003**：它是 token 最贵（62% 算力）又学得最差（loss 几乎不降）的一半数据。
   换成更新的中文对话集（>COIG / ShareGPT-ZH-ish 质量），预计同样算力下整体 loss 明显下降。
2. **补 YAML 以外**：情感陪伴类数据（当前最弱项）。
3. 2~3 epoch（这次只 1 epoch）+ LR 稍降。
4. A7A 上真实跑一遍 GGUF Q4_K_M 测 tok/s（对比既有 Qwen3-0.6B-Q4_K_M 的 22.4 t/s 基线）。

### 4.9 本次交付状态

| 环节 | 状态 |
|---|---|
| HF 底座调研 + 选型 | ✅ 见 §二（Qwen3-0.6B / Apache-2.0） |
| 安卓 NPU 路径（ORT QNN EP） | ✅ 见 §3.1（Snapdragon 8 Elite 官方 115 tok/s） |
| A7A NPU 路径判定 | ✅ 见 §3.2（**先别赌 NPU 跑 LLM**，CPU 22.4 t/s 已验证，NPU 留给视觉/语音） |
| GPU 训练 | ✅ 32m19s / 17,896 条 / loss 1.2300 |
| GGUF 导出 + 闸门 | ✅ f16 1509.3MB + Q4_K_M 484.2MB，`llama-cli --single-turn` rc=0 |
| HF 上传 Q4_K_M | ⏳ 未做（本机 `HF_TOKEN` 是 role read，需 `HF_WRITE_TOKEN`） |
| A7A / 手机真机实测 | ⏳ 未做（模型在远端 `/root/`，待下载） |

拿到 GGUF 后在 A7A 上的验证命令（沿用既有 CPU 最优配置）：

```bash
# 远端：/root/chat_slm_qwen3_0p6b-Q4_K_M.gguf  → 板子 ~/models/
taskset -c 6,7 ./llama-cli -m ~/models/chat_slm_qwen3_0p6b-Q4_K_M.gguf \
  -t 2 --single-turn -p '你好，介绍一下你自己'
# 对照基线：原版 Qwen3-0.6B-Q4_K_M 同配置 = 22.4 t/s
```

---

## 五、源

- SmolLM2 卡：https://huggingface.co/HuggingFaceTB/SmolLM2-135M
  （Apache-2.0；自述 *primarily understands and generates **English***）
- Qualcomm AI Hub Qwen3-0.6B：https://aihub.qualcomm.com/models/qwen3_0_6b
  （QNN SDK ≥2.45.0；SD 8 Elite NPU decode 115 tok/s；Apache-2.0）
- LiteRT-LM Qwen3-0.6B：https://huggingface.co/litert-community/Qwen3-0.6B
  （含 **MediaTek mt6993** NPU 定向 artifact）
- 芯原 VIP9000 Transformer/MHA/FP8 能力：ICDIA 2023 演讲 + BusinessWire 2024 稿
- 手机 runtime 对比：https://iotdigitaltwinplm.com/on-device-llm-runtimes-llama-cpp-vs-mlc-vs-onnx-2026/
- A7A 已有实测：见 `MEMORY.md`「A7A LLM 结论」「NPU 落地资源」
