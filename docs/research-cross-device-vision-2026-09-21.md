# 跨设备视觉推理选型调研（2026-09-21）

> 起因：用户纠偏「不能只局限 radxa a7a，要普惠全人类的设备」。
> 本轮只做**研究与选型**，不写能力代码；结论要能在 `backend.rs` 上直接落地。
> 方法：官方一手文档优先（onnxruntime.ai / ort.pyke.io / VeriSilicon/TIM-VX / Ultralytics docs），
> 许可证一律用 `gh api repos/<repo>/license` 实测 SPDX，不看二手博客的口径。

---

## 0. 决策摘要

| 议题 | 结论 | 一句话理由 |
| --- | --- | --- |
| 主运行时 | **ONNX Runtime**，Rust 侧用 `ort` | EP 抽象与我们的 `backend.rs` **语义逐条同名**：注册失败静默回落 CPU、`is_available()` 可探测、按序优先级 |
| 零依赖兜底 | **纯 Rust `tract`**（经 `ort-tract` 换后端，同一套 API） | 「一定能跑」不能指望用户机器上有个 `libonnxruntime.so` |
| 检测器默认下载/内置 | **YOLOX / PP-PicoDet**（Apache-2.0） | Ultralytics 全家和 YOLOv10 实测**全是 AGPL-3.0**（详见 §4.2） |
| NPU 能吃的东西 | **只有 CNN 类**（Conv/Pool/Eltwise/Resize/BN/Reduce） | TIM-VX 官方算子表没有 Attention/LayerNorm/RMSNorm/RoPE/KV-cache |
| LLM 路由器（v13） | **不上 NPU**，走 CPU/GPU | 同上；不要为了「用上 NPU」而把架构搞歪 |
| 浏览器端 | `ort-web`（WebGL/WebGPU） | 普惠的最后一公里：不需要装任何东西的设备也能跑 |

**一句话**：不要把 lyco 变成「某个 NPU 的适配层」，而是变成「一层统一的会话 API」——
哪家硬件厂商想让我们加速，**麻烦他们去给 ONNX Runtime 交 EP**（RKNN / QNN / CANN 就是这么进来的）。

---

## 1. 运行时矩阵（横向评测 + 覆盖度）

实测数据（搜到的一手 bench，非我方跑）：

- 树莓派上 YOLO11n-seg 导出格式对比（LearnOpenCV）：PyTorch 360ms → ONNX 157ms → **OpenVINO 81ms** → MNN 116ms → NCNN 292ms → TFLite 355ms。**同一张图同一个模型，导出格式能差 4 倍**，说明选型比模型调参更值钱。
- 骁龙 888 上 YOLOv5s（百度智能云整理）：NCNN-CPU 85ms / **NCNN-Vulkan 22ms** / MNN-CPU 78ms / MNN-GPU 18ms / **MNN-NPU 8ms**。→ 加速器一旦命中就是数量级差距，命中不了则退化到同一档次。

候选对比：

| 运行时 | 语言/依赖 | 设备覆盖 | 谁在适配新硬件 | 判定 |
| --- | --- | --- | --- | --- |
| **ONNX Runtime (EP)** | C++ dylib | CPU / CUDA / TensorRT / DirectML / WebGPU / OpenVINO / CoreML / NNAPI / QNN / RKNN(preview) / CANN(preview) / XNNPACK / VitisAI / MIGraphX | **硬件厂商自己**（Neuron/QNN/RKNPU 均为厂商贡献） | ✅ 主选 |
| LiteRT（原 TFLite） | C++ | Android NNAPI / iOS CoreML / XNNPACK；桌面覆盖弱 | Google | ❌ 移动端很强，但桌面/边缘 Linux 上她的两条主打路径（NNAPI/CoreML）都吃不到 |
| ncnn | C++，无第三方依赖 | ARM CPU 极强 + Vulkan GPU；NPU 各家要单独另适配 | 腾讯 + 社区 | ⚠️ 备选（Vulkan 路径对「无 NPU 的 Linux 盒子」很香） |
| MNN | C++ | CPU/OpenCL/Vulkan/Metal + 自家 NPU 后端 | 阿里 | ⚠️ 备选 |
| ExecuTorch | C++ | Mobile CPU/GPU/NPU，主打 PyTorch 原生链路 | Meta | ❌ LLM 侧再看 |
| Paddle Lite | C++ | **TIM-VX 已验证的 VeriSilicon NPU（含 VIP9000 同族）** | 百度 | ✅ **A7A 路线的实证来源**（见 §4.1） |
| **tract（纯 Rust）** | Pure Rust | CPU + WASM | sonos | ✅ Tier-0 兜底 |

**为什么不选「最快的那个」而选 ORT**：ncnn/MNN 在单设备上更快，但每接入一款新 NPU 都是**我们的工作量**；
ORT 是 EP 模型，接新 NPU 是**厂商的工作量**（GetCapability 图切分 + 他们自己的编译栈）。
普惠 = 把「适配 N 种设备」的成本外包给出货方。

---

## 2. 为什么ORT 的语义和 `backend.rs` 是同一套

`ort` 文档原话（ort.pyke.io/perf/execution-providers）：

