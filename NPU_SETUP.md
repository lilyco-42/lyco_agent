# lyco_agent NPU (Vivante VIP9000) 落地指南

调研日期 2026-09-14。面向 Radxa Cubie A7A (全志 A733)。
**先跑 `scripts/npu_probe.sh` 再决定要不要往下走 —— 内核分支决定一切。**

## 0. NPU 能做什么 / 不能做什么

这块 NPU 是 **vision/CNN 加速器顺带能跑极小 LLM**，不是 LLM 加速器。
结论来自 `petayyyy/a733_npu_driver`（同芯片，最完整的公开验证）。

| 类别 | 状态 | 实测 |
|---|---|---|
| 视觉 CNN / encoder | ✅ | MobileCLIP-S0 **22.6 ms/frame**；SmolVLM SigLIP 塔上 NPU → LLM 留 CPU，e2e 通过 |
| 语音 | ✅ | `Ronin-1124/cubie-a7a-voice-assistant`（**同款 A7A**）：Zipformer KWS/ASR + HiFi-GAN v2 声码器全在 NPU |
| Embedding 检索 | ✅ | `waz664/vip9000-embeddinggemma`：EmbeddingGemma-300M，cos **0.944** |
| 视频检测 | ✅ | `frigate_npu_vivante`：Frigate on VIP9000 |
| 极小 LLM | △ | SmolLM2-135M 20.7 tok/s / 360M 8.4 tok/s，**但窗口 W≤64**（无 KV-cache） |
| **Qwen 系 LLM** | ⛔ | Qwen2.5-0.5B 导出 cosine 0.236(int16)/0.541(FP16)、BF16 `vnn_VerifyGraph -3`；SmolLM2-1.7B `gen_nbg` segfault |

**对 lyco_agent 的机会，按 ROI 排序：**

1. **`lyv_knowledge` 语义检索升级**（价值最大）—— 现为 sqlite/FTS 关键词匹配，
   换成 NPU 上的 embedding 检索，命中率是质变。同芯片已验证可行。
2. **`vnn_identify` 上 NPU**（门槛最低）—— 图片分类是 NPU 本行，`ai-sdk` 自带现成量化模型。
3. `rembg_remove` (u2net) —— U-Net 有 upsample/concat，NPU 支持度**未验证**，优先级最低。

附带收益：petayyyy 实测 NPU 吃掉视觉后「6 个 A55 核空出」→ 对我们意味着
chromium / ffmpeg 能吃满 8 核，而那才是 `html_render_video` 的大头。

## 1. 前置检查：内核分支决定一切

```bash
uname -r
ls -l /dev/vipcore    # 期望: character device, major:minor 199,0
```

| 内核 | 结果 |
|---|---|
| `5.15.147-21-a733` (Radxa vendor/Tina) | ✅ petayyyy 在此跑通全部用例 |
| `6.6.98-sun60iw2` (Orange Pi) | ✅ 可用 |
| `6.6.98-4-aw2511` (Radxa 6.6 BSP) | ⛔ **hang**：`VIPDRV_WAIT_TASK status=-1`、`VIP not going to idle` |

`awesome-radxa-a733` 判「6.6 两条路全 hang」测的是 galcore/TIM-VX；
成功案例走的是 **Allwinner VIPLite**。若板子是 6.6 且 hang，
**不要去调 galcore**，直接评估刷 5.15 镜像的代价。

## 2. 板端 SDK

```bash
git clone https://github.com/ZIFENG278/ai-sdk ~/ai-sdk
```

关键路径（Radxa）：
- VIPLite 库：`~/ai-sdk/viplite-tina/lib/aarch64-none-linux-gnu/v2.0/`
  （`libVIPhal.so`、`libNBGlinker.so`）
