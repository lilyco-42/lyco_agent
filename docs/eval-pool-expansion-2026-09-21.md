# 评测池扩充：oha + lilyco CLI 带来的三类新覆盖（2026-09-21）

> 用户提议：**「选定一批 github 新的工具和 cli？我推荐我的 oha 和 lilyco cli」**
>
> 本文档回答该提议，并把「本机实测」的结果写下来。
> **结论先行：建议采纳，且这两个恰好补上了现有池的两个盲区 ——
> 其中一个当场暴露了一个让 CLI 完全不可用的真 bug。**

---

## 一、先说最有价值的发现：oha 把解析器打穿了

### 1.1 现场

```bash
$ lycore help-parse --cli oha --top 12
=== oha: 解析出 0 条动作（只读 0 条），展示前 12 ===

--- 注入用 schema（形态与人工 schema 一致）---
当前可用命令（oha 域，只读）：
只使用上面列出的命令；与上面命令无关的请求输出 (无需调用硬件命令)。
```

**0 条动作 → 注入集为空 → 这个 CLI 完全不可用。**

### 1.2 根因：`oha` 是 clap 的「描述在下一行」版式，而解析器只认「同一行」

oha 的 help（`clap` 4.5 derive，`#[command(long_about = None)]`）：

```
Options:
  -n <N_REQUESTS>
          Number of requests to run. Accepts plain numbers or suffixes: ...
  -c <N_CONNECTIONS>
          Number of connections to run concurrently. ...
```

**flag 名与描述被换行分开了。** 而 `split_line` 的判据是「**2 个以上空格**分隔
同一行的两段」→ 这种行切不出 `(cand, desc)` → 返回 `None` → 跳过。

### 1.3 这不是「oha 特殊」，是**整个池子的结构性盲区**

| CLI | flag 行数 | 同行带描述 | 能否解析 |
|---|---|---|---|
| docker | 5 | 5 | ✅ |
| cargo | 4 | 4 | ✅ |
| jq | 17 | 17 | ✅ |
| git / npm / kubectl / curl | 0 | 0 | ✅（走子命令路径） |
| **oha** | **14** | **0** | ❌ **0 条** |

**现有池里没有一个 CLI 是「描述在下一行」的版式** —— 因为池子是按
「我们手上有哪些 `--help` 样本」攒的，而**攒的人（我）一直用的是同一类 CLI**。

> ⚠️ **这是泛用性测试的经典失效模式**：测试集由被测物的作者挑选，
> 于是系统性地漏掉了他不知道的那一类。
> **oha 的价值不是「又一个 CLI」，而是它是从池外来的、由别人挑的。**

### 1.4 影响面（为什么这不是小 bug）

clap 是 **Rust 生态事实标准**（`clap` 4.x 每周下载量数千万），
而「描述换行」是 clap 在**描述较长时自动选择**的排版，不是特例写法。

**⇒ 未来 10 年里新增的 CLI 有极大比例是 Rust/clap 写的。
不修这条，「对未知 CLI 泛化」在 Rust 生态上直接归零。**

### 1.5 修复方向（确定性，不需 GPU）

`split_line` 之外补一条**两行合并**路径（版式 G，`split_line_multiline`）：

```
若当前行 split_line 返回 None，且
  ① 本行是 1~2 个 token 的候选词（`-n` / `-n <N_REQUESTS>`）
  ② 后续行缩进更深（> 本行）
→ 合并为「cand = 本行, desc = 后续更深缩进行的拼接」
```

**四个否决条件**（防误吞，依据 v18d 血训「附加文本会被读成命令」）：

| # | 条件 | 防的是什么 |
|---|---|---|
| A | 本行 `split_line` 已成功 | 重复处理同行描述（串味） |
| B | 本行 token 数 ∉ [1,2] | 把段落续行当成动作行 |
| C | 下一行缩进不更深 | 吞掉同级的下一个动作 |
| D | 下一行是段落标题（`Usage:`/`or:`/`Arguments:`/`Options:`/`FLAGS:`） | 把 `Arguments:` 当描述、把 `[URL]` 当命令 |

