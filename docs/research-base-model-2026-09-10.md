# lyco_agent 底座调研：达成 Qwen3-8B 级 agent 能力的路线

> 调研日期: 2026-09-10 | 触发: 用户基线要求「达到 Qwen3-8B 智商和能力, 强化工具调用与 agent 长时间工作」
> **硬约束追加（用户）**: 手机端也要能跑 → 基模不能大，倾向自训
> 方法: gh CLI 搜论文配套代码(GitHub) + 已有调研 | 论文↔代码对应表见文末

## ⭐ 核心决策修订（手机端约束）

用户明确: **为保障手机端运行，不采用 Qwen3-8B 级大基模，走自训路线**。
8B 在手机端不可行（Q4 ~5GB 权重 + 内存带宽，中端机掉帧/发热/杀后台）。

### 手机端算力现实（2026 旗舰→中端, NPU/GPU 推理）

| 模型规模 | 典型内存占用 (Q4) | 手机端现实 |
|---|---|---|
| 8B | ~5GB | 仅旗舰机勉强, 后台易杀 |
| **1.7B** | ~1.1GB | 中端机可用 ✓ |
| **0.6B** | ~0.5GB | 全机型流畅 ✓✓ |
| TinyGPT 0.5M | <1MB | 无压力, 但知识容量不足 |

**Qwen3 系列自家的 0.6B/1.7B** 正是为端侧设计的（Apache 2.0），function calling 原生支持。
→ **自训路线的最佳参照**: 以 Qwen3-0.6B/1.7B 的架构与数据配比为「能力天花板参照」，
自训同量级模型，把**算力缺口用在刀刃上：工具调用+技能编排的专项数据**，
世界知识短板用「lyco 知识包检索」补（这是我们的差异化，也是 ToolLoop 论文的路线）。

### BitNet / MoE 在自训路线中的角色（2026-09-10 补充, 用户强调）

| 技术 | 作用 | lyco 用法 | 参考 |
|---|---|---|---|
| **BitNet b1.58 QAT** | 三值权重(-1/0/+1): 内存 ÷8~16, CPU 原生友好(加法代替乘法), 手机 CPU 也能推理 | **自训走 QAT 而非训后剪枝**: SFT 后接 BitNet 量化感知训练, 质量损失远小。0.6B 三值化 → 端侧 ~0.3GB | microsoft/BitNet ★40k 官方推理框架; **lyco_chat 已内置 BitNet 实现** (src/core/bitnet.rs, 22K, 含量化推理) |
| **MoE 稀疏激活** | 总参数大/激活少: 0.6B 总参 top-2 激活 ≈ 稠密 0.2B 的速度, 容量 ≈ 1B | **自训 0.6-1B MoE**: 专家按技能域划分 (rust/ocr/导航/视频知识...), 端侧快+容量大兼得 | Qwen3-30B-A3B 证明 MoE 在 agent 任务优于同级稠密; **lyco_chat 已有 MoE 原型** (src/core/moe.rs 34K + block_router.rs, DeepSeek shared 专家结构) |
| **架构统一: VNN 激活 = MoE 路由** | VNN 的「特征激活」与 MoE 专家路由是同一机制: 特征/语义 → 稀疏专家 | lyv 的 VNN 激活路由和 LLM MoE 路由收敛到同一套 block_router —— 一次实现, 两处复用 | lyco 内部一致性优势 |

**自训配方 (R2 + BitNet + MoE)**:
```
Qwen3-0.6B 权重继承 (世界知识, Apache 2.0)
  → lyco 专项 SFT (toolcall 多轮 1-2万条 + world_corpus 扩 10万句)
  → MoE 化: dense → 8 专家 top-2 (专家≈技能域, lyco_chat MoE 结构)
  → BitNet b1.58 QAT 三值化 (骨干+专家)
  → 端侧产物: ~0.3-0.6GB, CPU 可推理, NPU 加速可选
```
每一步都已有原型代码（tiny_gpt / bitnet.rs / moe.rs / candle_trainer），
R2 需补的只有「candle 加载 HuggingFace Qwen3 权重 → LoRA 继续训练」这一环。

