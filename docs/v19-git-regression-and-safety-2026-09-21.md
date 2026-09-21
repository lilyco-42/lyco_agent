# v19：git 回归归因（D 档）+ 一个比分数更重要的安全漏洞（2026-09-21）

> 承接 `research-v17-help-selflearning-2026-09-21.md` §5 留下的**唯一未解释项**：
> > 「④ git 的 hparse(87.5) 反而低于裸 help(100.0) —— 这是新发现，需要单独解释。」
>
> 本文档完成该归因，并顺带发现一个**比分数问题严重得多的安全漏洞**。
> 全部结论由本机离线复现得出（**零 GPU、零云端**）。

---

## 一、先说结论

| 项 | 结论 |
|---|---|
| **D 档分数回归的原因** | **不是解析错误**（解析器对 git 抽出的 23 条全对，见 §二） |
| **真因** | `readonly_only` 过滤后只剩 **11 条**，**`git remote` / `git config` 等常用查询命令从未出现在 `git --help` 里**（它们在子命令 help 中）→ 注入集覆盖不全 |
| 🔴 **新发现（严重）** | **`git pull` 被 `is_readonly` 判为「只读」→ 会绕过 T1 门直接执行** |
| ⚠️ 同类误判 | `git backfill`（下载对象）、`git history`（help 自己写 "Rewrite history"）也被判只读 |

**⇒ 优先级重排：安全漏洞修复 > D 档分数回归。**

---

## 二、假说先被自己的实验证伪（记录过程）

### 2.1 初始假说：分组标题被误当命令

git 的 help 是**分组式**：

```
start a working area (see also: git help tutorial)
   clone      Clone a repository into a new directory
   init       Create an empty Git repository or reinitialize an existing one
...
examine the history and state (see also: git help revisions)
   diff       Show changes between commits, commit and working tree, etc
```

我怀疑标题行 `examine the history and state (...)` 的首词 `examine` 全小写、长度 ≥2
→ `is_subcommand_word("examine")` = `true` → **产出垃圾命令 `git examine`**。

### 2.2 实测：假说被证伪 ✅

用真实 `git --help` 输出离线复现 `split_line` + `looks_like_subcommand`：

```
 11 [SUB ] cand='clone'     desc='Clone a repository into a new director'
 24 [SUB ] cand='log'       desc='Show commit logs'
 26 [SUB ] cand='status'    desc='Show the working tree status'
 ...
```

**分组标题行（第 10/14/20/28/39 行）根本没被 `split_line` 切出来** ——
因为它们**没有「2 个以上空格」的分隔符**，`split_line` 返回 `None`，直接跳过。

**⇒ 假说错误，解析器对 git 抽出的 23 条全部正确，无垃圾命令。**

（这正是信条 5 的价值：**先测再断言**。若直接照假说去改解析器，
会把一个本来正确的逻辑改坏。）

---

## 三、D 档真因：只读过滤后覆盖不全

### 3.1 过滤结果（本机复现 `is_readonly`）

| | 命令 |
|---|---|
| **进入只读集（11 条）** | diff / grep / log / show / status / **backfill** / branch / **history** / tag / fetch / **pull** |
| **被滤掉（13 条）** | clone / init / add / mv / restore / rm / bisect / commit / merge / rebase / reset / switch / push |

### 3.2 为什么这解释了 87.5 < 100.0

`git --help` **只列顶层命令**，不列参数子形态。而评测里问的很多是**参数级**形态：

| 评测题 | 需要 | 是否在 11 条注入集里 |
|---|---|---|
| 看看最近提交 | `git log --oneline` | ✅ log 在，但 `--oneline` 不在 |
| 有哪些改动 | `git diff --stat` | ✅ diff 在，`--stat` 不在 |
| 当前在哪个分支 | `git branch` | ✅ |
| 远端有哪些 | `git remote -v` | ❌ **remote 不在 help 顶层** |

**⇒ 裸 help（100.0%）能看到整段原文，包括 Usage 行里的参数线索；
结构化注入只给 11 条无参数的骨架 → `--oneline` / `--stat` 这类丢失。**

**这与 v17 §5 的结论完全一致**（「对 help 短且命令空间小的 CLI，裸灌可能优于结构化注入」），
本文档把它**从观察升级为归因**：**丢失的是「参数级信息」，不是「命令级信息」。**

### 3.3 修复方向（确定性，不训模型）