> If an EP does not support a certain operator in a graph, it will fall back to the next successfully registered EP,
> or to the CPU if all else fails.
> ort will **silently fail and fall back** to executing on the CPU if all execution providers fail to register.

对照我们昨天立的：

| `backend.rs` | ORT / ort | 备注 |
| --- | --- | --- |
| `probe()` 只读不 panic | `ExecutionProvider::is_available()` | 都是「编译进没有」的静态探测 |
| `pick(prefer)` 偏好不可用静默降级 | `.build()` 注册失败静默回落 | **逐字同义** |
| `RunMeta.degraded` | `.error_on_failure()` / 手动 `register()` 拿 Result | 默认宽容，需要时也能严苛 |
| `available()` 恒非空（CPU） | CPUExecutionProvider 恒在场 | ✅ |

→ 结论：**不用为了 ORT 改 `backend.rs` 的设计，只需加一层「backend id ↔ EP 名」的映射**（§3）。

### Rust 侧的坑（提前记账）

- `ort` 目前只有 **`2.0.0-rc.13`（2026-07-28 更新，crates.io `max_stable_version` 为空）**——没有稳定版。
  对策：①依赖抽象成 `trait Engine`，ORT 只是实现之一；②版本号 pin 到精确 `=2.0.0-rc.13`。
- **shared library hell**：某些 Windows 自带旧版 `onnxruntime.dll`，运行时直接 assertion panic。
  对策：`load-dynamic` + `ORT_DYLIB_PATH` 显式指定我们自带的库，别赌 System32。
- `download-binaries` 只对 CUDA/TensorRT 提供预编译；其余 EP 要么 `system` 策略链接，要么自己编译 ORT（很慢）。

---

## 3. `backend.rs` catalog ↔ ORT EP 映射

现 catalog 11 项，逐项核用处，标 ⚠️ 的是本轮建议**新增**：

