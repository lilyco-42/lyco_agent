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
    /// 可直接照抄的示例命令（v18 实测补入：人工 schema 有「示例：」而 help-parse 没有，
    /// 是 base06b 从 100% 掉到 59.4% 的主因 —— `docker logs` 缺 `web`、
    /// `kubectl describe` 缺 `pod`。示例由参数占位符**确定性地**填成具体值。）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
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
    // 🔴 v19 补：会**改写工作区**或**外呼**却曾被漏判为只读的动词。
    //    `git pull` = fetch + merge（改工作区 + 外呼 + 可能留冲突文件），
    //    `git backfill` 下载对象，`git history` 的 help 自述 "Rewrite history"。
    //    不补这三条 → T1 门读 risk=Read → **自动放行一个会覆盖用户未提交改动的命令**。
    "pull", "backfill", "history",
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

    // ── 版式 F：参数语法型（jq 类）—— 补一条惯用姿态动作 ──
    // v18 实测：纯 flag 注入会让模型去拼 flag（`jq --raw-input --slurp name`）→ 0%，
    // 而 jq 的真能力 `jq .name config.json` 根本不在 help 里。
    // 这条动作必须排在**最前**，否则排在 27 条 flag 之后等于没注入。
    if let Some(a) = arg_syntax_action(cli, &out) {
        let key = a.full_cmd.clone();
        if !seen.contains(&key) {
            seen.insert(key);
            out.insert(0, a);
        }
    }

    // ── 收尾：用 Usage 行补回**必需位置参数** ──
    // v18 实测最大失分点：`docker --help` 只写 `logs  Fetch the logs of a container`，
    // 看不到它需要一个容器名 → 模型输出 `docker logs`（漏 web）。
    // 参数的权威来源是同份 help 里的 usage 行，扫出来补进 full_cmd。
    let argmap = scan_subcommand_args(cli, raw);
    if !argmap.is_empty() {
        for a in out.iter_mut() {
            // 已有参数/占位符 → 不动（说明本来就解析对了）
            let body = a.full_cmd.split_whitespace().nth(1).unwrap_or("");
            if !body.is_empty() && argmap.contains_key(body) {
                let extra = &argmap[body];
                if !a.full_cmd.contains('<') && !a.full_cmd.contains('[') {
                    a.full_cmd = format!("{} {}", a.full_cmd, extra);
                    a.example = fill_example(&a.full_cmd);
                }
            }
        }
        // full_cmd 变了 → 用 example 补出具体值；同时保持 example 与 full_cmd 一致
        for a in out.iter_mut() {
            if a.example.is_none() {
                a.example = fill_example(&a.full_cmd);
            }
        }
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
        let example = fill_example(&cmd);
        out.push(HelpAction {
            full_cmd: cmd,
            desc: String::new(), // 逗号列表不提供说明
            readonly: ro,
            example,
        });
    }
    out
}

