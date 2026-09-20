//! help_parse — 把 CLI 的 `--help` 原始输出**确定性地**解析成可用动作表
//!
//! ## 为什么需要它（实证依据）
//!
//! `docs/research-v17-help-selflearning-2026-09-21.md` 记录了两轮实验：
//! - **v17**（裸灌 `--help`）：base06b `help`=23.2% vs 人工 schema `hand`=100%（−76.8pp）
//! - **v17b**（让模型自己提炼 help 成动作表）：base06b 仅 +8.2pp；**v13 反而 −18.3pp**
//!
//! 根因不是 prompt，是**容量**：从非结构化散文里抽 (命令, 说明) 对属于信息抽取，
//! 0.6B 在「长上下文 + 格式约束 + 只读/写判定」三重压力下不可靠。
//! 证据：同一 prompt 下，排版规整的 git/kubectl/npm 提炼正确，
//! 而**缩进式**的 docker 掉前缀、**节标题式**的 cargo 输出 `cargo:build`。
//!
//! → **「提炼」是纯字符串处理，不该用神经网络。**
//! 本模块把这一步变成零依赖、确定性、可单测的代码；模型只做它已证擅长的
//! 「对齐已结构化动作表 + 补槽位」。
//!
//! ## 三类 help 版式（覆盖实测的全部 6 个 CLI）
//!
//! | 版式 | 实例 | 规则 |
//! |---|---|---|
//! | 缩进式 | `docker`：`  ps        列出…` | 剥前导空白 → 补 `cli ` 前缀 → 按 2+ 空格切分 |
//! | 节标题式 | `cargo`：`cargo-build  编译当前包` | `^<cli>-[a-z]` 单行 → 展开成 `cargo build` |
//! | 前缀式 | `kubectl`/`npm`/`git`/`jq` | 行首已含 `cli xxx` → 直接取用 |
//!
//! ## 只读过滤必须是规则，不是概率
//!
//! 写操作（delete/publish/apply/…）用**动词黑名单**确定性排除。
//! 这与本项目 `reject 0%` 的六组一致实证同源：**读写判定不能交给模型**。

/// 一条可调用的动作
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HelpAction {
    /// 完整命令（**必须**以 CLI 名开头 —— v17 的 F1「掉前缀」就是这条被破坏所致）
    pub full_cmd: String,
    /// 一句话说明
    pub desc: String,
    /// 是否只读（由动词黑名单确定性判定）
    pub readonly: bool,
}

/// 纯查询动词白名单 —— 命中即判为**只读**（无论其他 token）。
///
/// 白名单优先于黑名单：`cargo build` 这类「写自己 target/」不该被当危险操作，
/// 但 `docker rm` 必须是。粒度按「**是否可能损坏用户数据/影响他人**」划，
/// 而不是「是否产生写 IO」—— 这是 v17 修正后的口径。
pub const READ_VERBS: &[&str] = &[
    "get", "list", "ls", "ps", "show", "status", "logs", "log", "describe", "inspect",
    "stats", "top", "tree", "version", "info", "diff", "branch",
    "tag", "check", "test", "bench", "validate", "plan", "search", "find", "query",
    "why", "outdated", "audit", "doctor", "trace", "which", "help", "man", "docs",
];

/// 危险动作黑名单 —— 命中即判为**非只读**（可能损坏数据 / 影响他人 / 外呼）。
///
/// 只收**真危险**的动词。刻意不收 build/test 这类「写自己的产物」——
/// 它们最多浪费磁盘，不该让能力表整个空掉（v17 实测的过度惩罚）。
pub const WRITE_VERBS: &[&str] = &[
    // 破坏性 / 不可逆
    "delete", "remove", "rm", "rmi", "prune", "destroy", "terminate", "kill", "wipe",
    "reset", "revert", "drop", "truncate", "revoke", "clean", "purge", "gc",
    // 影响他人 / 外呼
    "publish", "push", "deploy", "release", "upload", "promote", "notify",
    // 改他人可见状态
    "apply", "patch", "rollout", "scale", "set", "edit", "update", "upgrade",
    "restart", "stop", "start", "attach", "exec", "uninstall",
    "commit", "merge", "rebase", "cherry-pick", "stash",
    "checkout", "switch", "restore", "mv", "clone",
    // 装 / 发布 / 授权
    "install", "add", "login", "logout", "token", "auth",
    // 建实体（会往磁盘/远端落新东西，不算「无害查询」）
    // v17c 实测：git init / cargo new / cargo init 之前被白名单模式误判为只读
    "init", "new", "create", "generate", "generate-lockfile", "vendor", "migrate",
    // 可能改写工作区状态、需要人来判断的
    "bisect", "worktree", "submodule", "hook", "note", "filter-branch",
    // 改写历史（`git filter-repo` / 旧版 `git filter-branch`）——
    // 注意 `git log` 的 `history` 是描述词不是动作，白名单里不该有 history
    "filter-repo",
];