| catalog id | ORT EP 名 | ort feature | 上游可用 | 备注 |
| --- | --- | --- | --- | --- |
| `cpu` | CPUExecutionProvider | — | ✅ | 恒在场；建议再加 `tract` 作为零依赖 CPU 实现 |
| `npu-nnapi` | NNAPI | `nnapi` | ✅ | Android 全家（高通/MTK/三星/SSC） |
| `npu-coreml` | CoreML | `coreml` | ✅ preview | Apple ANE + GPU + CPU 混合委派 |
| `npu-openvino` | OpenVINO | `openvino` | ✅ | Intel CPU/GPU/**NPU**；上面 bench 里赢麻的那个 |
| `npu-rknn` | Rockchip NPU | `rknpu` | ✅ preview | 已进 `OrtProvider` 枚举 |
| `npu-qnn` | QNN | `qnn` | ✅ | 高通 Hexagon HTP |
| `npu-vip9000` | **无上游 EP** | — | ❌ | 走 VeriSilicon fork 的 `vsi_npu`（OpenVX/TIM-VX）或 Paddle Lite NNAdapter（见 §4.1） |
| `npu-webnn` | WebNN（浏览器侧） | ort-web | ⚠️ | 浏览器路线单独看，不混进原生 build |
| `gpu-cuda` | CUDA / TensorRT | `cuda`/`tensorrt` | ✅ | 我们自己的 L40 就在这档 |
| `gpu-vulkan` | **ORT 无原生 Vulkan EP** | — | ❌ | 或经由 WebGPU EP（Linux 上落在 Vulkan/GL）；真要 Vulkan 得看 ncnn/MNN |
| `gpu-metal` | CoreML 已覆盖 GPU | `coreml` | ✅ | 同上表 coreml 行 |
| ⚠️ `gpu-directml` | DirectML | `directml` | ✅**预编译可用** | **Windows 上最普惠的加速器**：任何 DX12 显卡（Intel/AMD/NVIDIA 通吃）。本机就是 Windows，建议加 |
| ⚠️ `gpu-webgpu` | WebGPU | `webgpu` | ✅ experimental | 「能开浏览器的设备」这条兜底线；官方标注可能产生错误结果，别放默认路径 |
| ⚠️ `npu-cann` | CANN | `cann` | ✅ preview | 华为昇腾 |

**建议下一步（很小的改动）**：catalog 补 `gpu-directml`、`gpu-webgpu`、`npu-cann` 三项，
并给每个 `Backend` 加一个 `ep: Option<&'static str>` 字段——记下「这个后端最终由哪个 ORT EP 兑现」，
排障时不用猜（`vip9000` 这种没有上游 EP 的记 `None`，走厂家 fork / Paddle Lite）。

---

## 4. YOLO 侧的三条硬约束

### 4.1 算子边界：NPU 是 CNN 加速器，不是 Transformer 加速器

VeriSilicon **TIM-VX** 官方仓库自述「150+ ops」，支持 TFLite/TVM/Paddle Lite/**ONNXRuntime(Official)** 绑定；
但算子表明确缺 User attention 家族。A733/同族 NPU 的实测分析给得更直白：

| 大模型需要的 | VeriSilicon NPU |
| --- | --- |
| Attention | ✗ |
| LayerNorm / RMSNorm | ✗ / 不完全 |
| RoPE | ✗ |
| MatMul（大矩阵） | 算力/带宽不够 |
| FP16 / BF16 | 多数型号只有 INT8/INT16 |
| **KV Cache** | **完全没有对应硬件结构** |

→ **判据（写进 `verifier` 的那种）**：
① CNN 视觉模型（YOLO/SSD/PicoDet/MobileNet-SSD）→ 可尝试 NPU；
② 任何 Transformer/LLM（含 v13 路由器、RT-DETR 类）→ 只走 CPU/GPU，**别硬塞**。
这条同时保住我们之前那条「`GRPO_SYS` 冻结」的克制：不为用硬件而改模型语义。

**A7A 的正经路线有了实证**：Paddle Lite 的 TIM-VX 已验证模型清单里，躺着我们想要的东西——
`yolov5s_int8_640_per_channel`、`picodet_relu6_int8_416_per_channel`、`ssd_mobilenet_v1_relu_voc_int8_300_per_layer`。
也就是说：**「PicoDet + per-channel int8」已经在 VIP9000 同族 NPU 上被跑通过**，这是可引用的先例，不是推测。

### 4.2 许可证雷区（实测 SPDX，2026-09-20）

```
THU-MIG/yolov10                        AGPL-3.0   ← 常被误传为 Apache-2.0，实测不是
ultralytics/ultralytics                AGPL-3.0   ← v5/v8/v11/YOLO26 全家
Megvii-BaseDetection/YOLOX             Apache-2.0 ✅
PaddlePaddle/PaddleDetection           Apache-2.0 ✅（含 PP-PicoDet）
RangiLyu/nanodet                       Apache-2.0 ✅
```

AGPL-3.0 的杀伤点在**第 13 条网络条款**：只要用户通过网络用你的服务（SaaS/API/内部服务也算），
就要把整个衍生作品的源码公开。对要闭源出货的产品来说，等于「改 AGPL 或买 Enterprise」。
→ 默认内置/分发 **YOLOX 或 PP-PicoDet**；Ultralytics 权重放「**用户自备**」通道，
 lyco 只提供加载逻辑和推荐转换命令，不替用户承担 AGPL 义务。

### 4.3 NMS 归属 + 端到端模型的取舍

- NMS 放在 **Rust 侧**（不在 ONNX 图里）：确定性由我们控制，也避免各家 NPU 对 `NonMaxSuppression` 算子的支持差异。
- NMS-free 端到端模型（YOLO26 / YOLOv10）确实省一步后处理，但存在两个代价：
  ① 论文自己承认小模型有约 **1.0% AP** 落差；
  ② v10 引入的**大核 depthwise 卷积 + PSA 部分自注意力**在 NPU 上未必更快（业界反馈「有时反而不如 v8/v9」）。
- 结合 §4.2，**YOLOX / PicoDet + 我们自己写的 NMS** 是当下最稳的默认：
  版权干净、算子最保守（纯 CNN）、CPU 上也跑得动。

---

## 5. 三层落地顺序（对齐「能力保证 / 加速可选」）

| Tier | 内容 | 依赖 | 状态 |
| --- | --- | --- | --- |
| **T0 一定能跑** | ONNX 会话走纯 Rust `tract`（`ort` + `alternative-backend` + `ort_tract::api()`） | 零 | 今天就能做 |
| **T1 有就更快** | `ort` 默认后端 + 按 `backend::available()` 注册 EP 列表 | 目标机装了对应 runtime/驱动 | 逐个设备灰度 |
| **T2 浏览器** | `ort-web`（WebGL/WebGPU） | 一个标签页 | 排最后 |

能力形态不变（沿用能力拓宽结论）：**新能力 = 新 CLI 子命令，router 不参与、不重训**。
预计 `lycore vision detect --image X [--model yolox-nano] [--backend auto]`，
输出里必带 `RunMeta{backend, device, elapsed_ms, degraded}`——**降级必须可读**。

---

## 6. 明确不做

- ❌ 不为 A7A/VIP9000 单独写一条 NPU 后端路径（除非是 ORT `vsi_npu` 或 Paddle Lite NNAdapter 这种「有官方绑定」的接法）。单独维护 = 把普惠承诺做成负债。
- ❌ 不把 v13 路由器或任何 LLM 路径搬到 NPU（见 §4.1）。
- ❌ 不在默认路径提供 AGPL 权重的自动下载（见 §4.2）。

## 7. 开放问题（下一步要解决）

1. `ort` 无 stable 版：`Engine` trait 的边界怎么切最省事？
2. T0 用 tract 时，模型要重新从 PyTorch/ONNX 导出验证一遍算子覆盖（tract 与 ORT 支持面不同）。
3. VIP9000 到底走 VeriSilicon ORT fork（`vsi_npu`）还是 Paddle Lite NNAdapter？前者贴合 T1 架构，后者有官方验证清单。**倾向前者做长期，后者做存在性验证。**
4. 量化：NPU 多要求 per-channel INT8/UINT8，需在导出管线里固化量化脚本与精度回归门槛（mAP 掉 >2% 即拒绝）。