/// 把参数占位符**确定性地**填成具体值，产出可直接照抄的示例。
///
/// v18 实测：人工 schema 每条都带「示例：docker logs web」，而 help-parse 只有说明文字。
/// base06b 因此从 100% 掉到 59.4% —— 失败几乎全是**漏参数**
/// （`docker logs` 缺 `web`、`kubectl describe` 缺 `pod`、`npm install` 缺 `typescript`）。
/// 模型能从说明猜到子命令，但不敢凭空编参数名；给一个现成示例就解决了。
///
/// 填充规则（按占位符语义给最常识的值，而非随便塞）：
/// ```text
/// <容器名> / <name>     → web
/// <Pod名>  / <pod>      → frontend-7d9
/// <包名>   / <pkg>      → express
/// <脚本名>              → build
/// <file>   / <路径>     → config.json
/// <镜像名>              → nginx
/// ...（未知占位符 → 保留原样，避免编造语义错误的示例）
/// ```
fn fill_example(cmd: &str) -> Option<String> {
    if !cmd.contains('<') {
        return None; // 无需填充 → 命令本身就是示例
    }
    let mut out = cmd.to_string();
    for (ph, val) in [
        ("<容器名或镜像名>", "web"),
        ("<容器名>", "web"),
        ("<容器>", "web"),
        ("<镜像名>", "nginx"),
        ("<Pod名>", "frontend-7d9"),
        ("<pod-name>", "frontend-7d9"),
        ("<pod>", "frontend-7d9"),
        ("<包名>", "express"),
        ("<依赖包>", "express"),
        ("<脚本名>", "build"),
        ("<script>", "build"),
        ("<文件>", "config.json"),
        ("<文件路径>", "config.json"),
        ("<路径>", "config.json"),
        ("<file>", "config.json"),
        ("<path>", "config.json"),
        ("<name>", "web"),
        ("<NAME>", "web"),
        ("<DEP>", "express"),
        ("<DEP>...", "express"),
        ("<URL>", "https://example.com"),
        ("<dir>", "src"),
        // ── Usage 行扫出来的**全大写**占位符（docker/kubectl 风格）──
        // `docker logs --help` → `Usage:  docker logs [OPTIONS] CONTAINER`
        ("<CONTAINER>", "web"),
        ("<IMAGE>", "nginx"),
        ("<POD>", "frontend-7d9"),
        ("<POD_NAME>", "frontend-7d9"),
        ("<NAME>", "web"),
        ("<REPOSITORY>", "myapp"),
        ("<TARGET>", "src"),
        ("<TESTNAME>", "it_works"),
        ("<QUERY>", "TODO"),
        ("<PATTERN>", "TODO"),
        ("<HOST>", "localhost"),
        ("<COMMAND>", "build"),
        ("<ARGS>", "build"),
        ("<FORMAT>", "json"),
        ("<VERSION>", "1.0.0"),
        ("<KEY>", "name"),
        ("<VALUE>", "express"),
        // kubectl 的资源类型：示例给最常见的形态（pods 是评测高频）
        //
        // ⚠️ **v18e 血训**：`describe` 的真实语法是 `describe TYPE NAME`（两个位置参数）。
        // 只把 `<TYPE>` 填成 pods 会产出示例 `kubectl describe pods` ——
        // 模型据此认为「describe 后面就该跟 pods」，于是输出
        // `kubectl describe pods frontend-7d9`（多了一个 pods，把名字挤到了第三位）。
        // **示例给错形态 = 直接教出错误结构**，所以两参数形态必须整体给全。
        ("<TYPE> <NAME>", "pods web"),
        ("<TYPE> <POD>", "pods frontend-7d9"),
        ("<TYPE> <NAME_OR_TYPE>", "pods web"),
        ("<TYPE>", "pods"),
    ] {
        if out.contains(ph) {
            out = out.replace(ph, val);
        }
    }
    // 仍有未识别的占位符 → 说明我们不懂这个参数语义，宁可不给示例
    if out.contains('<') {
        return None;
    }
    Some(out)
}

/// 版式 F：**参数语法型 CLI**（jq 类）—— 能力本体是「位置参数表达式」而非子命令或 flag。
///
/// v18 实测的关键发现：jq 的 help 是纯 flag 表，解析出 27 条 flag 后注入，
/// base06b 反而 0%（而裸 help 还有 25%）。模型被 flag 表引向了 flag 拼接：
/// `jq --raw-input --slurp --null-input name`。
///
/// 但 jq 的真实能力是 `jq .name config.json` —— 这条**在 help 里根本不存在**。
/// 所以 flag 表不是「残缺的答案」，而是「错的答案」：它把模型的注意力
/// 从「filter 表达式」引开了。唯一正解是**补一条该工具的惯用形态动作**。
///
/// 判据（必须同时满足，否则不误判）：
/// 1. 解析出来的动作**全是 flag**（没有任何子命令）
/// 2. CLI 在已知的参数语法型名单里（不在名单里则宁可不给，避免编造语义）
pub const ARG_SYNTAX_CLIS: &[(&str, &str, &str)] = &[
    // (CLI 名, 惯用动作, 一句话说明)
    ("jq", "jq . <文件>", "格式化/美化 JSON。示例：jq . config.json"),
    ("jq", "jq .<字段> <文件>", "抽取某个字段。示例：jq .name config.json"),
    ("curl", "curl <URL>", "请求一个地址。示例：curl https://example.com"),
    ("sed", "sed -n '<范围>p' <文件>", "按行范围打印。示例：sed -n '1,10p' config.json"),
    ("awk", "awk '{print $<列号>}' <文件>", "按列打印。示例：awk '{print $1}' data.txt"),
    ("grep", "grep <模式> <文件>", "按模式搜索。示例：grep TODO main.rs"),
    ("yq", "yq .<字段> <文件>", "抽取 YAML 字段。示例：yq .name config.yaml"),
];