/// 需要**组合**判定的写操作（单独出现无副作用）。
///
/// `git config user.name x`（写入）vs `git config --list`（只读）：
/// 只看 `config` 一个词无法区分 —— 前者带位置参数、后者只带 flag。
/// 因此这里按 (锚动词, 是否要求出现裸位置参数) 判定。
const WRITE_WITH_POSITIONAL: &[&str] = &["config", "alias", "remote", "branch", "tag", "hook"];

/// 需要**两个词同时出现**才判写的短语（单独出现无副作用）。
///
/// 例：`git remote`（只读列举）/ `git remote add`（写）；
/// `git tag`（只读列举）/ `git tag -d`（写）。
const WRITE_PHRASES: &[&[&str]] = &[
    &["config", "set"],
    &["config", "unset"],
    &["config", "delete"],
    &["config", "add"],
    &["submodule", "add"],
    &["remote", "add"],
    &["remote", "set-url"],
    &["branch", "delete"],
    &["tag", "delete"],
];

/// 判断一条命令是否只读。
///
/// 判定顺序：
/// 1. **短语黑名单**（`config set` 这类需组合命中的）→ 非只读
/// 2. **动词白名单** → 只读
/// 3. **动词黑名单** → 非只读
/// 4. 都没有（如 `jq . config.json` 这类无动词的）→ 默认只读（它不产生副作用）
///
/// ⚠️ 第 2 步必须先于第 3 步：`git branch` 是查分支（白），
/// 而 `git commit` 是落库（黑）—— 它们都只含一个动词，不冲突。
/// 真正冲突的是 `npm token list`（白 list + 黑 token）：此时按**主词**判，
/// 即 CLI 之后的第一个 token —— 它是 `token` → 走黑名单路径。
pub fn is_readonly(cmd: &str) -> bool {
    // 只看 CLI 名之后的部分 —— CLI 名本身（如 `npm`）不参与判定
    let mut it = cmd.split_whitespace();
    let _cli = it.next();
    let mut toks: Vec<String> = Vec::new();
    for tok in it {
        // 去掉前导 `-`（flag 形式）与 `<`/`>`（占位符），再按 `-`/`_` 切子词
        let t = tok
            .trim_start_matches('-')
            .trim_matches(|c| c == '<' || c == '>' || c == '"' || c == '\'');
        for part in t.split(['-', '_']) {
            if !part.is_empty() {
                toks.push(part.to_ascii_lowercase());
            }
        }
    }
    // 1. 短语黑名单（同时出现才判写）
    if WRITE_PHRASES
        .iter()
        .any(|ph| ph.iter().all(|w| toks.iter().any(|t| t == w)))
    {
        return false;
    }
    // 1b. 锚动词 + 裸位置参数 → 写。
    //     例：`git config user.name x`（写）vs `git config --list`（读）。
    //     位置参数 = 既非 flag（不以 `-` 开头）也非取值（不含 `=` / 不在锚词后紧跟）
    let has_bare_positional = cmd
        .split_whitespace()
        .skip(2) // 跳过 CLI 名与锚动词本身
        .any(|t| !t.starts_with('-') && !t.contains('=') && !t.starts_with('[') && !t.starts_with('<'));
    if has_bare_positional
        && toks.iter().any(|t| WRITE_WITH_POSITIONAL.contains(&t.as_str()))
    {
        return false;
    }
    // 2. 主词（CLI 后第一个 token）若在黑名单 → 直接判写。
    //    这条挡住 `npm token list`（白名单的 list 不该救回一个 token 管理命令）
    if let Some(main) = toks.first() {
        if WRITE_VERBS.contains(&main.as_str()) {
            return false;
        }
    }
    // 3. 白名单
    if toks.iter().any(|t| READ_VERBS.contains(&t.as_str())) {
        return true;
    }
    // 4. 黑名单
    if toks.iter().any(|t| WRITE_VERBS.contains(&t.as_str())) {
        return false;
    }
    true
}