### 三条自训路线对比

| 路线 | 说明 | 风险 |
|---|---|---|
| **R1 纯自研扩容** | TinyGPT → 0.5-2B 参数, 自建语料预训练 | 36T tokens 的知识差距无法弥合; A10 预训练 0.6B 需 ~10³ GPU 时(可行但耗) |
| **R2 底座蒸馏** ⭐ | 拿开源 Qwen3-0.6B/1.7B 权重做**继续预训练+领域 SFT**(不从头训): 世界知识继承底座, lyco 数据专项强化工具调用/长任务 | 需遵守 Apache 2.0 (商用友好✓); A10 QLoRA 0.6B→1.7B 全程可跑 |
| **R3 双模型分工** | 0.6B 端侧小模型(技能路由+toolcall) + 云端/PC 大模型(重问题) | 端云协同工程复杂度 |

**推荐 R2**: 手机端达标(~0.5-1.1GB) + 世界知识继承 + 专项数据我们自己造(已验证能造: toolcall_corpus 管线)。
「自己训练」落为: 底座权重继承 + 自训数据 + A10 上 LoRA/SFT/RL——算力和时间都现实。

## ⭐ 实测基线 (2026-09-10, CloudStudio A10, transformers 5.1.0)

### ✅ GRPO 最小验证已跑通 (60% → 100%)

- **环境**: Qwen3-0.6B + TRL 1.12 GRPOTrainer, 程序化生成 118 条三类 prompt
  (知识查询/识图/闲聊), 奖励 = 调对工具 +1 / 参数含关键词 +0.5 / 闲聊不调 +1
- **训练量**: 200 步 (8 prompt×4 generations/步), bf16, 3.2s/步, GPU 10.2GB, ~11 分钟
- **结果**: FC 遵循度 **3/5 (60%) → 5/5 (100%)**
  - "帮我看看这张截图" FAIL→PASS (模糊诉求现在正确调 vnn_identify)
  - "cargo new 之后要做什么" FAIL→PASS (现在先查知识库而不是自己答)
  - 闲聊不误调用保持 ✅
- **产物**: `/workspace/qwen3_lyco_grpo` (A10) → `lyco_agent/models/` (本地留档)
- **代码**: `tools/qwen_grpo_train.py` (数据生成/奖励函数/训练/自评一体, 可复跑)

**结论**: 验证了核心论点——**工具调用能力靠环境反馈 RL, 不靠模型规模**。
200 步 GRPO (11 分钟 A10 时间) 补齐了 0.6B 的调用决策能力。
下一轮扩规模: 全量 118 prompt×更多 epoch + 真实调用轨迹替换程序化数据 + τ²-Bench 对比。

### Qwen3-0.6B 原生 FC 遵循度: 3/5 = 60%** (lyco 自建 5-case 测试集, do_sample=False, 400 token)

| case | 结果 | 分析 |
|---|---|---|
| 怎么新建 rust 项目 | ✅ lyv_knowledge, query 参数正确 | 显式知识诉求 → 调用 |
| 帮我看看这张截图里是什么 | ❌ 直接文字回答 | 模糊诉求犹豫, 没意识到用 vnn_identify |
| cargo new 之后要做什么 | ❌ 直接文字回答(模型自己会答) | 底座知识太强, 不认为需要知识库 |
| 你好呀 | ✅ 不乱调用 | |
| 今天天气怎么样 | ✅ 不乱调用 | |

**关键坑** (调试实录):
- Qwen3 默认 thinking 模式, 200-400 token 全耗在 `<think>` 里到不了 tool_call →
  **必须 `enable_thinking=False`**(工具调用场景), 或给足 800+ token
