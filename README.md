<div align="center">

# lyco_agent

**能学、能验证、能执行**的本地智能协助助手

视频知识 → 可验证问答 → 学习闭环 —— 单位算力的系统能力上，工具调用与学习能力比放大参数更划算

</div>

## 它是什么

lyco_agent 把「操作演示视频」变成「可提问的知识库」：

```text
学习: 视频 → ffmpeg 抽帧 → ASR 转写 (CloudStudio GPU) → OCR 画面验证
      → 时间切片 + 关键帧 + 文字 = 知识包 (.lyv)

问答: "怎么新建 rust 项目?"
      → 检索命中 cargo new (ASR∩OCR 双源强关联)
      → 返回: 视频切片 3.0s-9.8s + 关键帧 + OCR 验证过的文字

诚实: 问了知识库里没有的 → 直说"还没学会", 记入学习队列 → 回流训练
```

**为什么不是又一个 RAG**：GUI 操作类知识（第三层菜单在哪个按钮）纯文字回答天生别扭。
lyco 的回答带**时间戳切片 + 关键帧截图 + OCR 量化验证**——证据可核验。

## 两层实现

| 层 | 语言 | 职责 | 状态 |
|---|---|---|---|
| 训练/原型 | Python | lyv 管线原型、GRPO 训练、agent loop 验证 | 已验证 |
| 部署运行时 | Rust (`lycore/`) | 检索/执行/验证/学习回流, 零 Python 依赖 | v0.1 |

## 分工模型架构 (2026-09-10 定案)

多任务共训实验证明 0.6B 容量不足以多任务（v5: 双退化），定案为**专训模型×路由**：

| 部署档 | 模型 | 体积 | 职责 |
|---|---|---|---|
| GPU 服务器 (双 A10+/48GB) | Qwen3.8-27B AWQ (原生 VLM) | 15.6GB | FC 决策 + 原生图像/视频识图（OSWorld 84.3）—— 权重已就位 |
| A10 单卡 (24GB) | Qwen3-8B AWQ | 5.7GB | FC 决策 + 知识库问答 —— 权重已就位远端 |
| 端侧 (无 GPU/AMD iGPU) | Qwen3-0.6B GRPO 专训 (CPU llama.cpp) | 484MB | tool_call 决策（FC 100%） |
| 端侧 | Qwen3-0.6B rewrite 专训 | 484MB | 口语化查询改写（64%, 迭代中） |

> 注意: CloudStudio 平台收割所有非 kernel 长驻进程（实测 setsid sleep 也不活），
> 常驻推理需 GPU 服务器/推理云/本机——CloudStudio 只适合会话内任务（训练/评测）

lycore 的 ModelBackend trait 按任务路由到不同后端，每能力独立训练独立替换。

## 快速开始 (Rust 运行时)

```bash
# 构建
cargo build --release -p lycore

# 学习一个视频 (SRT 驱动; ASR 路径见训练侧)
lycore learn --video demo.mp4 --srt demo.srt --pack ./mypack

# 问答 (纯检索模式)
lycore ask --pack ./mypack "怎么新建 rust 项目"

# 接上本地大模型 (llama.cpp server --jinja)
lycore ask --pack ./mypack --llama http://localhost:8081 "怎么运行项目"

# 收集"不会的" → 训练任务回流
lycore harvest --pack ./mypack

# 部署体检
lycore doctor --pack ./mypack
```

## Agent 架构

```
用户问题
  → 模型决策 (Qwen3-0.6B-GRPO / 任意 OpenAI 兼容 LLM)
      ├─ lyv_knowledge  → 知识包检索 (sqlite/FTS) → 切片+帧+文字
      ├─ vnn_identify   → OCR 失败时的内部识图 NN (特征激活式)
      └─ 无匹配 → 学习队列 (诚实说不会)
  → 多轮回填 ≤4 轮 → 最终回答

学习闭环: 端侧 NO_HIT → learning_queue.jsonl
        → lernen::harvest → training_tasks.jsonl
        → CloudStudio GRPO/SFT → 新模型下发
```

## 训练侧亮点 (CloudStudio A10 实测)

| 实验 | 结果 |
|---|---|
| Qwen3-0.6B 原生 FC 基线 | 60% 遵循度 (5-case) |
| GRPO 200 步 (工具环境奖励) | **100%** — 11 分钟 A10 |
| 多跳课程 GRPO 250 步 | prereq 驱动 4 跳任务链 |
| 世界知识字级语料 | 2519 行合并训练 |

> 工具调用能力靠环境反馈+专项数据, 不靠模型规模 —— 这不是论点, 是上面四行数据。

## 设计文档

- [DESIGN.md](DESIGN.md) — 架构/论点/实验记录/已知经验
- [docs/research-base-model-2026-09-10.md](docs/research-base-model-2026-09-10.md) — 底座选型与竞品
- [docs/deepseek-training-guide-2026-09-10.md](docs/deepseek-training-guide-2026-09-10.md) — R1/BitNet/GRPO 技术映射

## 许可

MIT