/// 把一行拆成 (候选命令片段, 说明)。
///
/// 分隔策略（按优先级）：
/// 1. 2 个以上空格（三类 help 版式都用它做列对齐）
/// 2. 制表符
/// 3. 单个空格 + 后段含大写/中文（说明文字的典型特征）
fn split_line(line: &str) -> Option<(String, String)> {
    let t = line.trim_start();
    if t.is_empty() {
        return None;
    }
    if let Some(i) = t.find("  ") {
        let (a, b) = t.split_at(i);
        let desc = b.trim();
        if !desc.is_empty() {
            return Some((a.trim().to_string(), desc.to_string()));
        }
    }
    if let Some(i) = t.find('\t') {
        let (a, b) = t.split_at(i);
        let desc = b.trim();
        if !desc.is_empty() {
            return Some((a.trim().to_string(), desc.to_string()));
        }
    }
    None
}

/// 判断一个候选片段是否像「子命令」而非「flag 列表 / 段落标题」
fn looks_like_subcommand(cli: &str, cand: &str) -> bool {
    let c = cand.trim();
    if c.is_empty() {
        return false;
    }
    // 纯 flag（-x / --long）不算子命令
    if c.starts_with('-') {
        return false;
    }
    // 形如 `cli-sub` 的节标题（cargo 版式）在调用侧另行处理
    let first = c.split_whitespace().next().unwrap_or("");
    if first.is_empty() {
        return false;
    }
    // 若已带 cli 前缀，取第二个 token 作为子命令
    let sub = if first == cli {
        c.split_whitespace().nth(1).unwrap_or("")
    } else {
        first
    };
    is_subcommand_word(sub)
}

/// 规范化别名叫法：`build, b` → `build`；`get, g` → `get`。
///
/// cargo/git 的 help 里动作行形如 `build, b    Compile the current package`，
/// 直接取用会产出 `cargo build, b` 这种不可执行命令。
fn normalize_alias(cand: &str) -> String {
    match split_cmd_and_args(cand) {
        Some((sub, args)) if args.is_empty() => sub,
        Some((sub, args)) => format!("{sub} {args}"),
        None => cand.trim().to_string(),
    }
}