**⚠️ 接线点必须两处**（v19b 血训：只改一处 = 白改）：

1. `parse_help` 的缩进式分支 —— 且必须在**逗号列表路径之前** `continue`，
   否则 clap 的 flag 行会落进 npm 的逗号列表路径，把参数名当命令。
2. `parse_flags_as_actions` —— 这个回退**同样依赖 `split_line`**，
   不改则「子命令路径 0 条 + flag 回退 0 条」的双重失效依旧。

### 1.6 ✅ 修复实测（已实现并验证）

```
改前（7df13dc）:  oha → 解析出  0 条动作（只读  0 条）
改后            :  oha → 解析出 37 条动作（只读 37 条）
```

**零回归验证**（同一批 help 文本，改前 vs 改后逐条比对）：

| CLI | 改前 | 改后 | 变化 |
|---|---|---|---|
| docker | 57（只读 41） | 57（只读 41） | **=** |
| git | 24（只读 8） | 24（只读 8） | **=** |
| cargo | 16（只读 7） | 16（只读 7） | **=** |
| npm | 68（只读 49） | 68（只读 49） | **=** |
| kubectl | 43（只读 32） | 43（只读 32） | **=** |
| jq | 30（只读 30） | 30（只读 30） | **=** |
| **oha** | **0** | **37** | **+37** |

**⇒ 6 个池内 CLI 逐字不变，只有池外的 oha 从 0 变 37。**
这正是「补盲区」应有的形状：**只在新版式上生效，不动旧版式。**

新增 2 条单测：`clap_long_help_description_on_next_line_is_parsed`（正向）
+ `same_line_style_is_untouched_by_multiline_path`（反向，防串味）。

---

## 二、lilyco CLI：补的是**另一个**维度的盲区，而且比 oha 更关键

### 2.1 它是唯一「带机器可读注册表」的 CLI

```bash
$ lfiles --schema | jq '.[] | {name, safety, required}'
stats    required=['root']  safety=read_only
rename   required=['root']  safety=confirm      ← T1
dedup    required=['root']  safety=read_only
find     required=['root']  safety=read_only
```

**现有池 6 个 CLI 全是纯文本 help，一个机器可读字段都没有。**
lilyco 的 `--schema` 是 JSON（9844B），带 `required` / `safety` / `kind.type` / `default`。

### 2.2 🔴 它顺手解决了 G2「推断结果」的核心难题

规划文档 §4.P1-G2 的难点是：**「这条命令会输出什么」的信息在哪？**
答案是——**在 lilyco 的 `about` 里已经写好了**：

| 命令 | about 里的 Returns |
|---|---|
| `find` | `Returns { root, count, total_size, total_size_human, by_ext: {...}, files: [...] }` |
| `stats` | `Returns { root, file_count, total_size, by_ext: [...], by_dir: [...], largest: [...] }` |
| `dedup` | `Returns { root, groups: [{ size, wasted, hash, files }], wasted_bytes, scanned, ... }` |
| `rename` | `Returns { root, dry_run, matched, count, renames: [{from,to}], skipped: [...] }` |

**⇒ G2 不必「猜」，可以「读」。** 而且 `required` 字段还**直接喂给 `paramcheck`**
（现在 `paramcheck::RULES` 是**手写表**，只有 30 条；lilyco 的 schema 自带 truth）。

### 2.3 它还是唯一能端到端验证「四端一致」的目标

lilyco 的固化设计 = **一域一二进制 × 四端**（CLI / TUI / Web / MCP），
且验收标准是「**同一 handler，四端结果 JSON 逐字一致**」。
→ 可以把「模型路由」接到一个**知道自己完整能力面**的 CLI 上，
这是别的 CLI 做不到的实验（`docker --help` 不告诉你 `docker ps --json` 的 schema）。

