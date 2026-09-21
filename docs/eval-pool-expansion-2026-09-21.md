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

### 1.7 🔴 第二轮 / 第三轮修复：**「池子同质化」掩盖的整类缺陷**

**修完 1.6 之后，我拿 L1 池的 `gold` 去做 ground 校验**（断言「我手写的
schema 动作必须真实出现在 `help-parse` 输出里」）—— 结果 **42 处断言失败**。
**这不是数据集写错了，是解析器还有三个 bug。** 若无这层校验，
我会以为是数据集问题去改数据，bug 就留在那儿了。

#### 三个 bug 的共同病因：池内 CLI **恰好**全是「裸 flag + 无值子命令」

```
docker ps / docker logs        ← 无值子命令
git status / git log           ← 无值子命令
cargo test / cargo build       ← 无值子命令
jq --slurp / jq --compact-output ← 裸 flag（不带值）
```

**池内 6 个 CLI 的 flag 行，没有一条带值占位符。** 于是下面三个缺陷
在池内**一个都不显形**，我一跑池内回归就「全绿」，误以为解析器没问题。

| # | bug | 触发形态（池外） | 后果 |
|---|---|---|---|
| **2** | `split_line_multiline` 的判据是「空白 token 数 ≤2」 | `-w, --warmup <NUM>` = **3** 个 token | flag **组数只有 2**（`-w,` + `--warmup <NUM>`）却被误拒 → hyperfine 丢 `--warmup`/`--runs`/`--min-runs`/`--max-runs`/`--shell`/`--prepare` **6 条最高频 flag** |
| **3** | 项目符号被当动作候选 | bat 的 `* plain: disables all components.` | `*` 被当裸词，再吞掉其下更深的说明行 → 产出 `bat plain` 垃圾命令 |
| **4** | `parse_comma_list` 不判缩进 | bat 的 `            changes, grid, header-filename, numbers, snip`（**缩进 12 的值枚举**） | 与 npm 的 `All commands: a, b, c`（**缩进 0**）格式上无法区分 → 产出 `bat changes`/`bat grid`/`bat snip` **5 条垃圾命令** |
| **5** | `full_cmd` 丢弃 flag 的值占位符（`format!("{cli} {flag}")`） | `-w, --warmup <NUM>` → `hyperfine --warmup` | **示例字段随之变成 `示例：hyperfine --warmup`——跑起来报缺参**。注入给模型的每条带值 flag 都是不可执行形态 |
| **6** | 描述收集越过了下一条（缩进更深的）flag 行 | clap 缩进随有无短名而变：`  -s, --setup` **缩进 2** / `      --reference` **缩进 6** | `--setup` 的 desc 里混进 `--reference <CMD> The reference command...` → **一条动作污染三条 flag 的描述**；fd 同理错位 19 条 |

#### 修复（全部只改判据，不动旧版式）

1. **bug 2** — 判据从「空白 token 数 ≤2」改为「**flag 组数 ≤2**」：
   剥掉值占位符（`<...>` / `[...]`）后，剩下必须全是 `-` 打头的片段。
2. **bug 3** — 新增条件 B0：`*` / `•` 打头的行直接拒。
3. **bug 3'** — 裸词候选项加 `indent > 6 → None`（描述行缩进 ≥8，子命令 ≤4）。
4. **bug 4** — `parse_comma_list` 加条件 0：`indent > 6 → 空`。
5. **bug 5** — 在 `cand` 里定位 flag 后**紧随的第一个值占位符**并带上；
   同时给 `fill_example` 补一批**跨 CLI 语义无歧义**的量纲型占位符
   （`<NUM>`→10 / `<CMD>`→ls / `<FILE>`→config.json …），
   **未知语义的一律不填**（宁可返回 `None` 不给示例，不编造）。
6. **bug 6** — 新增 `is_flagish_line`，描述收集时若下一行自身是 flag 行则立即停。

#### 逐字零回归证明（不是比条数，是比内容）

用 `--include-writes --top 9999` 固定口径，**改前 vs 改后全量逐字 diff**：

| CLI | 组 | 改动前 | 改动后 | 差异 |
|---|---|---|---|---|
| docker | 池内 | — | — | **逐字 0 差异** ✅ |
| git | 池内 | — | — | **逐字 0 差异** ✅ |
| cargo | 池内 | — | — | **逐字 0 差异** ✅ |
| kubectl | 池内 | — | — | **逐字 0 差异** ✅ |
| jq | 池内 | — | — | **逐字 0 差异** ✅ |
| rg | L1 | — | — | 逐字相同 |
| zoxide | L1 | — | — | 逐字相同 |
| starship | L1 | — | — | 逐字相同 |
| **bat** | L1 | **5** | **38** | +33（修掉 5 条垃圾命令 + 恢复 33 条真 flag） |
| **hyperfine** | L1 | **14** | **28** | **翻倍**（6 条带值 flag 救回 + 2 条 desc 解污染） |
| **oha** | L1 | **37** | **42** | +5（带值 flag 补回占位符） |
| **fd** | L1 | — | — | **19 条内容修正**（desc 错位 → 正确对应） |