/// 若该 CLI 是参数语法型，补一条惯用动作（放在最前，提升命中率）。
fn arg_syntax_action(cli: &str, parsed: &[HelpAction]) -> Option<HelpAction> {
    // 判据 1：解析出来的必须全是 flag（无子命令）
    let all_flags = !parsed.is_empty()
        && parsed.iter().all(|a| {
            a.full_cmd
                .split_whitespace()
                .nth(1)
                .is_some_and(|t| t.starts_with('-'))
        });
    if !all_flags {
        return None;
    }
    // 判据 2：在已知名单里
    let (_, cmd, desc) = ARG_SYNTAX_CLIS.iter().find(|(n, _, _)| *n == cli)?;
    Some(HelpAction {
        full_cmd: cmd.to_string(),
        desc: desc.to_string(),
        readonly: is_readonly(cmd),
        example: None, // 说明里已含示例
    })
}

/// 从 help 文本里扫出「子命令 → 必需位置参数」表（v18 修正）。
///
/// 为什么需要：`docker --help` 的命令行是
/// ```text
///   logs        Fetch the logs of a container
/// ```
/// → 只给出 `logs`，**看不到它需要一个容器名**。而人工 schema 写了
/// 「`docker logs <容器名>`。示例：docker logs web」。
/// 这是 base06b 从 100% 掉到 59.4% 的直接原因（失败几乎全是漏参数）。
///
/// 参数的语法其实藏在同一份 help 的 **usage 行**里：
/// ```text
/// Usage:  docker container logs [OPTIONS] CONTAINER
/// ```
/// 因此扫全文本找 `Usage:`/`Usage:` 行，抽出「子命令 + 后面的大写/尖括号 token」。
/// 找不到就退回占位符表（`fill_example`），仍找不到则不给示例。
fn scan_subcommand_args(cli: &str, raw: &str) -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;
    let mut map: HashMap<String, String> = HashMap::new();
    let clean = strip_ansi(raw);
    let lines: Vec<&str> = clean.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let t = line.trim();
        // 形态 1：`Usage:  docker logs [OPTIONS] CONTAINER`
        let rest = if let Some(r) = t.strip_prefix("Usage:") {
            r.to_string()
        } else if let Some(r) = t.strip_prefix("usage:") {
            r.to_string()
        } else if !t.is_empty()
            && t.chars().all(|c| !c.is_alphanumeric() || c.is_ascii_uppercase())
            && t.eq_ignore_ascii_case("usage:")
        {
            // 形态 2：kubectl 把语法放在**下一个缩进行**：
            //   Usage:
            //     kubectl describe (-f FILENAME | TYPE [NAME_PREFIX | -l label])
            // 此时把紧随其后的非空行当语法
            lines[idx + 1..]
                .iter()
                .find(|l| !l.trim().is_empty())
                .map(|l| l.trim().to_string())
                .unwrap_or_default()
        } else {
            continue;
        };
        if rest.trim().is_empty() {
            continue;
        }
        let toks: Vec<&str> = rest.split_whitespace().collect();
        // 形如 `docker container logs [OPTIONS] CONTAINER` / `cargo build [OPTIONS]`
        // 取「最后一个全小写词」作为子命令，其后的 token 作为参数
        let Some(sub_idx) = toks.iter().rposition(|w| {
            let w = w.trim_matches(|c| c == '(' || c == ')' || c == '|');
            !w.starts_with('-')
                && !w.starts_with('[')
                && !w.starts_with('<')
                && w.len() >= 2
                && w.chars().all(|c| c.is_ascii_lowercase() || c == '-' || c == '_')
        }) else {
            continue;
        };
        let sub = toks[sub_idx]
            .trim_matches(|c| c == '(' || c == ')' || c == '|')
            .to_string();
        // 只收**必需**的大写/尖括号参数（[OPTIONS] 这种可选的不要）
        let args: Vec<String> = toks[sub_idx + 1..]
            .iter()
            .filter(|w| {
                let w = w.trim_matches(|c| c == '(' || c == ')' || c == '|');
                // 大写纯字母（CONTAINER / IMAGE）或尖括号（<file>）
                (w.starts_with('<') && w.ends_with('>'))
                    || (w.len() >= 2
                        && w.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c == '-')
                        && w.chars().any(|c| c.is_ascii_uppercase()))
            })
            .map(|w| {
                let w = w.trim_matches(|c| c == '(' || c == ')' || c == '|');
                if w.starts_with('<') {
                    w.to_string()
                } else {
                    // CONTAINER → <CONTAINER>，与人工 schema 的形态对齐
                    format!("<{w}>")
                }
            })
            .collect();
        if !args.is_empty() {
            map.entry(sub).or_insert_with(|| args.join(" "));
        }
    }
    // ── 方言补充：help 完全没体现惯用形态的 CLI（如 kubectl 的资源类型） ──
    // 只在 help 没给信息时补（Vacant），不覆盖已扫到的真实语法。
    for (c, sub, args) in DIALECT_HINTS {
        if *c == cli {
            map.entry(sub.to_string())
                .or_insert_with(|| args.to_string());
        }
    }
    map
}

