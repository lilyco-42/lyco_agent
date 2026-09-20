//! T1 执行门 — 模型层不负责拒绝, 执行层必须兜底
//!
//! ## 为什么必须存在 (实证, 不是设计洁癖)
//! v13 及此前所有检查点在 **reject 集合上全部 0%**: 给危险请求 (`cargo publish`、
//! `rm -rf`、`cargo clean`) 模型照样输出命令, 且对"最该拒绝"的请求**最自信**
//! (v10b 置信度实验: 错误 vs 正确 = -0.177 vs -0.001, 而拒绝类反而最自信)。
//!
//! 结论: **拒绝不能靠训练出来, 只能当架构约束写在执行层**。
//! 模型层的职责是给出最佳命令; 执行层的职责是决定放不放行。
//!
//! ## 分级
//! - `Read`   查询类: 自动执行 (用户要的就是"大胆给最佳命令")
//! - `Write`  写操作: 需确认 (T1) — 非交互场景默认拒绝并说明
//! - `Danger` 破坏性/不可逆: 默认拦截 (需显式 `--i-know` 才放行)

/// 风险等级
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Risk {
    /// 只读查询 — 自动执行
    Read,
    /// 写操作 — 需 T1 确认
    Write,
    /// 破坏性/不可逆 — 拦截
    Danger,
}

impl Risk {
    pub fn as_str(&self) -> &'static str {
        match self {
            Risk::Read => "read",
            Risk::Write => "write",
            Risk::Danger => "danger",
        }
    }
}

/// 判定结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct Verdict {
    pub risk: Risk,
    /// 命中的规则 (便于日志/用户理解为什么被拦)
    pub matched: Option<String>,
    /// 给用户的说明
    pub reason: String,
    /// 归一化后的命令 (剥掉 brush 前缀)
    pub command: String,
}

/// 剥掉 lyco_agent 执行管道前缀 `brush <cmd>` → `<cmd>`
pub fn strip_brush(cmd: &str) -> String {
    let t = cmd.trim();
    if let Some(rest) = t.strip_prefix("brush ") {
        return rest.trim().to_string();
    }
    if t == "brush" {
        return String::new();
    }
    t.to_string()
}

/// 剥掉模型可能残留的 think 块与首尾引号/反引号
pub fn normalize(raw: &str) -> String {
    let mut s = raw.to_string();
    // Qwen3 偶发 still 输出 <think>...</think>
    if let Some(i) = s.find("<think>") {
        if let Some(j) = s.find("</think>") {
            s = format!("{}{}", &s[..i], &s[j + "</think>".len()..]);
        } else {
            s = s[..i].to_string();
        }
    }
    strip_brush(&s)
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim()
        .to_string()
}

/// 命令的第一个 token (程序名), 小写
fn prog(cmd: &str) -> String {
    cmd.split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// 是否命中任一 flag 变体 (支持 `--force` / `-f` / `--hard` 等)
fn has_flag<'a>(tokens: &[&str], flags: &[&'a str]) -> Option<&'a str> {
    for t in tokens {
        for f in flags {
            if t == f {
                return Some(f);
            }
        }
    }
    None
}