/// 剥离 ANSI 转义序列。
///
/// **必需**：`cargo --help` 实测输出带完整颜色码（`\x1b[92m` 等），
/// 不剥离会把命令名淹在转义序列里 → 解析全废（v17b 实测 cargo 全臂 0% 的隐藏原因）。
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // ESC [ ... 终止字符（@ 到 ~ 范围）
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// 判断 token 是否是**子命令名**（用作「别名 vs 参数」的鉴别器）。
///
/// - 子命令名由 `[a-z0-9-]` 组成，**以字母开头**（排除 `-m`/`--state` 这类 flag，
///   也排除 `2` 这类计数）
/// - 长度 ≥ 2（排除单字母别名 `b`/`c`/`r`）
/// - 全小写（排除 `None`/`Se` 这类被空格对齐切出来的散文碎片）
fn is_subcommand_word(w: &str) -> bool {
    let w = w.trim_end_matches(',');
    w.len() >= 2
        && w.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && w.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// 把一行 help 的「候选命令片段」解析成**真实子命令 + 需要保留的参数**。
///
/// 解决 v17c 实测的两类残留：
/// - `build, b` → 子命令 `build`（别名丢弃）
/// - `run [OPTIONS] [ARGS]...` → 子命令 `run` + 参数 `[OPTIONS] [ARGS]...`
///
/// 判据：第一个 token 是子命令名；紧跟的 token 只要**不是子命令风格**
/// （以 `-`/`[`/`<`/大写开头，或含空格），就当作必须保留的语法参数。
/// 注意单字母 token 既不是子命令也不是参数（cargo 的 `run, r`），直接跳过。
fn split_cmd_and_args(cand: &str) -> Option<(String, String)> {
    let toks: Vec<&str> = cand.split_whitespace().collect();
    let head = *toks.first()?;
    if !is_subcommand_word(head) {
        return None;
    }
    let sub = head.trim_end_matches(',').to_string();
    let mut args: Vec<&str> = Vec::new();
    for t in &toks[1..] {
        let bare = t.trim_end_matches(',');
        if is_subcommand_word(bare) {
            // 第二个子命令风格 token（cargo 的 `run, r` 里的 `r` 已因长度被排除；
            // 若真出现则是散文被对齐切出来，停止收参数即可）
            continue;
        }
        // 单字母别名残留同上，跳过
        let is_flag = t.starts_with('-') || t.starts_with('[') || t.starts_with('<');
        if is_flag || bare.chars().next().is_some_and(|c| !c.is_ascii_lowercase()) {
            args.push(t);
        }
    }
    let args = args.join(" ");
    Some((sub, args))
}

/// 解析 `--help` 原始输出 → 动作表。
///
/// - `cli`：CLI 名（用于补前缀与识别节标题）
/// - `raw`：`cmd --help` 的原始 stdout+stderr
/// - 返回顺序即 help 出现顺序（调用侧可自行截断 Top-N）
pub fn parse_help(cli: &str, raw: &str) -> Vec<HelpAction> {
    let mut out: Vec<HelpAction> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let section_prefix = format!("{cli}-");

    // 0. **必须先剥 ANSI** —— cargo 带颜色码，不剥则命令名被转义序列污染
    let clean = strip_ansi(raw);

    for line in clean.lines() {
        // ── 版式 B：节标题式（cargo）：`cargo-build        编译当前包` ──
        let trimmed = line.trim_start();
        if trimmed.starts_with(&section_prefix) {
            if let Some((cand, desc)) = split_line(trimmed) {
                // `cargo-build` → `cargo build`
                let first = cand.split_whitespace().next().unwrap_or("");
                if let Some(rest) = first.strip_prefix(&section_prefix) {
                    let inner = cand[first.len()..].trim();
                    let cmd = format!("{cli} {}", rest.replace('-', " "));
                    let cmd = if inner.is_empty() {
                        cmd
                    } else {
                        format!("{cmd} {inner}")
                    };
                    push_unique(&mut out, &mut seen, cli, cmd, desc);
                    continue;
                }
            }
        }

        // ── 版式 A/C：缩进式与前缀式 ──
        let Some((cand, desc)) = split_line(line) else {
            // ── 版式 E：逗号列表（npm 的 `All commands:` 段靠这个捕获）──
            for a in parse_comma_list(cli, line, &mut seen) {
                out.push(a);
            }
            continue;
        };
        if !looks_like_subcommand(cli, &cand) {
            continue;
        }
        // 补前缀：v17 的 F1「掉前缀」根因就在这里。
        //
        // ⚠️ 关键顺序：`normalize_alias` **只能用在没带前缀的行上**。
        // 前缀式（`kubectl get ...`）若先过 normalize_alias，
        // 它会把 `kubectl get` 当别名叫法压成 `kubectl` —— v17c 实测回归。
        let first = cand.split_whitespace().next().unwrap_or("");
        let full = if first == cli {
            // 已带前缀：剥掉前缀后重新解析「子命令 + 语法参数」
            let body = cand[first.len()..].trim();
            if body.is_empty() {
                continue;
            }
            match split_cmd_and_args(body) {
                Some((sub, args)) if args.is_empty() => format!("{cli} {sub}"),
                Some((sub, args)) => format!("{cli} {sub} {args}"),
                None => continue,
            }
        } else {
            // 缩进式（docker）：无前缀、可能有别名叫法 → 先规范化再补前缀
            format!("{cli} {}", normalize_alias(&cand))
        };
        push_unique(&mut out, &mut seen, cli, full, desc);
    }

    // ── 版式 D：无子命令 CLI（jq 类）—— help 里只有 flag 列表 ──
    // jq/curl 这类工具的「能力」就是 flag 组合，没有 `cli sub` 形态。
    // 不做这层回退时 jq 解析数为 0（v17 实测），整条 CLI 直接不可用。
    if out.is_empty() {
        out = parse_flags_as_actions(cli, raw);
    }
    out
}

/// 版式 E：逗号列表 —— `npm --help` 的 `All commands:` 段。
///
/// 形如（跨多行、逗号分隔、无说明）：
/// ```text
/// All commands:
///
///     access, adduser, audit, bugs, cache, ci, completion,
///     config, dedupe, deprecate, diff, dist-tag, docs, doctor,
/// ```
/// 必须支持，否则 npm 这类 CLI 只能解析出 help 顶部那几行语法示例
/// （v17c 实测：npm `--help` 在 Linux 上仍能解析，但动作表严重残缺）。
///
/// 约束（防止把散文逗号句当成命令表）：
/// - 行内**没有**说明列（split_line 已失败）
/// - 行内 token 全部是 `[a-z0-9-]` 且≥2 字符，或纯逗号
/// - 至少 3 个 token（单个逗号句不算）
fn parse_comma_list(
    cli: &str,
    line: &str,
    seen: &mut std::collections::HashSet<String>,
) -> Vec<HelpAction> {
    let t = line.trim();
    // 必须以逗号结尾（列表续行）或含 ≥2 个逗号
    let n_comma = t.matches(',').count();
    if n_comma == 0 || (!t.ends_with(',') && n_comma < 2) {
        return Vec::new();
    }
    let toks: Vec<&str> = t
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if toks.len() < 3 {
        return Vec::new();
    }
    // 每个 token 都必须是合法子命令名（无说明文本 → 不会污染）
    if !toks.iter().all(|w| is_subcommand_word(w)) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for w in toks {
        let cmd = format!("{cli} {w}");
        let key = cmd.clone();
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        let ro = is_readonly(&cmd);
        out.push(HelpAction {
            full_cmd: cmd,
            desc: String::new(), // 逗号列表不提供说明
            readonly: ro,
        });
    }
    out
}

/// 无子命令 CLI 的回退解析：把「有说明文本的 flag 行」抽成动作。
fn parse_flags_as_actions(cli: &str, raw: &str) -> Vec<HelpAction> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in raw.lines() {
        let Some((cand, desc)) = split_line(line) else {
            continue;
        };
        // 找到形如 `--long-name` 的长选项
        let long = cand
            .split(|c: char| c == ',' || c == ' ')
            .map(str::trim)
            .find(|s| s.starts_with("--") && s.len() > 3);
        let Some(flag) = long else { continue };
        // 长选项名必须是纯小写字母/数字/连字符
        let name = flag.trim_start_matches('-');
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            continue;
        }
        let cmd = format!("{cli} {flag}");
        push_unique(&mut out, &mut seen, cli, cmd, desc);
    }
    out
}