| 修复 | 做法 | 状态 |
|---|---|---|
| **对短 help 保留原文** | 若 `readonly_only().len() < 15` 且 help 原文短（<2KB）→ 注入原文 + 结构表**双份** | 待做 |
| 参数补齐 | `--deepen` 对 top-N 子命令逐个跑 `git <sub> --help` 拿 usage 行 | ✅ 已实现（`deepen_with_subcommand_usage`） |
| git 专用高频参数表 | `--oneline` / `--stat` / `-v` / `-a` 等，与 `DIALECT_HINTS` 同层 | 待做 |

---

## 四、🔴 比分数更严重的问题：`git pull` 被判只读

### 4.1 事实

```
git pull  →  is_readonly = true   ← 🔴 错
```

**原因**：`is_readonly` 的判定链里，`pull` 不在 `WRITE_VERBS`。
GitHub 上的 git help 里 `pull` 的描述是「Fetch from and integrate with another repository」
—— **"integrate" 是合并动作**，但判定器看的是动词名 `pull`，不在黑名单 → 落到第 5 步默认 `true`。

### 4.2 后果（为什么这是安全问题）

按产品链路：**T1 门读 `risk` 决定放行或拦截。**

```
git pull  →  判为只读（Risk::Read）  →  T1 门自动放行  →  执行
```

但 `git pull` 的实际语义 = **fetch + merge**：
- 会**改写当前工作区文件**
- 可能触发**合并冲突**，留下冲突标记文件
- 在有本地改动时会**覆盖/冲突**用户未提交的工作
- 会**访问网络**（外呼）

→ **一个会改工作区、会外呼、可能丢失用户未提交改动的命令，被自动放行了。**

这与产品已固化的原则直接冲突：
> 「删除/覆盖/外呼 ≥T1」（`lilyco` 安全策略）；
> 「执行层 ≥T1 门是唯一防线」（reject 0% 的 6 次一致证据）。

### 4.3 同类误判（一并列出）

| 命令 | 现状 | 应有 | 理由 |
|---|---|---|---|
| `git pull` | 只读 ✅ | **写** | fetch + merge，改工作区 + 外呼 |
| `git backfill` | 只读 ✅ | **写** | 下载缺失对象（写磁盘 + 外呼） |
| `git history` | 只读 ✅ | **写** | help 自己写 "EXPERIMENTAL: Rewrite history" |
| `git bisect` | 写 ❌ | 写 ✅ | 正确（判为写，虽偏保守） |
| `git fetch` | 只读 ✅ | 只读 ✅ | 正确（只下载 refs，不改工作区）|

**注**：`fetch` 判只读是**对的**（它不改工作区），这说明判定器并非「一律偏保守」，
而是**个别动词遗漏**——`pull` 是被漏掉的那个。

### 4.4 修复（最小改动）

```rust
// WRITE_VERBS 补充：pull / backfill / history 等会改工作区或外呼的动词
"pull",        // = fetch + merge，改工作区 + 外呼
"backfill",    // 下载对象
"history",     // git history = rewrite history（help 原文自述）
```

**同时建议加一条结构性断言**（防止未来再漏）：
凡描述含 `merge` / `rewrite` / `integrate` / `overwrite` 的动词，
**即使动词名不在黑名单，也应判写** —— 即**读描述文本**，而非只读动词名。

> ⚠️ 这条与 v17 §4c「术语同义词不可全自动」是同一个认识的另一面：
> **help 描述文本里含有判定所需的信息，但当前判定器完全没用它。**

---

## 五、下一步（优先级已重排）

| 优先 | 动作 | 理由 |
|---|---|---|
| **P0** | 🔴 **`git pull`/`backfill`/`history` 补进 `WRITE_VERBS`** + 加「描述含 merge/rewrite → 判写」的结构性规则 | **安全漏洞**，比分数重要；改动极小，可当天完成 |
| **P0** | 补单测：断言 `git pull` / `git backfill` / `git history` **必须**判非只读 | 防回归 |
| **P1** | **短 help 保留原文**（双份注入）：`readonly_only().len() < 15` 且原文 <2KB → 结构表 + 原文 | 直接修 D 档（87.5 → 目标 ≥90） |
| **P1** | git 高频参数表（`--oneline`/`--stat`/`-v`） | 补「参数级信息缺口」 |
| **P2** | 用 `--deepen` 跑 `git log --help` 等子命令，验证能否自动补出参数 | 若能，则不需手写参数表 |