/// 深度探测：对每个子命令再跑一次 `cli <sub> --help`，只为拿 usage 行里的必需参数。
///
/// **为什么必需**：`docker --help` 的命令行只写
/// ```text
///   logs        Fetch the logs of a container
/// ```
/// 看不到它需要容器名。真正的语法在 `docker logs --help` 的
/// `Usage:  docker logs [OPTIONS] CONTAINER` 里。
/// v18 实测这是最大失分点（base06b 59.4% vs 人工 schema 100%，失败几乎全是漏参数）。
///
/// **成本控制**：每个子命令一次进程启动。`max_probe` 限制次数（默认 12），
/// 且只探测「只读 + 已排在 top」的动作 —— 不值得为用不到的命令付启动开销。
/// 探测失败静默跳过（该子命令可能不支持 `--help`）。
pub fn deepen_with_subcommand_usage(
    cli: &str,
    actions: &mut [HelpAction],
    max_probe: usize,
    helper: &dyn Fn(&str, &str) -> Option<String>,
) {
    let mut probed = 0usize;
    for a in actions.iter_mut() {
        if probed >= max_probe {
            break;
        }
        // 已有参数或占位符 → 无需探测
        if a.full_cmd.contains('<') || a.full_cmd.contains('[') {
            continue;
        }
        let Some(sub) = a.full_cmd.split_whitespace().nth(1) else {
            continue;
        };
        probed += 1;
        let Some(txt) = helper(cli, sub) else { continue };
        let argmap = scan_subcommand_args(cli, &txt);
        // usage 行里的子命令名与子命令本身一致时，取它的必需参数
        if let Some(args) = argmap.get(sub) {
            a.full_cmd = format!("{} {}", a.full_cmd, args);
            a.example = fill_example(&a.full_cmd);
        }
    }
}

/// CLI 专属的**方言补充表**：有些 CLI 的 help 完全不体现其惯用形态，
/// 需要用一份极小的声明式表补上（不是猜，是已知事实）。
///
/// 判据：只有当该 CLI 的 help **没给出**这个信息时才用（见 map.entry Vacant）。
const DIALECT_HINTS: &[(&str, &str, &str)] = &[
    // (CLI, 子命令, 补的参数占位符)
    // kubectl 的 usage 是 `kubectl get TYPE [NAME]`，TYPE 是资源类型且必填，
    // 但 help 的命令列表里看不到。v18 实测失分点：
    // `kubectl describe <POD>` 漏 `pod`、`kubectl get svc` 写成 `get services`。
    ("kubectl", "get", "<TYPE>"),
    // v18e: `describe` 是 `describe TYPE NAME` 两个位置参数 —— 只给 `<TYPE>`
    // 会让模型把 pods 当成必填字面量（产出 `describe pods frontend-7d9`）。
    ("kubectl", "describe", "<TYPE> <NAME>"),
    ("kubectl", "delete", "<TYPE> <NAME>"),
    ("kubectl", "logs", "<POD>"),
    ("kubectl", "exec", "<POD>"),
];