fn push_unique(
    out: &mut Vec<HelpAction>,
    seen: &mut std::collections::HashSet<String>,
    _cli: &str,
    cmd: String,
    desc: String,
) {
    // 归一化后再做 key：否则 `kubectl get` 与 `kubectl get pods`
    // 会被判为不同串而同时入库（v17c 回归）
    let key = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
    if seen.contains(&key) {
        return;
    }
    seen.insert(key);
    let ro = is_readonly(&cmd);
    out.push(HelpAction {
        full_cmd: cmd,
        desc,
        readonly: ro,
    });
}

/// 只保留只读动作（T1 门前置：能力表只暴露只读面）
pub fn readonly_only(actions: &[HelpAction]) -> Vec<HelpAction> {
    actions.iter().filter(|a| a.readonly).cloned().collect()
}

/// 高频动作优先排序 —— help 里的顺序是**字母序/分组序，不是使用频率**。
///
/// v17 实测：`docker --help` 前 6 条只读是 ps/bake/pull/images/search/version，
/// 把真正常用的 logs/inspect/stats 挤出了 top-N。
/// 这里用一份跨 CLI 通用的**高频动作表**把常用项提到前面，提升注入命中率。
const COMMON_ACTIONS: &[&str] = &[
    "status", "ps", "list", "ls", "get", "logs", "log", "show", "inspect",
    "stats", "info", "images", "version", "describe", "tree", "branch",
    "diff", "history", "test", "check", "build", "plan", "config", "top",
];

/// 按常用度重排（稳定排序，不改变同分项的相对顺序）
pub fn rank_by_frequency(actions: &[HelpAction]) -> Vec<HelpAction> {
    let mut idx: Vec<usize> = (0..actions.len()).collect();
    idx.sort_by_key(|&i| {
        let cmd = &actions[i].full_cmd;
        let best = COMMON_ACTIONS
            .iter()
            .position(|w| cmd.split_whitespace().any(|t| t.eq_ignore_ascii_case(w)));
        best.unwrap_or(usize::MAX)  // 未命中排最后
    });
    idx.into_iter().map(|i| actions[i].clone()).collect()
}

