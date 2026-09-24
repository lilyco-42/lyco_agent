# 已知盲区（fixtures.lock 里数字偏低的那些项）

> `fixtures.lock` 必须是**纯数据**，CI 直接 `diff`。说明写在这儿。
>
> ⚠️ lock 里是**当前值，不是目标值**。数字低 = 盲区，本文件逐条记录。

## 2026-09-24 首次固化时照出来的三个

| fixture | 条数 | 应该有多少 | 盲区性质 |
|---|---|---|---|
| `gh` | **2** | 几十条 | gh 有大量子命令（`pr` / `issue` / `repo` / `workflow` / `auth` / `api`…），几乎全丢 |
| `python` | **1** | 少量 | `python --help` 是 usage 行 + flag 列表，没有子命令形态 |
| `sqlite3` | **0** | 0（合理） | sqlite3 根本没有 CLI 子命令，help 是点命令风格（`.help`） |

## 为什么这些过去一直没被发现

池内那 6 个 CLI（docker / git / cargo / npm / kubectl / jq）**恰好都是同一类版式**：
缩进式「子命令 + 描述」，对齐靠双空格。

于是解析器只对这一类形态是对的。换一批陌生的（gh / python / sqlite3），立刻露馅。

**这是"池子同质化掩盖整类缺陷"的第三次实测**（前两次：clap long-help、flag 值占位符）。

## 修某项的流程

1. 改 `help_parse`
2. CI 的 `help-parse 池回归` 会红（条数对不上）
3. 确认是**改进**（条数增加 / 占位符补齐 / desc 去污染）而不是破坏
4. 更新 `fixtures.lock` + 在本文件里销掉对应条目
5. 池内其它项**必须零变化**（第 1 条硬线：补盲区不得误伤旧形态）
