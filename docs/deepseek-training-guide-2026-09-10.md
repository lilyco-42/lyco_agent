# DeepSeek 系论文与代码 — lyco 自训技术指导暂存

> 2026-09-10 | /lyco 预研 (ACQUIRE→REASON 阶段) | 所有仓库 gh api 实测验证 (星标/许可/更新时间)
> 关联: `research-base-model-2026-09-10.md` (R2 自训路线) | 用途: 自训 0.6-1.7B 端侧 agent 的技术依据

## 一、DeepSeek 官方仓库实录 (gh api 验证)

| 仓库 | ★ | 许可 | 更新 | 对 lyco 的价值 |
|---|---|---|---|---|
| deepseek-ai/deepseek-harness | 217k | - | - | 「Everything is a Plugin」架构理念 — 与 lyco SkillRegistry 同构, 值得研究 |
| deepseek-ai/DeepSeek-R1 | 92.0k | MIT | - | **纯 RL 训练 + 蒸馏方法论主参考** (见下) |
| deepseek-ai/DeepSeek-V3 | 104k | - | 2025-08 | MLA (Multi-head Latent Attention) + MoE 架构 + 高效训练工程 |
| deepseek-ai/DeepSeek-OCR | 23.9k | MIT | 2026-01 | **Contexts Optical Compression (arXiv 2510.18234)**: 图片→压缩上下文, 与 lyv「视频知识包」思路同源 |
| deepseek-ai/Janus | 17.8k | - | - | 统一多模态理解+生成 — VNN 视觉骨干候选 |
| deepseek-ai/FlashMLA | 12.9k | - | - | MLA 高效 kernel — 自训架构若采 MLA 可直接用 |

## 二、DeepSeek-R1 训练方法论 → lyco 自训映射 (核心)

R1 README 实测提取的两条关键技术声明:

### 2.1 Post-Training: 纯 RL on Base Model
> "directly apply reinforcement learning (RL) to the base model without relying on
> supervised fine-tuning (SFT) as a preliminary"

**对 lyco 的启示**: 我们不必先造海量 SFT 语料再微调——可以在「视频知识问答环境」里
直接跑 RL (GRPO), 用环境奖励(检索命中/OCR验证通过/切片正确)驱动学习。
这正是 FEE 论文(环境反馈>SFT)的工程化, 也是 lyco「学习闭环」的技术路线。
- 可行性: TinyZero (★13.2k) 已证明 **单卡级 RL 复现 R1-Zero 可行** (Qwen 0.5B/1.5B + GRPO)
- 工具: verl/HybridFlow (★23.4k, Apache-2.0, 活跃) 或 unsloth GRPO notebook (A10 友好)

### 2.2 Distillation: Smaller Models Can Be Powerful Too
> "reasoning patterns of larger models can be distilled into smaller models"

**对 lyco 的启示**: R1 官方蒸馏配方 = 大模型推理数据 → SFT 小模型 (1.5B/7B/8B)。
lyco 版: **Qwen3-8B (本地跑) 生成高质量工具调用轨迹 → 蒸馏进自训 0.6B**。
轨迹数据在 A10 上生成 (8B 推理快), 蒸馏 SFT 也 A10 可跑 — 完全符合「训练在 CloudStudio」。

### 2.3 DeepScaleR 处方 (1.5B RL 扩展案例)
DeepScaleR-1.5B-Preview: 1.5B 底座 + GRPO + 长上下文课程训练, 数学推理追平 o1-preview。
已证: **小模型 + 正确的 RL 课程 ≫ 同规模 SFT**。lyco 长任务训练照此处方设计课程。

## 三、训练工具链 (A10 兼容性筛选过)

| 工具 | ★ | 许可 | 用途 | A10 适配 |
|---|---|---|---|---|
| huggingface/open-r1 | 26.5k | Apache-2.0 | R1 完整开源复现 (GRPO/蒸馏全流程) | 单卡模式支持 |
| verl-project/verl | 23.4k | Apache-2.0 | HybridFlow RL 后训练框架 (字节) | 0.6-1.7B 单卡可跑 |
| unslothai/unsloth | 75.9k | Apache-2.0 | 快速微调+GRPO, 显存省 70%+ | **A10 首选** (notebooks 仓库 250+ 现成配方, 含 Qwen3 GRPO) |
| hiyouga/LlamaFactory | 74.7k | Apache-2.0 | 统一微调 UI (100+ 模型) | 备选 (交互式调参) |
| Jiayi-Pan/TinyZero | 13.2k | Apache-2.0 | R1-Zero 最小复现 (Qwen 0.5B/1.5B) | **最小化验证起点** (信条 5: 原子化构建) |

## 四、DeepSeek-OCR 与 lyv 的架构同源性 (重要发现)

DeepSeek-OCR 核心思想: **「上下文光学压缩」— 把长文本压成一张图, 视觉 encoder 解码**。
报告 97% 压缩比下精度保持 (arXiv 2510.18234)。

**与 lyv 的同构**: lyv 把「操作视频」压成「切片+关键帧+OCR 文字」的知识包 = 时间维度的
上下文光学压缩。两者都认可同一论点: **高分辨率视觉 token 是比文字 token 更高效的知识载体**。

**可借鉴**:
1. DeepSeek-OCR 的视觉 encoder 可作 VNN 骨干候选 (MIT 许可, 商用无障碍)
2. 其「图形化知识表示」路线与 LVK 格式互相印证 — lyv 格式设计有权威背书
3. 端侧 VNN 若接 DeepSeek-OCR 蒸馏版, OCR→VNN 级联可直接融进一个模型

## 五、Decide (build-vs-buy 判定)

| 能力 | 判定 | 依据 |
|---|---|---|
| RL 训练框架 | **adopt** verl 或 unsloth | 成熟框架 ≥80% 覆盖, 自研无意义 |
| R1 方法论 | **fork** (方法论移植) | 纯 RL+蒸馏处方移植到 lyco 环境 |
| DeepSeek-OCR encoder | **fork-extend** | VNN 骨干候选, 需接 lyv 级联 |
| lyco 环境定义 | **build** | 「视频知识问答环境」无人做过 — 我们的差异化 |

## 六、Act (下一步最小验证)

1. **最小化验证 (本周)**: unsloth GRPO notebook 改造 → Qwen3-0.6B 在「lyco 工具调用环境」
   跑 500 步 GRPO → 重跑 qwen_fc_test.py 看 60%→? (环境奖励: 调对工具=+1)
2. 数据侧并行: Qwen3-8B (本地/A10) 生成 1k 条「模糊诉求→正确调用」轨迹 → SFT 预热
3. 精读清单新增: DeepSeek-R1 论文 (RL 流程) / DeepSeek-OCR 论文 (arXiv 2510.18234)

## 七、Re-observe 条件

- GRPO 500 步后 FC 遵循度 <75% → 检查奖励函数设计 (可能需要 process reward)
- unsloth 在 A10 显存不足 → 降 0.5B 底座或换 LlamaFactory QLoRA
- DeepSeek-OCR 蒸馏跑不通 → VNN 骨干退回 ConvNeXt-Tiny (已有 export_convnext.py)

## 八、Act 结果 (2026-09-10 当轮验证)

**GRPO 200 步 (非 500), FC 遵循度 60% → 100%** — Re-observe 触发条件未命中, 闭环正向。
显存 10.2GB (A10 23GB 的 44%), unsloth 不再必需 — TRL 原生即可。
详细数据见 `research-base-model-2026-09-10.md` ⭐ 实测基线节。
下一轮: 真实轨迹数据 + 扩 epoch + τ²-Bench 长任务评测。