**池内逐字差异总数 = 0。** 这是补盲区唯一可接受的形状 ——
**若池内数字也动了，说明我在拿新判据误伤已正确的旧形态。**

新增 2 条单测（共 4 条锁 long-help 系）：
`flag_value_placeholder_is_preserved_in_full_cmd`（含「无值 flag 不得被硬塞占位符」的反向断言）
+ `deeper_indented_next_flag_is_not_swallowed_into_desc`。

#### 方法论收获（比 bug 本身更重要）

> **池内回归「全绿」不等于解析器对。它只证明「解析器对池内那类 CLI 是对的」。**

池子是「按我手上有什么样本」攒的，而攒的人一直用同一类 CLI（无值子命令 / 裸 flag）。
**这是泛用性测试的经典失效模式：测试集由被测物的作者挑选，于是系统性漏掉他不知道的那一类。**
两次（clap long-help、值占位符）都是这个模式 ——
**唯一有效的对策是「外部陌生 CLI + ground 校验」，即用户提的 L1 池。**

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

## 三点五、把 L1 池扩到 14 个 —— delta 打穿了**更深**的一层

（承接 §三：L1 池按「优先 Rust/clap 系」扩到 14 个。
新装：`dust`/`sd`/`tokei`/`xh`/`delta`/`just`/`eza`；`procs`/`bottom` 因源 504 未装上。）

### 3.5.1 表观症状 vs 真因

`delta --help` 只解析出 **6 条动作，全是垃圾**：

```
delta ancestral 'their' / delta brightblack / delta brightred / ...
```

**但真症状不是「垃圾多」，而是 delta 的 ~200 个真 flag 一条都没进来。**

连锁：`parse_help` 里 flag 回退的触发条件是 `out.is_empty()`，
而那 1 条散文垃圾让 `out` **非空** → **flag 回退整个不执行**。
**一条流沙堵死整条河。**

> 教训：**「解析出垃圾」比「解析出 0 条」更危险** —— 0 条会触发回退、
> 一眼可见；垃圾则沉默地顶替了整条通路。

### 3.5.2 根因 1：英文句末双空格 → 散文被切成「命令 + 描述」

`split_line` 的判据是「同行 2+ 空格」。而**英文排版里句末跟双空格是惯例**：
```text
          This styles the decoration of the header above the diff between the
          ancestral commit and 'their' branch.  See STYLES section. The style
```
→ `cand = "ancestral commit and 'their' branch."`、`desc = "See STYLES section..."`，
而 `is_subcommand_word("ancestral")` 为真 → 产出 `delta ancestral 'their'`。

**修**：`looks_like_subcommand` 加散文守卫 ——
① 去掉省略号后仍含**孤立句点** → 是句子；② 词数 > 4 → 是散文。

⚠️ ① **必须先抹掉 `...` / `..`**：`run [ARGS]...` 的变长参数标记不是句点。
漏这条会让 `cargo run [OPTIONS] [ARGS]...` 整条丢失
—— 被既有单测 `alias_and_arg_suffix_are_cleaned` 当场抓住。

### 3.5.3 根因 2：值枚举靠「缩进 ≤6」区分 —— 被 delta 打穿

§1.7 为 bat 加的守卫（值枚举缩进 12 vs 命令列表缩进 0）在 delta 上失效：

```text
     In addition, all of them have a bright form:
     brightblack, brightred, brightgreen, ...      ← 缩进只有 5！
```

**同一种错误的两个缩进（bat 12 / delta 5）→ 阈值法必然两头漏。**
绝对数值不是结构，**相对位置才是**。

**修**：改用**引导语**判据 —— 逗号列表必须被「含 command 词的 `:` 结尾行」背书。
delta 的 `In addition, all of them have a bright form:` 不含 command 词 → 拒。

#### ⚠️ 记一次走错的方向：区块状态机

先试过「进入散文区后禁用解析」，**被同一份 delta help 打穿**：
它的第 363 行 `Following the hyperlink spec for terminal emulators:`
位于 **flag 表中间**，把状态置真后再无节标题复位 → 剩下 ~700 行 flag 全丢。

> **help 的 flag 表与散文段落是交错的，任何区块状态机都会被这种现象骗。**
> 已回退，改回「每行独立判断 + 局部上下文（引导语）」。

### 3.5.4 最后一坑：引导语追踪写死了「回溯 8 行」

npm 的 `All commands:` 列表长达 ~13 行 → 列尾 **14 条真子命令**
（`version`/`update`/`start`/`stop`/`team`/`token`/`uninstall`/`unpublish`/
`unstar`/`search`/`set`/`shrinkwrap`/`star`）判为「无背书」丢弃，**68 → 54**。

