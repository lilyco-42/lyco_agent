# 新增执行类工具（shell_exec / file_write / schedule）— 2026-09-16

> 触发：用户 *"加这几个工具 用 brush 做 shell 跨平台 nushell 备用 shell"*（需求：**日常任务真能干活**）。
> 背景：原 7 个工具**全是内容处理类**，没有"执行/写文件/定时" → 「完成日常任务」不可能
> （见上一轮结论）。本变更补齐这三类。

## 一、依赖选型（`lyco` 信条 1：不臆造，已核实）

| 项目 | ★ | 许可 | 说明 |
|---|---|---|---|
| `reubeno/brush` | 2219 | MIT | **bash/POSIX 兼容 shell，Rust 实现**（`gh api` 核实，2026-09-15 活跃） |
| `nushell/nushell` | 40519 | MIT | 结构化 shell，作为备用（同上核实） |

均 MIT → 商用无碍。

## 二、shell 选择链（`tools_runtime::pick_shell`）

```
LYCO_SHELL(显式覆盖) → brush(跨平台首选) → nu(备用) → 平台默认(sh / cmd)
```
三者都只在「命令字符串」层面工作 → 可互换；缺失时优雅降级并在错误里提示安装。

## 三、三个新工具

| 工具 | 参数 | Capability | Risk | **Verifier** |
|---|---|---|---|---|
| `shell_exec` | `command`(必), `cwd` | Shell | **High** | `ExitCode`（exit=0） |
| `file_write` | `path`(必), `content`(必) | FileWrite | Medium | `FileExists`（存在且非空） |
| `schedule` | `spec`(必), `command`(必), `apply` | Shell+FileWrite | Medium | None |

**安全默认**：`schedule` 默认 **dry-run**(只返回 cron/schtasks 片段)，`apply=true` 才真正登记 —— 不可逆动作需显式开启。
**超时**：`shell_exec` 120s / `schedule apply` 30s（std 无内置 timeout，用 `try_wait` 轮询 + kill）。

## 四、改动面（单一真源四处同步，测试兜底）

`capability::TOOL_CAPS` · `skill::ALL_SKILLS`(+`VerifierId::{ExitCode,FileExists}`) ·
`llamacpp::CHAT_TOOLS` · `executor::execute` · `tests/executor_test.rs` · 各自断言 7→10。

## 五、验证（CloudStudio A10）

- 构建：`Finished`；`cargo test` **87 passed / 0 failed**（含 3 条新冒烟测试）
- 冒烟（真跑，非编译）：`file_write_creates_file_and_parents` ✓ · `shell_exec_runs_command` ✓ · `schedule_dry_run_returns_snippet_without_applying` ✓
- `lycore tools` → **tools count: 10** ✓

## 六、⚠️ 后果与待办

1. **模型需要重训**：现 v3 模型是 **7 工具**契约；工具集变 10 后它不知道新工具（推理期 schema 与权重不匹配）。
   → 需把新工具的训练样本补进 `fc_train_v3.py`（"帮我启动 Minecraft 服务器"→`shell_exec`、"写个启动脚本"→`file_write`、
   "每天早上 8 点自动启动"→`schedule`），重训并重跑 held-out（评测也要加这三类 + **参数正确率**）。
2. **参数正确率**仍未纳入评测（上一轮已指出）—— 这次一并补。
3. 端侧实际可用的 shell 取决于设备是否装了 brush/nu；未装则回落平台默认 `sh`/`cmd`（功能仍在，跨平台一致性略降）。
