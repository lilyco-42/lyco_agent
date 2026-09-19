# lyco_agent 项目 — Agent 引导

Rust 实现的轻量化 Agent 运行时(`lycore`) + 分工模型训练实验平台。

## 🗺️ Agent 工作区域(2026-09-19 划定, 多 agent 并行防踩线)

- **本仓 `lyco_agent`**(`lycore/` `tools/` `docs/` `scripts/` `smoke/` `models/`)=
  **lyco_agent 负责 agent 的工作区域**。
- **`lilyco` 主框架仓**(本机 `D:/Code/lilyco`, 21 crate, CLI/Web/MCP/TUI 四端)=
  **另一 agent 维护**——本仓任务不碰其代码, 跨仓需求走沟通, 勿直接改对方文件。
- **`lyco-ops`**(本机 `D:/Code/p2-lyco_ops`, remote `github.com/lilyco-42/lyco-ops`)=
  lyco_agent 侧运维脚本仓(lain42 计费 / CloudStudio 训练脚本 / 板端部署), 归本 agent。

## ⛔ 算力约束(硬性, 2026-09-10 用户明确指令)

**验证只允许占用云端算力; 本地 ASR / 训练 / 大批量推理是禁止的。**

- 本地**禁止**: 跑 ASR(`faster_whisper` 等)、模型训练、大批量推理、
  下载/加载 GB 级模型做实测。这些会长时间占满本机 CPU/GPU, 卡住用户。
- 本地**允许**: `cargo check` / `cargo build` / `cargo test`、代码编辑、
  轻量校验、产物下载与归档。
- 训练 / 推理 / ASR 一律走 **CloudStudio 云端 GPU**
  (spaceKey 与 JWT 获取流程见 [cute-pet/docs/cloudstudio-access.md](https://github.com/lilyco-42/cute-pet/blob/main/docs/cloudstudio-access.md))。

## 仓库约定

- **大文件不入库**: `.gitignore` 已整目录忽略 `models/`(含 `.gguf`, 单文件 462M)、
  `pack_merged.tar.gz`。训练产物走 Release / 对象存储分发, 不要 `git add -f` 强塞。
  > 教训: 只写 `models/*.tar` 会漏掉 `.gguf`, 曾导致近 1G 量化模型差点入库。
- **提交前自检**: `git status --short` 确认没有 GB 级文件被暂存。

## 关键文档

| 方向 | 文档 |
|---|---|
| 底座模型选型调研 | `docs/research-base-model-2026-09-10.md` |
| 训练指南 | `docs/deepseek-training-guide-2026-09-10.md` |
| 云/板端运维脚本（CloudStudio A10 编排、RADXA/NPU 部署） | [lilyco-42/lyco-ops](https://github.com/lilyco-42/lyco-ops)（2026-09-19 自本仓拆出） |

## 当前底座决策(2026-09-10)

用户指定 **Qwen3.8-27B** 作为底座(原生 VLM, Apache 2.0 可商用),
分工架构为双档: 0.6B 端侧专训模型 + 云端 27B 决策/识图后端。
llama.cpp 自 b10883 起支持 Qwen3.8 架构(见 ggml-org/llama.cpp issue #28243)。