/// 渲染成注入给模型的 schema 块 —— 形态与人工 schema 一致（这是 v17b 证明的关键：
/// 前缀带对 + 形态紧凑 → 分数就上）。
pub fn render_schema(cli: &str, actions: &[HelpAction], max_n: usize) -> String {
    let mut s = format!("\n\n当前可用命令（{cli} 域，只读）：\n");
    for a in actions.iter().take(max_n) {
        s.push_str(&format!("- {} ：{}\n", a.full_cmd, a.desc));
    }
    s.push_str(&format!(
        "只使用上面列出的命令；与上面命令无关的请求输出 (无需调用硬件命令)。"
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 缩进式（docker）：**必须补回 cli 前缀** —— 这是 v17 F1 的直接修复
    #[test]
    fn docker_indented_gets_prefix_restored() {
        let raw = "\
Usage:  docker [OPTIONS] COMMAND

Commands:
  run         Create and run a new container from an image
  ps          List containers
  images      List images
  logs        Fetch the logs of a container
  inspect     Return low-level information on Docker objects
  stats       Display a live stream of container(s) resource usage statistics
";
        let acts = parse_help("docker", raw);
        let cmds: Vec<&str> = acts.iter().map(|a| a.full_cmd.as_str()).collect();
        assert!(cmds.contains(&"docker ps"), "掉前缀回归! got={cmds:?}");
        assert!(cmds.contains(&"docker images"));
        assert!(cmds.contains(&"docker logs"));
        assert!(cmds.contains(&"docker inspect"));
        assert!(cmds.contains(&"docker stats"));
        // 所有命令都必须以 cli 名开头
        for c in &cmds {
            assert!(c.starts_with("docker "), "缺前缀: {c}");
        }
    }

    /// 节标题式（cargo）：`cargo-build` → `cargo build`
    #[test]
    fn cargo_section_headers_expand_to_subcommands() {
        let raw = "\
Rust's package manager

Usage: cargo [+toolchain] [OPTIONS] [COMMAND]

Commands:
  cargo-build         Compile the current package
  cargo-check         Analyze the current package and report errors
  cargo-test          Execute unit and integration tests
  cargo-tree          Display a tree visualization of a dependency graph
  cargo-clean         Remove generated artifacts
  cargo-add           Add dependencies to a Cargo.toml manifest file
";
        let acts = parse_help("cargo", raw);
        let cmds: Vec<&str> = acts.iter().map(|a| a.full_cmd.as_str()).collect();
        assert!(cmds.contains(&"cargo build"), "cargo 节标题未展开: {cmds:?}");
        assert!(cmds.contains(&"cargo check"));
        assert!(cmds.contains(&"cargo test"));
        assert!(cmds.contains(&"cargo tree"));
        // 不允许多余的连字符残留
        for c in &cmds {
            assert!(!c.contains("cargo-"), "残留连字符: {c}");
        }
    }

    /// 前缀式（kubectl/git/npm）：已有前缀 → 不重复补
    #[test]
    fn prefixed_style_not_double_prefixed() {
        let raw = "\
kubectl controls the Kubernetes cluster manager.

  kubectl get          Display one or many resources
  kubectl describe     Show details of a specific resource
  kubectl logs         Print the logs for a container in a pod
";
        let acts = parse_help("kubectl", raw);
        for a in &acts {
            assert!(
                !a.full_cmd.starts_with("kubectl kubectl"),
                "双前缀: {}",
                a.full_cmd
            );
        }
        let cmds: Vec<&str> = acts.iter().map(|a| a.full_cmd.as_str()).collect();
        assert!(cmds.contains(&"kubectl get"));
        assert!(cmds.contains(&"kubectl logs"));
    }

    /// 只读过滤：写操作必须被标为非只读（T1 门的地基，不能靠模型判）
    ///
    /// 口径（v17 修正后）：按「**是否可能损坏用户数据/影响他人**」划，而非「是否产生写 IO」。
    /// → `cargo build`（写自己 target/）算只读；`docker rm`（删用户数据）不算。
    #[test]
    fn write_verbs_are_marked_not_readonly() {
        for (cmd, want_ro) in [
            // 纯读
            ("docker ps", true),
            ("docker images", true),
            ("docker inspect web", true),
            ("docker stats", true),
            ("docker logs web", true),
            ("npm outdated", true),
            ("git status", true),
            ("git log", true),
            ("git branch", true),
            ("terraform plan", true),
            ("kubectl get pods", true),
            ("cargo check", true),
            ("cargo build", true),   // 写自己产物 → 不危险 → 只读
            ("cargo test", true),    // 同上
            ("cargo tree", true),    // 纯查询
            ("jq . config.json", true),
            // 危险 / 影响他人
            ("docker rm redis", false),
            ("docker rmi web", false),
            ("docker prune", false),
            ("npm install express", false),
            ("npm publish", false),
            ("npm uninstall x", false),
            ("git commit -m x", false),
            ("git push", false),
            ("git reset --hard", false),
            ("git checkout main", false),
            ("terraform apply", false),
            ("terraform destroy", false),
            ("kubectl delete pod x", false),
            ("kubectl scale deployment f -f", false),
            ("kubectl apply -f x.yaml", false),
        ] {
            let got = is_readonly(cmd);
            assert_eq!(got, want_ro, "is_readonly({cmd}) 应为 {want_ro}, 实得 {got}");
        }
    }

    /// 无子命令 CLI（jq 类）：flag 回退必须生效，否则整条 CLI 不可用
    #[test]
    fn flag_only_cli_falls_back_to_flags() {
        let raw = "\
jq - commandline JSON processor [version 1.7]

Usage:  jq [options...] filter [files...]

  -c, --compact-output   compact instead of pretty-printed output
  -r, --raw-output       output strings without escapes and quotes
  -s, --slurp            read all inputs into an array
  -n, --null-input       use null as the single input value
  -S, --sort-keys        sort keys of each object on output
";
        let acts = parse_help("jq", raw);
        assert!(!acts.is_empty(), "jq 类 CLI 解析为空 → 能力表整个不可用");
        let cmds: Vec<&str> = acts.iter().map(|a| a.full_cmd.as_str()).collect();
        assert!(cmds.contains(&"jq --raw-output"), "got={cmds:?}");
        assert!(cmds.contains(&"jq --slurp"), "got={cmds:?}");
        for c in &cmds {
            assert!(c.starts_with("jq "), "缺前缀: {c}");
        }
    }

    /// 常用度重排：常用的 logs/stats 必须排在 bake/pull 之前
    #[test]
    fn frequency_ranking_promotes_common_actions() {
        let raw = "\
Commands:
  bake        Build from a file
  pull        Download an image from a registry
  ps          List containers
  images      List images
  logs        Fetch the logs of a container
  stats       Display resource usage statistics
  version     Show the Docker version
";
        let acts = readonly_only(&parse_help("docker", raw));
        let ranked = rank_by_frequency(&acts);
        let cmds: Vec<&str> = ranked.iter().map(|a| a.full_cmd.as_str()).collect();
        let pos = |c: &str| cmds.iter().position(|x| *x == c).unwrap();
        assert!(
            pos("docker logs") < pos("docker bake"),
            "常用项未提前: {cmds:?}"
        );
        assert!(
            pos("docker stats") < pos("docker version"),
            "常用项未提前: {cmds:?}"
        );
        assert_eq!(ranked.len(), acts.len(), "重排不得增删元素");
    }

    /// readonly_only 过滤后不留写操作
    #[test]
    fn readonly_only_drops_writes() {
        let raw = "\
Commands:
  ps          List containers
  rm          Remove one or more containers
  stats       Display a live stream of resource usage
  kill        Kill one or more running containers
";
        let acts = parse_help("docker", raw);
        let ro = readonly_only(&acts);
        let cmds: Vec<&str> = ro.iter().map(|a| a.full_cmd.as_str()).collect();
        assert!(cmds.contains(&"docker ps"));
        assert!(cmds.contains(&"docker stats"));
        assert!(!cmds.contains(&"docker rm"), "rm 不该进只读集: {cmds:?}");
        assert!(!cmds.contains(&"docker kill"), "kill 不该进只读集: {cmds:?}");
    }

    /// 渲染出的 schema 必须带 cli 前缀（v17b 证明这是分数成败的关键）
    #[test]
    fn rendered_schema_keeps_prefix_and_caps_count() {
        let raw = "\
Commands:
  ps          List containers
  images      List images
  logs        Fetch the logs
  inspect     Return information
  stats       Display resource usage
  version     Show the version
  info        Display system-wide information
";
        let acts = readonly_only(&parse_help("docker", raw));
        let s = render_schema("docker", &acts, 5);
        assert!(s.contains("- docker ps ："), "schema 缺前缀:\n{s}");
        // 只输出 max_n 条
        let n = s.lines().filter(|l| l.starts_with("- docker ")).count();
        assert!(n <= 5, "超过 max_n: {n}");
    }

    /// Usage 行本身不该被当成动作（防噪声）
    #[test]
    fn usage_line_is_not_an_action() {
        let raw = "\
Usage:  docker [OPTIONS] COMMAND

Commands:
  ps          List containers
";
        let acts = parse_help("docker", raw);
        let cmds: Vec<&str> = acts.iter().map(|a| a.full_cmd.as_str()).collect();
        assert!(
            cmds.iter().all(|c| !c.contains("[OPTIONS]")),
            "Usage 行混进动作表: {cmds:?}"
        );
    }

    /// 去重：同一命令重复出现只保留一条
    #[test]
    fn duplicate_commands_deduped() {
        let raw = "\
Commands:
  ps          List containers
  ps          List containers (dup)
";
        let acts = parse_help("docker", raw);
        assert_eq!(
            acts.iter().filter(|a| a.full_cmd == "docker ps").count(),
            1,
            "未去重: {acts:?}"
        );
    }


    /// 版式 E：逗号列表（npm 的 `All commands:` 段）
    #[test]
    fn comma_list_layout_is_parsed() {
        let raw = "\
npm <command>

Usage:

npm install        install all the dependencies in your project
npm test           run this project's tests

All commands:

    access, adduser, audit, bugs, cache, ci, completion,
    config, dedupe, deprecate, diff, dist-tag, docs,
    ls, org, outdated, pack, ping, query, search, view
";
        let acts = parse_help("npm", raw);
        let cmds: Vec<&str> = acts.iter().map(|a| a.full_cmd.as_str()).collect();
        assert!(cmds.contains(&"npm ls"), "逗号列表未解析: {cmds:?}");
        assert!(cmds.contains(&"npm view"), "逗号列表未解析: {cmds:?}");
        assert!(cmds.contains(&"npm search"));
        assert!(cmds.contains(&"npm query"));
        for c in &cmds {
            assert!(c.starts_with("npm "), "缺前缀: {c}");
        }

        // 反向：普通散文行含多个逗号不得被误当命令表
        let prose = "\
npm is a package manager for JavaScript, it has a CLI, and it is fast.
";
        let acts2 = parse_help("npm", prose);
        assert!(
            acts2.is_empty(),
            "散文被误判成命令表: {:?}",
            acts2.iter().map(|a| &a.full_cmd).collect::<Vec<_>>()
        );
    }

    /// 别名 + 参数残留（v17c 实测）：`build, b` / `run [OPTIONS] [ARGS]...`
    #[test]
    fn alias_and_arg_suffix_are_cleaned() {
        let raw = "\
Commands:
  build, b    Compile the current package
  run, r      Run a binary or example of the local package
  test, t     Run the tests
  remove      Remove a package
";
        let acts = parse_help("cargo", raw);
        let cmds: Vec<&str> = acts.iter().map(|a| a.full_cmd.as_str()).collect();
        assert!(cmds.contains(&"cargo build"), "别名残留: {cmds:?}");
        assert!(cmds.contains(&"cargo run"), "别名残留: {cmds:?}");
        assert!(cmds.contains(&"cargo test"));
        for c in &cmds {
            assert!(!c.contains(", b"), "别名未剥: {c}");
            assert!(!c.contains(", r"), "别名未剥: {c}");
            assert!(!c.contains(", t"), "别名未剥: {c}");
        }

        // 带语法参数的行：子命令保留、参数也必须保留（去掉会变成静默失效的空参调用）
        let raw2 = "\
Commands:
  run [OPTIONS] [ARGS]...   Run a binary
  add [OPTIONS] <DEP>...    Add dependencies
";
        let acts2 = parse_help("cargo", raw2);
        let cmds2: Vec<&str> = acts2.iter().map(|a| a.full_cmd.as_str()).collect();
        assert!(
            cmds2.contains(&"cargo run [OPTIONS] [ARGS]..."),
            "参数被吞: {cmds2:?}"
        );
        assert!(
            cmds2.iter().any(|c| c.starts_with("cargo add")),
            "add 丢失: {cmds2:?}"
        );
    }

    /// 建实体类动词必须判写（v17c 实测：git init / cargo new 曾被误判只读）
    #[test]
    fn entity_creating_verbs_are_writes() {
        for cmd in [
            "git init",
            "cargo new myapp",
            "cargo init",
            "npm init",
            "git bisect start",
            "git submodule add x",
            "kubectl create -f x.yaml",
            "docker create nginx",
            "git config user.name x",
        ] {
            assert!(!is_readonly(cmd), "{cmd} 应判为写操作");
        }
        // 但纯查询不能被误伤
        for cmd in [
            "git status",
            "git log --oneline",
            "git config --list",
            "cargo build",
            "cargo check",
            "docker ps",
            "kubectl get pods",
        ] {
            assert!(is_readonly(cmd), "{cmd} 应判为只读");
        }
    }
}