### 2.4 ⚠️ 但要小心一个循环论证

lilyco 是**我们自己设计的**，它的 help 是**为了友好而精心写的**
（长 about、明确 Returns、`--schema`）。
→ **用 lilyco 测出的高分不能证明「对陌生 CLI 泛化」**，只能证明
「**对我们自己设计的 CLI 效果好**」。

**正确用法**：lilyco 作为**上限对照 + G2/G3 的试验田**（因为它的信息最全，
能验证「机制本身能不能工作」），**不进入泛用性分数**。
泛用性分数仍必须用**别人写的、我们没碰过的** CLI（oha 这类）。

---

## 三、建议的池子分层（这样才测得准）

| 层 | 来源 | 用途 | 是否计入泛用性分数 |
|---|---|---|---|
| **L0 现有池** | docker/git/cargo/npm/kubectl/jq | 基线对照（已知四档 A/B/C/D） | ❌（已被研究过，会污染） |
| **L1 外部陌生** | **oha** 等 GitHub 新工具 | **泛用性主分数** | ✅ |
| **L2 自研** | **lilyco 系**（lfiles/lgrep/lmpkg…） | G2/G3 试验田 + 上限对照 | ❌（作者偏差） |
| **L3 冷门/异形** | 待补：DSL 类、flag-only 类、多级子命令类 | 边界探测 | ✅ |

**⚠️ 写进流程**：报「泛用性 N%」必须说明**这批 CLI 分别属于哪层**。
只报 L1+L3 才算泛用性证据；混进 L0/L2 会让分数虚高。

**oha 已知会暴露的两件事**（建议作为 L1 的第一个用例）：
1. **clap long-help 版式** → 当前 0 条动作（§1）
2. **flag-only + 无 schema** → 与 jq 同为「能力在 flag 组合」，但 oha 的
   `-n/-c/-z` 是**数值/时长参数**（`10k`/`1m`/`10s`）→ 考参数值理解，jq 没有这一项

---

## 四、下一步（按优先级）

| 优先 | 动作 | 归属 |
|---|---|---|
| **P0** | ✅ **已修 clap long-help 解析**（oha 0 → 37 条，池内 6 个 CLI 零回归） | 已完成 |
| **P0** | 建 `L1` 外部陌生 CLI 池（oha 起头，扩到 ≥15 个：`fd`/`rg`/`hyperfine`/`dust`/`zoxide`/`starship`/`bat`/`sd`/`tokei`/`xh`… **优先挑 Rust/clap 系**） | 本机可做 |
| **P1** | 为 G2 做「Returns 抽取器」：从 help 文本抽输出形态，先拿 lilyco 验证机制可行 | 本机可做 |
| **P1** | lilyco 四端一致性 × 模型路由的端到端实验（G2/G3 试验田） | 本机可做 |
| **P2** | 评估 lilyco `--schema` 作为 `paramcheck` 真值源（替代手写 RULES） | 本机可做 |
| **P2** | oha 的 `-n/-c/-z` 是数值/时长参数（`10k`/`1m`/`10s`）→ 单独造「参数值理解」题 | 本机可做 |

---

## 五、源

- 本机实测：`oha --help`（6355B，clap 4.5 derive）、`lfiles --help`（3282B）、
  `lfiles --schema`（9844B JSON）
- 解析器：`lycore/src/help_parse.rs`（`split_line:185` / `parse_help:321` /
  `parse_flags_as_actions:823`「flag 回退同样依赖 split_line」）
- 规划文档：`docs/generalization-plan-2026-09-21.md`（§4.P1 G2/G3）
- oha：https://github.com/hatoo/oha（已 clone 到 `D:/Code/oha`）
- lilyco：`D:/Code/lilyco`（`lfiles` 已本机构建成功：`target/debug/lfiles.exe`）
