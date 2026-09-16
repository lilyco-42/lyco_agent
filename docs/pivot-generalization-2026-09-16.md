# 方向纠正：泛化 > 内置工具（2026-09-16）

> 用户指令：*"所有 要加强 泛化性 你懂么 添加大量 CLI 训练 不要内置工具你懂么"*
> 这是对前几轮的**方向性纠正**，记录如下（含已核实的语料）。

## 一、纠正了什么

| 之前（错的方向） | 现在（纠正后） |
|---|---|
| 手写 65 条样本 + **硬编码 10 工具**契约（`CHAT_TOOLS`） | **大规模真实语料** + **工具由数据/运行时动态提供**（不内置） |
| 评测只测"我们的 10 个工具" | 评测要测**没见过的工具**（泛化），必要时用终端 agent 基准 |
| 加工具 = 改能力/技能/schema/executor 四处 + 重训 | 加工具 = **换一份工具描述**；模型靠"读描述选工具"（Hammer/TinyAgent 路线） |

**核心认识**：模型不该"记住 10 个工具名"，而该**读懂任意工具描述并选对**。
这正是 Hammer 的 function masking、TinyAgent/Gorilla 的 ToolRAG、TSCG 的 schema 压缩共同指向的东西。

## 二、已核实的数据源（`gh`/HF API 实测，非臆造）

| 语料 | 规模 | 许可 | 状态 | 用途 |
|---|---|---|---|---|
| **`NousResearch/hermes-function-calling-v1`** | 多 config（single-turn 1893 / glaive 5k / multi-turn…） | apache-2.0 | ✅ 可拉 | **任意工具**的 FC 监督 → 泛化 |
| **`nvidia/Nemotron-Terminal-Corpus`** | 5,689（`skill_based_mixed`；另有 easy/medium/adapters） | cc-by-4.0 | ✅ 可拉（需选 config） | ⭐ **真实 CLI 终端任务轨迹**（terminus-2 产出） |
| `MadeAgents/xlam-irrelevance-7.5k` | 7,500 | cc-by-4.0 | ✅ 可拉 | "该不调" + 多工具选择 |
| `Salesforce/xlam-function-calling-60k` | 60k | cc-by-4.0 | ⚠️ **gated**（需 HF token） | 备用；待用户提供 token |
| `harborframework/terminal-bench*` | — | — | ✅ | **泛化评测基准**（终端 agent 任务） |

**A10 直连 huggingface.co 实测 HTTP 200**（`hf-mirror.com` 反而 000）→ 语料可直接在 A10 拉取，无需中继。

## 三、v4 训练语料构成（`tools/fc_train_v4.py`）

```
hermes-function-calling-v1   → 用**每条数据自己的 tools**渲染 prompt (任意工具, 泛化核心)
xlam-irrelevance-7.5k        → 该不调 (answers 为空 → 不调用)
Nemotron-Terminal-Corpus     → CLI 轨迹 → 映射成 shell_exec(command=...) 调用 (CLI 训练)
(小比例) 我们自己的域样本      → 保住既有 7 工具行为不回退
```
- **SFT**（assistant-only loss masking）→ **GRPO**（reward: 工具名命中 / 该不调不调用）。
- **评测**：held-out 用**训练未见的 hermes 工具类别**（泛化）+ 我们的域套件（不回退）。

## 四、代码层要做的（"不要内置工具"）

lycore 侧需把工具集从**编译期常量**改为**运行时来源**：
```
tools_source::load(pack_dir / LYCO_TOOLS env)
  ├─ 优先: <pack>/tools_openai.json 或 $LYCO_TOOLS 指向的文件（任意工具集）
  └─ 回退: 内置 10 工具（保留仅为"零配置可用"，不再是契约）
```
`LlamaCppBackend` 不再直接读 `CHAT_TOOLS`，改用注入的工具集。这样**换一份 JSON 就换一套工具**。

## 五、诚实边界
- Nemotron-Terminal 的轨迹是 **DeepSeek-V3.2 在 Linux 容器里的命令**，映射成我们的 `shell_exec`
  是**近似**（多轮 batch 命令 → 单次调用），需在文档里标注为近似而非等价。
- `xlam-60k`（Hammer 的原始训练集）**因 gated 暂时不可用**；若用户提供 HF token 可补上。
- 全部为英语/通用域为主；**中文** 与 **视频知识（LVK）** 仍需我们自己的域数据兜底。
