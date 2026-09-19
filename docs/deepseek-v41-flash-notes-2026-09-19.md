# DeepSeek-V4.1-Flash 技术报告 精读笔记（2026-09-19）

- 报告：《DeepSeek-V4.1-Flash: Pushing the Limits of KV Cache Compression》，DeepSeek-AI，51 页
  （本地：`papers/DeepSeek_V41_Tech_Report.pdf`，抽文 `papers/v41_clean.txt`）
- 发布：2026-09-10；552B MoE；1M 上下文；**MIT 开源**（HF `deepseek-ai/DeepSeek-V4.1-Flash`）；API 名 `deepseek-flash`
- 定位：能力超越 V4 Pro，同时把 KV cache 压到极限 —— **标题就点明了它的真正卖点是"省缓存"**，不是刷分

## 1. 一句话概括"它是如何做到的"

**用架构不对称（CED + CSA2 + FP4 KV + SWA Bounded Replay）把长上下文成本砍掉一个量级，
再用"数据管线 + RL"把能力补回来 —— 后训练本身没有任何算法创新。**

报告原文（第 5 页）：
> our post-training introduces **no algorithmic innovation**: the recipe follows the standard paradigm of
> supervised fine-tuning (SFT) followed by reinforcement learning (RL) and on-policy distillation (OPD)...
> **All substantive changes lie instead in the data pipeline.**

## 2. 架构：三个层面的联合优化（第 4–5 页）

| 层面 | 设计 | 效果 |
|---|---|---|
| **模型结构** | **Causal Encoder-Decoder (CED)**：20 层因果编码器 + 20 层解码器；解码器全局 KV 由编码器末层隐状态投影而来 | prefill 每 token 激活 **8B**，decode **16B** —— 输入重的 Agent 场景极省 |
| | **CSA2**（Compressed Sparse Attention 2）：跨层复用全局 KV（含 main KV + indexer K）与 Top-K 索引；每层静态指定 **Full / Reindex / Reuse** 三种模式 | 去掉重复缓存；每层仍保留自己的 global Q 与 SWA KV |
| | 由 V4 的 CSA+HCA 混合改为**纯 CSA2** | 简化全局分支 |
| | **SWA Bounded Replay**：只重放最近 `n_win` 个 token 近似重建 SWA KV（V4 要重放 `L×n_win`） | **持久化 KV 再降到 V4-Flash 的 1/8** |
| **缓存精度** | 训练时就用 **FP4 全局 KV cache** | 性能仅边际下降；算力按 BF16/FP8/FP4 = 1 / 0.5 / 0.25 计权 |
| **部署** | **Single-Pass mHC** + **Mega-mHC** kernel（访存减半）；**Engram** 条件记忆（196B，按相关性选择性激活）；**DSpark** 投机解码（半自回归草稿 + 置信度调度验证） | 每层 Reuse 模式 prefill 仅 15 个 kernel、decode 11 个 |

**结果**：全局 KV 降到 **890 字节/token**（≈V4-Flash 的 1/4，HBM 1/4、SSD 1/8）；
相对初代模型 KV 缩小 **437 倍**；**上下文 4K→1M（256×）decode FLOPs 只涨 1/4**（近乎常数）。

## 3. 预训练（第 4 章）

- **45T tokens** 多模态语料；稀疏注意力**从 0 开始就在 64K 序列上训练，不做 dense warmup**。
- 数据构造的关键态度（对我们最有启发）：
  - 追求"**不同语料之间的整体性相互作用带来的独特信息增益**"，而非单样本质量。
  - **主动过滤"信息增益有限的模型生成内容"**（弱模型输出、低质机翻），并把这类内容称为
    **"implicit duplication（隐式重复）"** —— 因为它只是把已有信息换了个说法，
    **在长训练周期里有害**。
  - 引入领域专家制定细粒度数据质量评估维度；探索 model-in-the-loop 数据迭代。

> ⚠️ 这一条几乎是点名批评我们的做法：我们给路由器"扩说法"（同义改写、加语气词后缀）
> 正是**隐式重复**。这解释了为什么 v3/v4 在 heldB 上撞到 80% 天花板 —— **我们没有引入新信息**。