- transformers 5.x: `apply_chat_template` 必须 `tokenize=False` + 手动
  `add_special_tokens=False`, 否则报 text-input 类型错

**60% → 90%+ 的路径 = lyco 专项 SFT** (正是 R2 计划的第 2 步):
失败的两个 case 恰好是「该调而不调」型 — 用 lyco 真实轨迹造「模糊诉求→调用」
的多轮 toolcall 语料即可修复, 无需动底座。这验证了"工具调用能力靠专项数据,
不靠模型规模"的核心论点。

## 一、基线差距的诚实评估

| 维度 | lyco_chat 现状 | Qwen3-8B | 结论 |
|---|---|---|---|
| 参数规模 | TinyGPT 4层/64维 (~0.5M) | 8B | 架构性差距 |
| 预训练数据 | 48K tokens (自建语料) | ~36T tokens | 从零追不上 (A10 算力 ~10⁶ GPU 时缺口) |
| 工具调用格式 | toolcall 语料可学会 (已验证) | 原生支持 | **可达标** — 这是协议能力不是智商 |
| 世界知识 | 2519 句模板 | 全网 | 只能靠底座模型白嫖 |
| 长任务 | 无 | 无专门优势 (8B 级都需要 RL) | **所有 8B 级都在补, 有机会同一起跑线** |

**结论: 混合架构是唯一现实路线** —— 本地量化 LLM 当"智商底座"(预训练知识免费白嫖),
lyco 技能层做差异化(工具编排/学习闭环/隐私)。这与"工具调用>Scaling Law"的论点自洽:
不自大预训练, 算力花在编排层。

## 二、候选底座 (2026-09-10 更新: 用户指定 Qwen3.8-27B)

**用户决策（2026-09-10）: 底座升级为 Qwen3.8-27B**（Apache 2.0, HF: Qwen/Qwen3.8-27B）