### 5.2 修复实测（v19b 补测）

**数字勘误**：本文档 §3.1 的「解析出 23 条 / 只读 11 条」是**离线 Python 端口**的读数。
修完 P0 安全项后，用**真二进制**复测（`lycore help-parse --cli git`）为：

```
=== git: 解析出 24 条动作（只读 8 条）===
```

**只读从 11 降到 8** —— 正是本次 P0 修复的直接效果：
`backfill` / `history` / `pull` 被正确移出只读集（实测 `--include-writes` 可见它们
已归入写侧，`git history` 的描述仍是 `EXPERIMENTAL: Rewrite history`）。

**⇒ 安全修复已验证生效**（`git pull` 不再被 T1 门自动放行）。

### 5.3 ⚠️ 一个比 bug 更值得记的失误：修了实现，忘了接线

v19 我做了两件事：
1. 补 `WRITE_VERBS`（安全）—— ✅ 有效
2. 新增 `render_schema_with_raw_fallback`（修 D 档）—— ❌ **无效**

第 2 项**写出来了、也配了单测、单测还全绿**，但
`router::help_aware_schema`（`router.rs:370`）与 `lycore help-parse`（`lycore.rs:923`）
**两个真实调用点都还在调 `render_schema`**。

**后果**：生产行为与改动前**逐字一致**，而单测给我一个「修好了」的假信号。

**怎么发现的**：v19b 验证 P0.4 时顺手跑了一次真二进制：

```
$ lycore help-parse --cli git
=== git: 解析出 24 条动作（只读 8 条）===
...
只使用上面列出的命令；与上面命令无关的请求输出 (无需调用硬件命令)。
                                   ← 到这里就结束了，没有 help 原文
```

git 恰是**唯一要救的那一档**（只读 8 条 < 阈值 15，原文 2290B < 4096B）——
两个条件都满足却没追加原文 → 只有一种可能：调用点没接线。

**⇒ 纪律（加入流程）：**
> **单测证明「函数正确」，不证明「函数被调用」。**
> 凡是新增一个「替换旧行为」的函数，必须同时：
> ① 改掉**所有**调用点（`grep` 旧函数名确认零残留）；
> ② 加一条**接线断言**（从真实入口跑一次，断言新行为出现）。
>
> 本次已补 `thin_cli_schema_actually_inlines_raw_help`（router.rs）作接线回归。

### 5.4 顺带证伪 P0.4 的前提：git 子命令 help 本机不可达

原计划「用 `--deepen` 跑 `git log --help` 拿参数」—— **实测四处穷举全失败**：

| 取法 | 结果 |
|---|---|
| `git log --help` | ❌ 缺 html 文档 |
| `git help log` | ❌ 同上 |
| `man git-log` | ❌ `man: command not found` |
| `git log -h` | ⚠️ 720B，**`--oneline` 不在其中** |

**⇒ 更精确的 D 档机理（修正 §三）：**
不是「结构化注入丢了信息」，而是**「骨架注入用一个更弱的形态覆盖了模型本来正确的先验」**：

```
裸 help   → 没给 --oneline，但也不干扰 → 模型靠先验输出 `git log --oneline` → 100%
hparse    → 8 条无参数骨架把模型往「只输出 git log」推 → 丢 --oneline → 87.5%
```

这与 B 档 kubectl 是**同一机制的两个方向**：B 档是先验**赢**了注入，
D 档是注入**削弱**了先验。详细处置见 `docs/generalization-plan-2026-09-21.md` §4.1b。

---

## 六、复现（本机，零 GPU）

```bash
# 1. 取 git help 原文
git --help > /tmp/git_help.txt

# 2. 离线复现解析（Python 端口，与 Rust 逻辑逐行对应）
#    见本文档 §二/§三 的脚本；关键函数：split_line / looks_like_subcommand / is_readonly
# 3. 单测（在 CloudStudio 远端，本机不编译）
node cloudstudio/tmp/sync_build.py --bin-name lycore2 --test
```

---

## 七、源

- 原调研（D 档首次提出）：`docs/research-v17-help-selflearning-2026-09-21.md` §5 / §4f
- 解析器实现：`lycore/src/help_parse.rs`（`parse_help:316` / `is_readonly:121` /
  `looks_like_subcommand:203` / `split_line:180` / `READ_VERBS:52` / `WRITE_VERBS:63`）
- 本机复现数据：真实 `git --help`（47 行）+ Python 端口判定