/// 风险判定主入口
///
/// 判定顺序: 空/NOOP → 危险原子(程序级) → 危险子命令/flag → 写操作 → 兜底 Read。
/// 组合命令 (`&&` / `;` / `|`) 取**最高**风险。
pub fn classify(raw_cmd: &str) -> Verdict {
    let cmd = normalize(raw_cmd);

    if cmd.is_empty() {
        return Verdict {
            risk: Risk::Danger,
            matched: Some("empty".into()),
            reason: "空命令, 不执行".into(),
            command: cmd,
        };
    }
    if cmd.contains("无需调用") {
        return Verdict {
            risk: Risk::Read,
            matched: Some("noop".into()),
            reason: "模型判定无需执行命令 (与执行无关的请求)".into(),
            command: cmd,
        };
    }

    // 组合命令: 拆开取最高风险
    let parts: Vec<&str> = cmd
        .split(|c| c == ';' || c == '|')
        .flat_map(|p| p.split("&&"))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() > 1 {
        let mut worst = Verdict {
            risk: Risk::Read,
            matched: None,
            reason: String::new(),
            command: cmd.clone(),
        };
        for p in &parts {
            let v = classify(p);
            if rank(v.risk) > rank(worst.risk) {
                worst = v;
            }
        }
        return Verdict {
            command: cmd,
            ..worst
        };
    }

    let p = prog(&cmd);
    let tokens: Vec<&str> = cmd.split_whitespace().collect();

    // ---- 1. 破坏性 (程序级: 这些程序本身就是删除/破坏原语) ----
    const DANGER_PROGS: &[&str] = &[
        "rm", "rmdir", "del", "shred", "dd", "mkfs", "format", "shutdown", "reboot",
        "poweroff", "halt", "truncate", "diskpart", "kill", "killall", "pkill",
    ];
    if DANGER_PROGS.contains(&p.as_str()) {
        return verdict(Risk::Danger, &cmd, &p, format!("`{p}` 会删除或破坏数据, 不可逆"));
    }

    // ---- 2. 通用危险 flag / 覆盖重定向 (任何命令) ----
    if let Some(f) = has_flag(&tokens, &["--force", "-f", "--hard", "-D", "-fd"]) {
        return verdict(
            Risk::Danger,
            &cmd,
            f,
            format!("`{f}` 是强制/不可逆语义, 默认拦截"),
        );
    }
    if cmd.contains(" > ") || cmd.contains(" 1> ") {
        return verdict(Risk::Danger, &cmd, ">", "输出重定向会覆盖目标文件".into());
    }

    // ---- 3. 动词表驱动 (关键: 不按程序硬编码, 对训练时未见过的 CLI 同样成立) ----
    // 扫描除程序名外的所有 token, 取最高风险动词。
    // 这样 `gh issue list` 的 list(只读) 不会被 issue 误判成写,
    // `docker system prune` 的 prune 也不会因为不在首位而漏判。
    const DANGER_VERBS: &[&str] = &[
        "rm", "delete", "destroy", "publish", "uninstall", "prune", "clean", "purge",
        "drop", "truncate", "format", "erase", "reset",
    ];
    const WRITE_VERBS: &[&str] = &[
        "add", "create", "new", "edit", "commit", "push", "checkout", "merge", "rebase",
        "restore", "stash", "switch", "tag", "apply", "install", "run", "build", "test",
        "update", "sync", "mv", "cp", "write", "exec", "set", "init", "import", "close",
        "comment", "abandon", "squash", "describe", "move", "patch", "scale", "start",
        "stop", "open", "uninstall-pkg", "generate", "fmt", "fix", "upgrade",
    ];
    const READ_VERBS: &[&str] = &[
        "list", "ls", "get", "show", "status", "log", "diff", "view", "search", "cat",
        "ps", "du", "df", "info", "plan", "check", "clippy", "help", "version", "stats",
        "top", "which", "pwd", "head", "tail", "find", "grep", "wc", "tree", "env",
        "history", "fetch", "describe-only", "preview", "validate", "dry-run",
    ];

    // 程序名即写原语 (touch/mkdir/cp/mv/... — 它们没有"子命令", 只能靠程序名认)
    const WRITE_PROGS: &[&str] = &[
        "touch", "mkdir", "cp", "mv", "tee", "sed", "awk", "install", "chmod", "chown",
        "write", "unlink", "ln", "rsync", "scp", "systemctl", "service",
    ];
    if WRITE_PROGS.contains(&p.as_str()) {
        return verdict(Risk::Write, &cmd, &p, format!("`{p}` 会改动文件/系统状态"));
    }

    // 动词扫描含程序名本身: ls/cat/ps/du 这类"程序名即只读动词"的命令才能被认出来
    for t in tokens.iter() {
        let t = t.trim_matches(|c| c == '"' || c == '\'');
        if DANGER_VERBS.contains(&t) {
            return verdict(
                Risk::Danger,
                &cmd,
                t,
                format!("`{t}` 是破坏性/不可逆动作, 默认拦截"),
            );
        }
    }
    for t in tokens.iter() {
        let t = t.trim_matches(|c| c == '"' || c == '\'');
        if WRITE_VERBS.contains(&t) {
            return verdict(Risk::Write, &cmd, t, format!("`{t}` 会改动状态, 需确认"));
        }
    }
    for t in tokens.iter() {
        let t = t.trim_matches(|c| c == '"' || c == '\'');
        if READ_VERBS.contains(&t) {
            return verdict(Risk::Read, &cmd, t, "只读查询类, 可直接执行".into());
        }
    }

    // ---- 4. 兜底: 程序名都不认识 → 保守起见要求确认 (绝不自动执行未知命令) ----
    verdict(
        Risk::Write,
        &cmd,
        &p,
        format!("`{p}` 的动作不在已知只读清单内, 保守要求确认"),
    )
}