**修**：改成粘性状态机（空行 / 列表续行保持，其余行清空）。

> **列表长度因 CLI 而异，任何固定窗口都会在某个 CLI 上过头或不足。**

### 3.5.5 验证：20 个 CLI 逐字 diff（不是比条数）

```bash
cd scripts/helpfixtures
python collect.py          # 采集 help（npm 用 node npm-cli.js 绕开缺装的 shim）
python diffcheck.py compare
```

| CLI | 改前 → 改后 | 逐字差异 |
|---|---|---|
| docker 57 / git 24 / cargo 16 / **npm 68** / kubectl 43 / jq 30 | — | **0** ✅ |
| fd 43 / rg 69 / hyperfine 28 / zoxide 6 / bat 38 / starship 14 / oha 42 | — | **0** ✅ |
| dust 41 / sd 7 / tokei 18 / xh 48 / just 67 / eza 63 | — | **0** ✅ |
| **delta** | **6（全垃圾）→ 109（全真 flag）** | 唯一变化 |

**池内逐字差异总数 = 0。** 垃圾命令数 0。

新增 4 条单测（共 28 条 help_parse），`cargo test --lib: 159 passed / 0 failed`：
`prose_paragraph_does_not_become_an_action`（含「flag 回退连锁」断言）/
`ellipsis_is_not_a_sentence` / `value_enum_without_command_intro_is_not_commands` /
`long_comma_list_tail_lines_are_still_parsed`。

### 3.5.6 L1 池现状（14 个）

| # | CLI | 星星 | 形态 | 现有解析条数 |
|---|---|---|---|---|
| 1 | fd | 13.8万 | clap long-help | 43 |
| 2 | rg | 5.1万 | clap long-help | 69 |
| 3 | hyperfine | 2.3万 | clap + **带值 flag** + T1 边界 | 28 |
| 4 | zoxide | 3.6万 | clap 短 help | 6 |
| 5 | bat | 5.4万 | clap + **值枚举陷阱** | 38 |
| 6 | starship | 5万 | clap | 14 |
| 7 | oha | 1万 | clap long-help + **外呼边界** | 42 |
| 8 | dust | 2.6万 | clap | 41 |
| 9 | sd | 8千 | clap | 7 |
| 10 | tokei | 1.4万 | clap | 18 |
| 11 | xh | 1.5万 | clap | 48 |
| 12 | **delta** | 2.4万 | **man-page 型（散文交错）** | 109 |
| 13 | just | 2.3万 | clap + 自定义子命令表 | 67 |
| 14 | eza | 4万 | clap | 63 |

---

## 四、下一步（按优先级）

| 优先 | 动作 | 归属 |
|---|---|---|
| **P0** | ✅ **已修 clap long-help 解析**（oha 0 → 37 条，池内 6 个 CLI 零回归） | 已完成 |
| **P0** | ✅ **已扩 L1 池到 14 个**（delta 打穿并修好：6 垃圾 → 109 真 flag，池内逐字零回归） | 已完成 |
| **P0** | 把 14 个 L1 CLI 的**评测题**补齐（现 `l1_pool.py` 只覆盖前 7 个共 114 条） | 本机可做 |
| **P1** | 跑 L1 评测（`_p2_l1_eval.py` 已写好，需 GPU：v13 / v15 双检查点） | CloudStudio |
| **P1** | 为 G2 做「Returns 抽取器」：从 help 文本抽输出形态，先拿 lilyco 验证机制可行 | 本机可做 |
| **P1** | lilyco 四端一致性 × 模型路由的端到端实验（G2/G3 试验田） | 本机可做 |
| **P2** | 评估 lilyco `--schema` 作为 `paramcheck` 真值源（替代手写 30 条 RULES） | 本机可做 |
| **P2** | oha 的 `-n/-c/-z` 是数值/时长参数（`10k`/`1m`/`10s`）→ 单独造「参数值理解」题 | 本机可做 |
| **P3** | 补 `procs`/`bottom`（scoop 源 504），并可加 `gitui`/`difftastic`/`difft` | 本机可做 |

---

## 五、源

- 本机实测：`oha --help`（6355B，clap 4.5 derive）、`lfiles --help`（3282B）、
  `lfiles --schema`（9844B JSON）
- 解析器：`lycore/src/help_parse.rs`（`split_line:185` / `parse_help:321` /
  `parse_flags_as_actions:823`「flag 回退同样依赖 split_line」）
- 规划文档：`docs/generalization-plan-2026-09-21.md`（§4.P1 G2/G3）
- oha：https://github.com/hatoo/oha（已 clone 到 `D:/Code/oha`）
- lilyco：`D:/Code/lilyco`（`lfiles` 已本机构建成功：`target/debug/lfiles.exe`）