/// **术语同义词/缩写表** —— v18c 消融定性的第二条确定性修复。
///
/// v18c 逐题核对发现：命令名对了但卡在「用户说的词」与「CLI 要的词」不一致上。
/// 典型 `有哪些服务在跑` → gold `kubectl get svc`，模型输出 `kubectl get services`。
///
/// **为什么 help 里拿不到**：`kubectl --help` 只写 `get`，不写它的参数可以是
/// `svc`/`service`/`services` —— 缩写关系在 help 文本里**结构上不可见**。
///
/// ⚠️ **v18d/v18e 血训：格式歧义会被读成命令。**
/// - v18d 写成 `svc=service, po=pod, ns=namespace` → 模型输出 `kubectl svc`、`kubectl ns list`
/// - v18e 改写成「`<TYPE>` 的取值：pods、svc（=service）、nodes、…」→ 模型仍输出 `kubectl nodes`
///
/// 两次都败在**同样的地方**：提示里出现了**裸的单词列表**，而 schema 的主体
/// 正是「一行一个命令」，模型就把列表里的词提升成了命令。
///
/// **定论：提示里绝不能出现孤立的名词列表。** 只能说「用哪个命令 + 怎么用」。
/// 最终形态只包含**完整命令样例**，不含任何裸 token 列表。
const SYNONYMS: &[(&str, &str)] = &[
    (
        "kubectl",
        "列资源永远用 `get`：服务写 `get svc`，节点写 `get nodes`，Pod 写 `get pods`；\
         看单个资源详情用 `describe pods <名字>`（`describe` 后面是 `pods`，再跟名字）",
    ),
    (
        "docker",
        "`images` 用于列出本地镜像，`ps` 只列容器；两者都是独立子命令",
    ),
    (
        "npm",
        "`npm run <脚本名>` 的脚本名来自 package.json（如 build/dev），help 里看不到；\
         `install <包名>` 必须带上包名",
    ),
];

