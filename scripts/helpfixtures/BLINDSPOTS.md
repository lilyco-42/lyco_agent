# 已知盲区（快照基线里条数偏低/为零的项）

> 2026-09-25 起 CI 比对从「条数（fixtures.lock）」升级为「**逐字快照**」（`snapshots/<cli>.txt`）：
> 条数相同但命令被换掉的内容级回归也逃不掉。`fixtures.lock` 退役。
>
> ⚠️ 快照是**当前值，不是目标值**。数字低 = 盲区，本文件逐条记录。

## 首日审计四连（2026-09-24 固化基线 → 当天全修）

| fixture | 当时 | 现状 | 盲区性质 |
|---|---|---|---|
| `gh` | **2** | ✅ **34 条** | 版式 H：`auth:          描述` 冒号**贴在命令名后** → `is_subcommand_word("auth:")` 对 ':' 判否 → 41 条真命令全丢、只剩 flag 回退。修复=`split_colon_line`（冒号后 ≥3 空格宽间隔才认，挡散文释义）；HELP TOPICS 8 条文档页被版式 I 显式排除（`gh accessibility` 不是可执行命令） |
| `python` | **1** | ✅ **24 条** | 那唯一 1 条是**垃圾**：Arguments 段定义行 `file   : program read…` → `python file`。修复=定义列表守卫（说明以冒号开头=词条释义，不是命令表）+ flag 回退新增 python 版式（`-c cmd : 说明`，值逐字转写） |
| `sqlite3` | **0** | ✅ **44 条** | 单横杠长 flag（`-append` / `-cmd COMMAND`）被旧规则（只认 `--`）整类丢弃。修复=flag 回退认单横杠 + 值逐字转写（丢值=跑不起来的静默失效） |
| `ffmpeg` | **7** | ✅ **31 条** | **条数最会骗人的一例**：7 条看着「有数」，内容全是版本横幅垃圾（`libavutil      61.  1.101` → `ffmpeg libavutil`），且 out 非空堵死 flag 回退 ⇒ 真 flag 全丢。修复=版本横幅守卫（说明首 token 纯数字/点=版本号）→ 垃圾清零后回退触发，31 条真 flag 进表 |

**教训**：`gh/python/sqlite3` 是**条数基线**照出来的（数字低=可疑）；
`ffmpeg` 是**逐字审计**照出来的（数字看着合理，内容全是垃圾）。
条数永远不说明质量 —— 这是快照升级的直接证据。

## 为什么这些过去一直没被发现

池内那 6 个 CLI（docker / git / cargo / npm / kubectl / jq）**恰好都是同一类版式**：
缩进式「子命令 + 描述」，对齐靠双空格。

于是解析器只对这一类形态是对的。换一批陌生的（gh / python / sqlite3 / ffmpeg），立刻露馅。

**这是"池子同质化掩盖整类缺陷"的实测**（前两次：clap long-help、flag 值占位符）。

## 未修/待定

- `router::plan` / `plan_with_help` / `plan_choice` 仍是**有意休眠**（非缺陷）：
  `lycore do`（lbrush 人机闭环）直接用 choices::* 实现了判定式脊，人是模型；
  `plan_choice` 是将来接 Qwen/Needle 时的**唯一替换点**（换掉 judge 那一环）。
  接线审计结论：capability.grants（skill.rs 用）、choices（cmd_do/lbrush/jni_bridge 用）、
  t1gate（lbrush do 流程用）均已接线；削引号 bug 已于 09-23 修复。

## 修某项的流程

1. 改 `help_parse`
2. CI 的 `help-parse 池回归` 会红（快照逐字 diff 不一致）
3. 确认是**改进**（垃圾消失 / 条数增加 / 占位符补齐 / desc 去污染）而不是破坏
4. 从 artifact（`if: always()`，红了也有）下载 `snapshots/` **逐条人工核对**后覆盖
   `scripts/helpfixtures/snapshots/`，在本文件销掉对应条目
5. 池内其它项**必须零变化**（第 1 条硬线：补盲区不得误伤旧形态 —— 逐字快照直接保证）