- 示例运行器：`~/ai-sdk/examples/vpm_run/vpm_run`（不存在则用 SDK 里的 Makefile 编）
- 现成模型：`~/ai-sdk/models/` → `mobilenet_v1_1.0_224_quant`、`resnet50-sim`、
  `inception_v1`、`lenet`、`lstm_mnist`、`MobileNetV2_Imagenet`
- 转换脚本：`~/ai-sdk/models/pegasus_*.sh`（板上转需完整 SDK，一般放主机做）

## 3. 最小验证（不动主机工具链，约 20 分钟）

目标：**用 SDK 自带模型在 NPU 上跑出一次推理**。这一步通了，后面才值得投入。

```bash
bash scripts/npu_probe.sh            # 环境体检
# 随后按 README 的 vpm_run 用法跑 models/mobilenet_v1_1.0_224_quant
```

Gate：
- G0 `/dev/vipcore` 存在且 `uname -r` 是 5.15/受支持分支
- G1 `libVIPhal.so` 找得到、`vpm_run` 能执行
- G2 现成 NBG 跑通并输出耗时

G2 不过 → 停在这里，把 `npu_probe.sh` 输出发回来，别往下投。

## 4. 三条路线

### 4.1 vnn_identify → NPU（先做这个）

`ai-sdk/models/mobilenet_v1_1.0_224_quant` 是现成的量化分类网络，
与 `vnn_identify`（终端/GUI/自然/文档 四分类）同类。
路径：用自己的数据微调或直接替换成 ImageNet 版分类头。

### 4.2 lyv_knowledge → embedding 检索（价值最大，工程量也最大）

参考 `waz664/vip9000-embeddinggemma`（Cubie A7S，同 A733）：

- EmbeddingGemma-300M，FP32 `seq128` transformer graph → VIPLite NBG（约 404 MB）
- CPU 侧负责：tokenize、embedding lookup、masked pooling、dense projection tail、L2 norm
- **关键坑**：必须给 NPU 图加 attention-bias 输入，否则 cosine 只有 0.747，
  加上后回到 **0.944**
- 需要 §5 的主机工具链做 ONNX → NBG 转换

### 4.3 rembg (u2net) → NPU（待验证，最后再碰）

u2net 输入固定 320×320 属静态 shape，理论适配 NBG；但 U-Net 的
upsample/concat 在 ACUITY 上的支持度无人验证过。**先做探针再决定。**

## 5. 主机工具链（仅在需要转自己的模型时）

来自 `petayyyy/a733_npu_driver` docs/01-setup-host.md：

- Docker 镜像 `ubuntu-npu:v2.0.10.1`
  下载：https://netstorage.allwinnertech.com:5001/fsdownload/Mh23BhPHq/docker_images_v2.0.x.zip
  （**约 11 GB**，内含 ACUITY toolkit 6.30.22、`pegasus.py`、`pegasus_export_ovx_nbg.sh`）
- 至少 30 GB 空闲磁盘
- 辅助脚本：`git clone https://github.com/ZIFENG278/ai-sdk work/ai-sdk/...`

⚠️ 只有 §4.2 / §4.3 需要它。§4.1 用现成模型即可跳过。

## 6. 已知坑

- **NPU 同一时间只能跑一个网络**，多个消费者要自己排队（`Ronin-1124` 实测）。
  lyco_agent 若同时开 `vnn_identify` + embedding 检索，必须加串行队列。
- NBG 是**静态 shape**，任何动态输入都要重导。
- 65536 维上限；int8 (pcq) 在大深度下质量崩，实际可用精度偏 int16。
- 板端转换慢且吃内存，尽量在主机（Docker ACUITY）做。
- `/dev/vipcore` 的 major:minor 应为 199,0，不对就是驱动没起来。

## 7. 待办

- [ ] 板子恢复后跑 `scripts/npu_probe.sh`，把输出贴回来
- [ ] 确认 `uname -r` 分支 → 决定要不要刷 5.15
- [ ] G2 通过后开 §4.1
- [ ] §4.2 需要先备好 11 GB 主机工具链