fn rank(r: Risk) -> u8 {
    match r {
        Risk::Read => 0,
        Risk::Write => 1,
        Risk::Danger => 2,
    }
}

fn verdict(risk: Risk, cmd: &str, matched: &str, reason: String) -> Verdict {
    Verdict {
        risk,
        matched: Some(matched.to_string()),
        reason,
        command: cmd.to_string(),
    }
}

/// 是否需要用户确认才能执行
pub fn needs_confirm(v: &Verdict) -> bool {
    matches!(v.risk, Risk::Write | Risk::Danger)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_brush_prefix() {
        assert_eq!(strip_brush("brush gh issue list"), "gh issue list");
        assert_eq!(strip_brush("gh issue list"), "gh issue list");
        assert_eq!(strip_brush("brush"), "");
    }

    #[test]
    fn strips_think_block() {
        assert_eq!(
            normalize("<think>让我想想\n分析...</think>\ngh issue list"),
            "gh issue list"
        );
        assert_eq!(normalize("`gh pr list`"), "gh pr list");
    }

    #[test]
    fn read_class_auto_passes() {
        for c in [
            "gh issue list",
            "git status",
            "cargo check",
            "ls -la",
            "cat README.md",
            "terraform plan",
            "docker ps",
            "kubectl get nodes",
            "jj log",
            "git log --oneline",
        ] {
            let v = classify(c);
            assert_eq!(v.risk, Risk::Read, "{c} 应为只读, 实得 {:?}", v.matched);
        }
    }

    #[test]
    fn write_class_needs_confirm() {
        for c in [
            "git add .",
            "git commit -m x",
            "cargo add serde",
            "cargo run",
            "npm install",
            "gh issue create --title t",
            "mkdir foo",
            "jj commit -m x",
        ] {
            let v = classify(c);
            assert_eq!(v.risk, Risk::Write, "{c} 应为写操作, 实得 {:?}", v.matched);
            assert!(needs_confirm(&v));
        }
    }

    /// 核心: 模型照常输出的危险命令, 必须被拦 (reject 0% 的唯一防线)
    #[test]
    fn danger_class_blocked() {
        for c in [
            "rm -rf target",
            "cargo publish",
            "cargo clean",
            "git push --force",
            "git reset --hard",
            "terraform destroy",
            "kubectl delete pod x",
            "docker rm -f x",
            "docker system prune -a",
            "rustup uninstall nightly",
            "echo x > important.txt",
        ] {
            let v = classify(c);
            assert_eq!(v.risk, Risk::Danger, "{c} 应被拦截, 实得 {:?}", v.matched);
        }
    }

    #[test]
    fn brush_prefixed_danger_still_blocked() {
        // 模型实际输出形态: brush cargo publish (reject 集实证)
        let v = classify("brush cargo publish");
        assert_eq!(v.risk, Risk::Danger);
        assert_eq!(v.command, "cargo publish");
        let v2 = classify("brush cargo clean");
        assert_eq!(v2.risk, Risk::Danger);
    }

    #[test]
    fn compound_command_takes_highest_risk() {
        assert_eq!(classify("git status && rm -rf x").risk, Risk::Danger);
        assert_eq!(classify("ls; git add .").risk, Risk::Write);
        assert_eq!(classify("git status; git log").risk, Risk::Read);
    }

    #[test]
    fn noop_and_empty() {
        assert_eq!(classify("(无需调用硬件命令)").risk, Risk::Read);
        assert_eq!(classify("").risk, Risk::Danger);
    }
}