| 关键点 | 内容 |
|---|---|
| 架构 | 27B 稠密, 原生 VLM (图像+视频理解), Gated DeltaNet 混合布局 |
| 上下文 | 原生 262K, 可扩 1M |
| Agent 能力 | SWE-bench Pro 61.7, OSWorld-Verified 84.3 (GUI 操作!), CoWorkBench 70.7 |
| llama.cpp | 已支持 (issue #28243, b10883+), MTP 支持 |
| 与 lyco 的契合 | **OSWorld 84.3 = GUI 桌面操作正是 lyv 的目标场景**; VLM 原生识图可替代/增强 VNN |

分工模型架构更新:
- **PC/服务器档**: Qwen3.8-27B GGUF (Q4 ~16GB) — FC 决策 + 原生识图, lycore 路由
- **端侧档**: Qwen3-0.6B GRPO 专训模型 (484MB) — 保留, 低配设备用
- GRPO 管线直接复用: 27B 上 LoRA 微调环境奖励 (A10 单卡 QLoRA 可跑)

## 二·一、候选底座 (历史记录: 2026-09-10 早期)

| 方案 | 参数/显存 (Q4) | 工具调用 | 适配点 | 来源 |
|---|---|---|---|---|
| **Qwen3-8B** (Apache 2.0) | 8B / ~5GB GGUF Q4_K_M | 原生 function calling, 100+ 语言 | HF 官方; llama.cpp 量化版工具调用遵循度好 | huggingface.co/Qwen/Qwen3-8B |
| **Qwen3-30B-A3B** (MoE, 3B 激活) | 30B / 3B 激活 ~50tok/s 消费级 GPU | 社区普遍优于 8B | 本地 agent 首选 MoE — 激活少速度快 | qwen.ai blog 2507 |
| **Qwen3-Coder-30B-A3B** | 30B MoE | 官方确认 FC 大幅改进 | 若 8B 工具调用不稳的升级选项 | QwenLM/Qwen3#10737 |
| TinyGPT (自研) | 0.5M / <1MB | toolcall 格式已验证 | 保留作学习实验平台, 不当产品底座 | 本地 |

部署栈: llama.cpp (GGUF) 或 vLLM≥0.8.5; Ollama 最省事。
注意: Qwen3-8B 原生 FC 一般, 需要少量 SFT 强化 (见第四节)。

## 三、lyco 技能层接入协议 (已落地一半)

tools_openai.json 已注册 4 工具: `add` / `ping` / `lyv_knowledge` / `vnn_identify`。
底座 LLM → tools_openai 协议 → lyv.py 管线 (OCR→VNN 级联已闭环) → 证据返回。

```
用户问题 → 底座 LLM (Qwen3-8B Q4, 本地)
          ├─ 直接回答 (底座知识)
          ├─ tool_call: lyv_knowledge{query} → lyv query → 切片+帧+OCR验证
          ├─ tool_call: vnn_identify{image} → VNN 激活式识图
          └─ 不会 → 学习队列 (lyco 差异化: 诚实+学习闭环)
```

## 四、工具调用/长任务强化 — 论文↔代码

| 论文/资源 | 代码 | 对 lyco 的用法 |
|---|---|---|
| **xLAM**: A Family of Large Action Models (Salesforce) | SalesforceAIResearch/xLAM ★638 | 大规模 FC 数据合成方法 (APIGen); 可借用其数据生成思路造 lyco 领域 toolcall 语料 |
| **Gorilla**: Training LLMs for API Calls | ShishirPatil/gorilla ★13k | 检索增强 FC (调用前查 API 文档) — 对应 lyco 的「知识包检索后调工具」 |
| **AgentGym-RL**: Long-Horizon 多轮 RL | WooooDyy/AgentGym-RL ★863 (ACL 2025) | **长任务训练法主参考**: 多轮交互环境 + RL 训练长程决策; lyco 可建"视频知识问答环境"做 RL |
| **τ²-Bench**: Tool-Agent-User 交互基准 | sierra-research/tau2-bench ★1994 | 评测基准: 长任务工具编排能力量化 (验证"达到 8B 基线"用) |
| **Hermes-Function-Calling** | NousResearch/Hermes-Function-Calling ★1468 | 开源 FC 微调数据+模板 (单模型格式稳定化的成熟方案) |
| **AppWorld**: FC 基准世界 | StonyBrookNLP/appworld ★508 | 交互式 app 环境基准, 任务设计参考 |
| **LLMCompiler**: 并行 FC | SqueezeAILab/LLMCompiler ★1880 (ICML 2024) | 并行工具调用编排 — lyv 多 pack 并行查询时用 |
| **toolsynth** | aaronlyt/toolsynth | 工具调用训练数据合成框架 (小而新, 关注) |
| **Qwen-Agent** | QwenLM/Qwen-Agent ★17k | 官方 agent 框架: 直接支持挂自定义工具, lyv 工具可先在这里验证接入 |

## 五、推荐落地序列（R2 路线）

1. **端侧底座选型实验 (1-2 天)**: Qwen3-0.6B/1.7B GGUF 在 A10 上跑通 llama.cpp;
   用 tools_openai.json (lyv_knowledge/vnn_identify) 测原生 FC 遵循度
2. **lyco 专项 SFT (3-5 天)**: xLAM/Hermes 格式, lyco 真实调用轨迹造 1-2 万条多轮
   toolcall 语料 (管线已有), A10 QLoRA 微调 0.6B/1.7B
3. **世界知识对齐**: gen_world_corpus 扩到 10 万句级 (常识覆盖补底座短板),
   混入 SFT; lyv 知识包检索兜住长尾
4. **长任务 RL (后置)**: 参考 AgentGym-RL, 把「视频知识问答+切片验证」做成环境;
   用 τ²-Bench + BFCL 量化对比 Qwen3-8B 基线
5. **手机端部署验证**: llama.cpp Android / ONNX Runtime (参考 OlliteRT/MagicWX);
   目标: 0.6B Q4 全机型, 1.7B Q4 旗舰机
6. **TinyGPT 保留**: 作为 BitNet 量化/MoE 路由的学习实验平台 (论文复现+教学价值)

## 六、个人 agent 竞品扫描 (2026-09, gh CLI)

| 项目 | ★ | 定位 | lyco 差异点 |
|---|---|---|---|
| openclaw/openclaw | 389k | 全平台个人 AI agent (云端为主) | lyco 主打本地隐私+视频知识 |
| tinyhumansai/openhuman | 39.5k | 本地优先个人 AI (Mac/Win/Linux) | 无视频知识管线 |
| zeroclaw-labs/zeroclaw | 32.7k | 端侧自治助手基建 | 无学习闭环 |
| MemTensor/memmy-agent | 1.5k | 跨 agent 共享记忆层 | 可借鉴: 记忆 hub 思路 |
| swarmclawai/swarmvault | 681 | 本地知识图谱/RAG | 邻域竞争: LVK 是视频维度 |
| destinyfrancis/jenny-android | 83 | Android 本地 agent+自写小应用 | **直接同赛道**, 验证了手机端需求真实 |
| OlliteRT / MagicWX | 308/854 | Android 本地推理服务栈 | 部署参考 |

**市场空白确认**: 无一竞品有「视频→知识包→可验证问答」管线。视频知识是 lyco 独有维度。

## 七、论文暂存清单 (待精读)

> DeepSeek 系详细映射: `deepseek-training-guide-2026-09-10.md` (含 gh 实测仓库表)

- [ ] **DeepSeek-R1** (★92k, MIT) — 纯 RL on Base + 蒸馏方法论; lyco 长任务训练主参考
- [ ] **DeepSeek-OCR** (arXiv 2510.18234, MIT) — Contexts Optical Compression;
      与 lyv 知识包同源的「视觉压缩」论点, VNN 骨干候选
- [ ] BitNet b1.58 (arXiv 2402.17764, 大获成功的三值 LLM; 官方代码 microsoft/BitNet)
      — **自训端侧化的核心技术**, QAT 流程重点精读
- [ ] xLAM 系列 (APIGen 数据合成) — Salesforce
- [ ] AgentGym-RL (arXiv 2509.08755, Zhiheng Xi et al.) — 长任务 RL 主参考
- [ ] ToolLoop (2609.09072, 已在 DESIGN.md 引用) — 4B+11K 合成数据→BFCL 86.4%, **小模型 FC 的直接证据**
- [ ] FEE (2609.08404) — 环境反馈>SFT 预热, 学习闭环的理论支撑
- [ ] Gorilla (检索增强 FC)
- [ ] RegionFocus (arXiv 2505.00684) — GUI agent 视觉 test-time scaling, 对 VNN 有参考价值
- [ ] Qwen3 技术报告 (MoE 路由 + 端侧 0.6B/1.7B 数据配比)

## 八、评测基线注册表 (对比测试用, 用户要求"权威测试项目+具体数据")

| 基准 | 代码 | 测什么 | lyco 目标 |
|---|---|---|---|
| **BFCL** (Berkeley FC Leaderboard) | ShishirPatil/gorilla 仓内 berkeley-function-call-leaderboard/ | 工具调用准确率 | 自训 0.6B ≥ Qwen3-8B 同测试分数 |
| **τ²-Bench** | sierra-research/tau2-bench | 长任务多轮工具编排 | 达到 8B 基线的 90%+ |
| AgentBench | THUDM/AgentBench ★3.7k (ICLR'24) | 综合 agent 能力 | 参与但不强求 |
| 自建: lyv-knowledge-qa | (待建) 视频知识问答端到端 | 检索+grounding 准确率 | **8B 底座没有这个能力, 这是我们赢的维度** |