## 4. 后训练（第 5 章）：数据管线才是主战场

标准 **SFT → RL → OPD(on-policy distillation)**，无算法创新。真正的工作量在下面四件事：

### 4.1 Agent 任务合成（5.1.1）
- 把每个任务形式化为 **三元组 (problem, environment, verification system)**，
  按 **difficulty（任务不平凡）+ correctness（三者无关键缺陷）** 两个维度打分，
  **并用这两个维度作为 reward 迭代训练模型自己造更好的任务**。
- 任务全生命周期监控：任务被新的 RL run 用到后，产生的轨迹成为**再审计质量的新证据**。
- 通用 Agent：基于真实工作流**重建大量 mocked tools**（复刻真实工具的输入格式 / 输出结构 /
  API schema / 行为约束），并**成规模收集负面反馈与失败案例** → 生成单轮/多轮环境 →
  **系统性 replay 失败 + 针对已观测弱点做定向强化学习**。

### 4.2 合成任务上的 RL（5.1.2）
- 大规模**异步** RL；沿两个维度扩展：训练算力 + **scaffold 数量**。
- 把 rollout 执行解耦为 **agent sandbox** 与 **worker container**，中间是 scaffold-agnostic 控制层
  （把异构交互归一化成统一 trajectory schema）。
- **模型合并（model merging）来重新初始化后续 RL run**：把不同 scaffold / 配置下训出的 checkpoint
  合并，**把不同优化路径上的改进叠加起来**，是"聚合并行 RL 算力并继续 scaling"的简单实用手段。

### 4.3 可控推理投入（5.1.4）—— 与我们"低功耗"诉求直接相关
- 在 system prompt 前置一个标量：`Reasoning Effort: {effort}`（1–100）。
- 对每个 prompt 在每个 effort 档采样 M 个回答；**同一 (x, b) 子组内做 group-relative 优势**，
  不同 effort 档之间**不互相比较**。
- **长度惩罚系数随 effort 指数衰减**：`k(b) = k0 · exp(−(b − b_min)/τ)`，`τ = λ·Δb`。
  effort 越高 → 惩罚越弱 → 允许更长推理。
- 线上只暴露三档：**max=100 / high=75 / low=50**，同一份权重即可在 cost–quality 前沿上移动。

### 4.4 大规模 On-Policy Distillation（5.2.4）
- 后训练最后一阶段：**全词表 OPD，用 40+ 个教师模型**，覆盖所有领域，异步生成。
- 每个领域的最佳教师可能来自**不同开发阶段**，且教师之间、教师与学生的**架构可以不同**。

## 5. 对我们的直接启示（lyco_agent 端侧驾驶模型）

1. **别再"扩说法"了**。合成同义改写 = implicit duplication，information gain ≈ 0。
   要引入**新信息**：真实用户 query、或**更强教师**生成的、我们没写过的新指令/新失败模式。
2. **把任务写成三元组 (problem, environment, verifier)** 并**用 verifier 做 RL reward** ——
   这正是我们架构里"Verifier 与模型解耦"的落地方式。我们的 `hw` 命令语法天然可编程校验。
3. **失败回放 + 定向 RL**：把我们已观测到的失败（如 GRPO 的 `cargo new 之后要做什么` 退化成
   写长文、路由器 heldB 弱线索误判）**变成定向 RL 任务**，而不是重复跑同一批 prompt。
4. **模型合并**：把 v3/v4（以及后续不同配置）的 checkpoint 合并后再继续 RL，
   低成本地把不同优化路径的收益叠起来。
5. **可控投入（effort 标量）**：给端侧模型一个"低投入档"，直接对应你最初的低功耗诉求
   （token 更少 = 功耗更低），比事后 BitNet 转换更现实。
6. **教师蒸馏（OPD）** 是突破 0.6B 天花板的正当路径：需要一个强教师（API 或大模型）。
   报告明确说教师可以异构、可以多来源。