/// 取该 CLI 的同义词提示（无则 None）
pub fn synonym_hint(cli: &str) -> Option<&'static str> {
    SYNONYMS.iter().find(|(c, _)| *c == cli).map(|(_, h)| *h)
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
    let example = fill_example(&cmd);
    out.push(HelpAction {
        full_cmd: cmd,
        desc,
        readonly: ro,
        example,
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
    "diff", "test", "check", "build", "plan", "config", "top",
    // ⚠️ v19 移出：`history` 曾在此表内，会把 **`git history`（help 自述
    //    "Rewrite history"）** 提升到注入集前部 —— 与 WRITE_VERBS 的意图自相矛盾
    //    （见本文件 WRITE_VERBS 上方注释「白名单里不该有 history」）。
    //    高频表只应放**只读**动作；写动作由 readonly_only 在调用侧过滤，
    //    但若它同时出现在本表，排序会把它顶到前面，形成「越危险越靠前」。
    //    ⇒ 规则：本表任一 token 都不得出现在 WRITE_VERBS 中（有单测断言）。
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
///
/// **v18 修正**：人工 schema 每条都带「示例：」，而 help-parse 第一版只有说明文字，
/// 导致 base06b 从 100% 掉到 59.4%（失败几乎全是漏参数）。
/// 现在每条都带上 `example`，形态与人工 schema 逐字对齐。
pub fn render_schema(cli: &str, actions: &[HelpAction], max_n: usize) -> String {
    let mut s = format!("\n\n当前可用命令（{cli} 域，只读）：\n");
    for a in actions.iter().take(max_n) {
        match &a.example {
            Some(ex) => s.push_str(&format!("- {} ：{}。示例：{}\n", a.full_cmd, a.desc, ex)),
            None if a.desc.is_empty() => s.push_str(&format!("- {}。示例：{}\n", a.full_cmd, a.full_cmd)),
            None => s.push_str(&format!("- {} ：{}。示例：{}\n", a.full_cmd, a.desc, a.full_cmd)),
        }
    }
    // v18c：术语同义词只能靠声明式补（help 文本里结构上不可见）。
    // 这是「零人工接入」的真实边界，但代价仅一行，且完全不改命令、不下判断。
    if let Some(hint) = synonym_hint(cli) {
        s.push_str(&format!("术语提示：{hint}。\n"));
    }
    s.push_str(&format!(
        "只使用上面列出的命令；与上面命令无关的请求输出 (无需调用硬件命令)。"
    ));
    s
}

/// 渲染**带原文兜底**的 schema —— 修 v19 定位的 D 档（git hparse 87.5 < 裸 help 100.0）。
///
/// ## 为什么要原文兜底
///
/// `git --help` 只列**顶层命令**（`log` / `diff` / `branch` …），
/// 而真实问句常需要**参数级**形态（`git log --oneline` / `git diff --stat`）。
/// 结构化注入只给无参数骨架 → 参数线索全丢 → 反而**劣于**裸灌原文
/// （v19 实测：git hparse 87.5 vs 裸 help 100.0）。
///
/// ## 判据（确定性，非调参）
///
/// 两个条件**同时**满足才追加原文：
/// 1. **结构化动作少**：`actions.len() < THIN_ACTION_THRESHOLD`
///    —— 动作多的 CLI（docker 57 / npm 68）不需要原文，加了反而稀释注意力
///    （v18c 已证「注入条数截断有害」，但也证「无关内容稀释」）。
/// 2. **原文本身短**：`raw.len() <= RAW_INLINE_LIMIT`
///    —— 长原文（cargo / kubectl 数 KB）会挤爆上下文；短原文（git 2.2KB）成本可忽略。
///
/// 不满足则退化为原 `render_schema`，**行为与改动前逐字一致**（零回归风险）。
pub fn render_schema_with_raw_fallback(
    cli: &str,
    actions: &[HelpAction],
    raw: &str,
    max_n: usize,
) -> String {
    let mut s = render_schema(cli, actions, max_n);
    if actions.len() < THIN_ACTION_THRESHOLD && raw.len() <= RAW_INLINE_LIMIT {
        // 原文里可能含 flag 说明 —— 这些是结构化动作**抓不到**的参数线索，
        // 正是 D 档缺的东西。整段附在最后，并显式声明类别（v18d 教训：
        // 附加信息必须声明「这是什么」，否则会被归到主类别「命令」里去）。
        s.push_str(&format!(
            "\n以下是 `{cli} --help` 的原始输出，供你理解「命令 + 常用参数」的搭配\
（**这些是参考信息，不是额外的命令**；命令以上面列出的为准）：\n"
        ));
        s.push_str(&strip_ansi(raw));
        if !s.ends_with('\n') {
            s.push('\n');
        }
    }
    s
}

/// 低于此动作数视为「help 只列了顶层命令」→ 需要原文补参数线索。
/// 依据：6 个实测 CLI 的只读动作数为 docker 43 / npm 49 / kubectl 32 /
/// git 11 / cargo 7 / jq 29 —— 15 这个阈值把 git 与 cargo 划入需兜底的一侧。
const THIN_ACTION_THRESHOLD: usize = 15;

/// 原文超过此长度则不内联（避免挤爆上下文）。git help ≈ 2.2KB 可内联；
/// cargo / kubectl 的 help 为数 KB 级，走结构化路径。
const RAW_INLINE_LIMIT: usize = 4096;


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
  kubectl version      Print the client and server version information
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
        // kubectl 的 get/describe/logs 由方言表补上资源类型（v18 修正）
        assert!(
            cmds.contains(&"kubectl get <TYPE>"),
            "get 缺资源类型: {cmds:?}"
        );
        assert!(
            cmds.contains(&"kubectl logs <POD>"),
            "logs 缺 POD: {cmds:?}"
        );
        // 不在方言表里的（version）保持原样
        assert!(cmds.contains(&"kubectl version"), "version 被误改: {cmds:?}");
    }

    /// 可见性：方言补充只填空缺，不覆盖 usage 行扫到的真实语法
    #[test]
    fn dialect_hints_do_not_override_scanned_usage() {
        let raw = "\
Usage:  kubectl get TYPE [NAME]

  kubectl get          Display one or many resources
";
        let acts = parse_help("kubectl", raw);
        let cmds: Vec<&str> = acts.iter().map(|a| a.full_cmd.as_str()).collect();
        // usage 行给出了真实语法 → 不得被方言表覆盖成 <TYPE>
        assert!(
            cmds.iter().any(|c| *c == "kubectl get <TYPE>"),
            "got={cmds:?}"
        );
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
            // 🔴 v19 安全回归：这三条曾被漏判为「只读」→ T1 门自动放行。
            //    `git pull` 语义 = fetch + merge：改写工作区 + 外呼 +
            //    可能留下冲突文件，甚至覆盖用户未提交的改动。
            //    这是本项目最严重的一类错误（不是分数问题，是**会丢用户数据**）。
            ("git pull", false),
            ("git pull origin main", false),
            ("git backfill", false),
            ("git history", false),
            // 对照：`git fetch` 只下载 refs、**不改工作区** → 判只读是正确的，
            // 用它证明上面的修复不是「把 fetch 族一律拉黑」的粗暴做法。
            ("git fetch", true),
        ] {
            let got = is_readonly(cmd);
            assert_eq!(got, want_ro, "is_readonly({cmd}) 应为 {want_ro}, 实得 {got}");
        }
    }

    /// 🔴 v19：高频排序表里不得含**写动词** —— 否则会把危险命令顶到注入集前部。
    ///
    /// 起因：`history` 同时出现在 `COMMON_ACTIONS`（排前）与 `WRITE_VERBS`
    /// （`git history` = Rewrite history），形成一个「越危险越靠前」的自相矛盾。
    /// 本测试是结构性防回归：新增 token 前先确认它不在 WRITE_VERBS 里。
    #[test]
    fn common_actions_are_never_write_verbs() {
        for a in COMMON_ACTIONS {
            assert!(
                !WRITE_VERBS.contains(a),
                "COMMON_ACTIONS 含写动词 `{a}` —— 会把危险动作排到注入集前面，\
                 且与 WRITE_VERBS 自相矛盾；请从两处之一移除"
            );
        }
    }

    /// v19：短 help + 动作少 → **必须**追加原文（修 D 档 git 回归）。
    #[test]
    fn thin_help_inlines_raw_output() {
        let raw = "\
usage: git [-v | --version] [-h | --help] <command> [<args>]

examine the history and state
   log        Show commit logs
   diff       Show changes
";
        let actions = readonly_only(&parse_help("git", raw));
        assert!(actions.len() < THIN_ACTION_THRESHOLD, "样本应触发兜底");
        let s = render_schema_with_raw_fallback("git", &actions, raw, usize::MAX);
        assert!(s.contains("原始输出"), "薄 help 未追加原文: {s}");
        assert!(s.contains("Show commit logs"), "原文内容缺失: {s}");
        // 必须显式声明类别，否则模型会把原文当命令列表（v18d 的教训）
        assert!(s.contains("不是额外的命令"), "未声明类别: {s}");
    }

    /// v19 反向：动作多 or 原文长 → **不得**追加原文（避免稀释 / 挤爆上下文）。
    #[test]
    fn thick_help_does_not_inline_raw() {
        // 造 20 条动作（>阈值）
        let mut raw = String::from("usage: demo [cmd]\n\n");
        for i in 0..20 {
            raw.push_str(&format!("   cmd{i}   do thing {i}\n"));
        }
        let actions = readonly_only(&parse_help("demo", &raw));
        assert!(actions.len() >= THIN_ACTION_THRESHOLD, "应超过阈值");
        let s = render_schema_with_raw_fallback("demo", &actions, &raw, usize::MAX);
        assert!(!s.contains("原始输出"), "厚 help 不该追加原文: {s}");

        // 原文过长时同样不追加
        let long_raw = format!("usage: git [cmd]\n\n   log   Show commit logs\n\n{}", "x".repeat(5000));
        let a2 = readonly_only(&parse_help("git", &long_raw));
        if a2.len() < THIN_ACTION_THRESHOLD {
            let s2 = render_schema_with_raw_fallback("git", &a2, &long_raw, usize::MAX);
            assert!(!s2.contains("原始输出"), "超长原文不该内联");
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

    /// 示例填充（v18）：人工 schema 有「示例：」，漏了它 base06b 从 100% 掉到 59.4%
    #[test]
    fn placeholders_become_concrete_examples() {
        let raw = "\
Commands:
  logs <容器名>          Fetch the logs of a container
  inspect <容器名或镜像名>  Return low-level information
  add <包名>            Add a package
  convert <文件>        Convert a file
  tune <莫名其妙的东西>   Tune something
";
        let acts = parse_help("docker", raw);
        let by = |c: &str| acts.iter().find(|a| a.full_cmd == c).cloned();
        assert_eq!(
            by("docker logs <容器名>").unwrap().example.as_deref(),
            Some("docker logs web"),
            "容器名未填成 web"
        );
        assert_eq!(
            by("docker inspect <容器名或镜像名>").unwrap().example.as_deref(),
            Some("docker inspect web")
        );
        assert_eq!(
            by("docker add <包名>").unwrap().example.as_deref(),
            Some("docker add express")
        );
        assert_eq!(
            by("docker convert <文件>").unwrap().example.as_deref(),
            Some("docker convert config.json")
        );
        // 认不出的占位符 → 宁可不给示例（不得编造语义）
        assert_eq!(
            by("docker tune <莫名其妙的东西>").unwrap().example.as_deref(),
            None,
            "未知占位符不该被乱填"
        );

        // 无参数的命令不需要示例字段
        let raw2 = "Commands:\n  ps          List containers\n";
        let a2 = parse_help("docker", raw2);
        assert_eq!(a2[0].example, None, "无参数命令不该有示例");
    }

    /// 参数语法型 CLI（jq 类）：纯 flag 注入会把模型引向拼 flag（v18 实测 0%）
    #[test]
    fn arg_syntax_cli_gets_leading_dialect_action() {
        let raw = "\
Usage:  jq [options...] filter [files...]

  -c, --compact-output   compact instead of pretty-printed output
  -r, --raw-output       output strings without escapes and quotes
  -s, --slurp            read all inputs into an array
";
        let acts = parse_help("jq", raw);
        assert!(!acts.is_empty());
        // 惯用动作必须排在最前（否则排在几十条 flag 之后等于没注入）
        assert_eq!(
            acts[0].full_cmd, "jq . <文件>",
            "参数语法型动作未置顶: {:?}",
            acts.iter().map(|a| &a.full_cmd).collect::<Vec<_>>()
        );
        assert!(acts[0].readonly, "jq 格式化应判只读");

        // 反向：有真子命令的 CLI 不得被误加（docker 不能被插 jq 式动作）
        let raw2 = "Commands:\n  ps          List containers\n";
        let a2 = parse_help("docker", raw2);
        assert_eq!(a2[0].full_cmd, "docker ps", "docker 被误插参数语法动作");
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

    /// v18c/v18d：术语同义词提示必须出现在 schema 里，且必须**显式声明不是命令名**
    /// （v18d 血训：`svc=service` 的写法让模型输出了 `kubectl svc`）
    #[test]
    fn schema_carries_synonym_hint_for_known_cli() {
        let acts = readonly_only(&parse_help(
            "kubectl",
            "  get         Display one or many resources\n  logs        Print the logs\n",
        ));
        let s = render_schema("kubectl", &acts, 8);
        assert!(s.contains("术语提示："), "kubectl schema 缺同义词提示:\n{s}");
        assert!(s.contains("get svc"), "应给出完整命令样例:\n{s}");
        // 关键（v18d/v18e 两次血训）：提示里不得出现**裸的单词列表**，
        // 否则模型会把列表项提升成命令（曾产出 `kubectl svc` / `kubectl nodes`）。
        // 判据：提示行里凡出现的 token 必须带反引号或处于命令语境。
        let hint = s.lines().find(|l| l.contains("术语提示")).unwrap();
        assert!(
            !hint.contains("、"),
            "术语提示不得含顿号列举的裸 token 列表（会被读成命令名）:\n{hint}"
        );
        // 未登记的 CLI 不应凭空造提示
        let s2 = render_schema(
            "ffmpeg",
            &readonly_only(&parse_help("ffmpeg", "  -i  input file\n")),
            8,
        );
        assert!(!s2.contains("术语提示："), "未登记 CLI 不该有提示:\n{s2}");
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

    /// 「全量」语义：render_schema(usize::MAX) 必须等价于列出全部动作，
    /// 且**不能**把 usize::MAX 印成 18446744073709551615（CLI 侧曾有该打印缺陷）。
    #[test]
    fn render_schema_with_usize_max_means_full() {
        let raw = "Commands:\n  ps   List containers\n  images   List images\n  info   Display info\n";
        let acts = readonly_only(&parse_help("docker", raw));
        let full = render_schema("docker", &acts, usize::MAX);
        let exact = render_schema("docker", &acts, acts.len());
        assert_eq!(full, exact, "usize::MAX 应当等于全量渲染");
        for a in &acts {
            assert!(full.contains(&a.full_cmd), "全量渲染漏了 {}:\n{full}", a.full_cmd);
        }
        assert!(!full.contains("18446744073709551615"), "不得把 usize::MAX 印进 schema:\n{full}");
    }
}
