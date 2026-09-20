//! v13 路由器接线 — MVP 能力①: 把自然语言需求变成**一条可执行的 CLI 命令**
//!
//! ## 链路
//! ```text
//! NL → v13 模型 → 命令文本 → 归一化(剥 think/brush 前缀) → T1 门 → 决策 → brush 执行
//! ```
//!
//! ## 三条硬纪律 (都是踩过的坑)
//! 1. **system prompt 必须与训练逐字一致** (含 brush 通用 shell 域), 且 `enable_thinking=false` —
//!    不一致会直接掉点; 开着思考模型会先长篇推理而不输出命令。
//! 2. **temperature = 0** — 路由器要的是确定性, 0.7 会让同一请求产出不同命令。
//! 3. **模型层不负责拒绝** — reject 集合实测 0%, 危险命令照样输出且最自信。
//!    安全判定全部在 `t1gate`, 这里只做"该不该放行"的分诊, 不做"命令对不对"的判断。
//!
//! 模型推理抽象为 `CommandModel` trait: 本机单测用 Scripted (不碰模型),
//! 真实部署用 `LlamaCppRouter` (llama-server HTTP, 通常在远端 GPU 上)。

use crate::t1gate::{classify, normalize, Risk, Verdict};

/// v13 训练时的 system prompt — 逐字对齐, 改一个字都要重训
pub const V13_SYS: &str = "你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。\
支持的域：hw(硬件)/gh(github)/ff(ffmpeg)/lb(行情持仓只读)/brush(shell 通用命令)。\
只输出命令本身，不要解释；不支持的请求输出 (无需调用硬件命令)。";

/// 模型说"这活儿不该用命令干"
pub const NOOP: &str = "(无需调用硬件命令)";

/// 模型 IO 抽象 (便于无模型单测 + 远端/端侧多种后端)
pub trait CommandModel {
    fn infer(&mut self, system: &str, user: &str) -> anyhow::Result<String>;
}

/// llama-server (OpenAI 兼容 HTTP) 后端
pub struct LlamaCppRouter {
    client: reqwest::blocking::Client,
    base_url: String,
    model: String,
}

impl LlamaCppRouter {
    pub fn new(base_url: &str, model: &str) -> Self {
        crate::tools_runtime::install_crypto_provider();
        Self {
            client: reqwest::blocking::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
        }
    }

    pub fn health(&self) -> anyhow::Result<()> {
        let r = self
            .client
            .get(format!("{}/health", self.base_url))
            .send()?
            .status();
        anyhow::ensure!(r.is_success(), "llama-server unhealthy: {r}");
        Ok(())
    }
}

impl CommandModel for LlamaCppRouter {
    fn infer(&mut self, system: &str, user: &str) -> anyhow::Result<String> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            // 路由器: 零温度(确定性) + 短上限(只要一条命令) + 关思考
            "temperature": 0.0,
            "max_tokens": 64,
            "chat_template_kwargs": {"enable_thinking": false},
        });
        let v: serde_json::Value = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&body)
            .send()?
            .error_for_status()?
            .json()?;
        Ok(v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string())
    }
}

/// 路由结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct Route {
    /// 归一化后的命令 (剥掉 think 块与 brush 前缀)
    pub command: String,
    /// 模型原始输出 (排障用)
    pub raw: String,
    /// 模型判定"这不需要命令"
    pub is_noop: bool,
}

/// 分诊决策
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Decision {
    /// 只读 — 直接执行
    Run,
    /// 写操作 — 需 T1 确认后才执行
    Confirm,
    /// 破坏性 — 拦截 (需显式授权才可能放行)
    Block,
    /// 与命令无关的请求 — 不执行
    Noop,
}

/// 完整执行计划: NL → 命令 → 风险 → 决策
#[derive(Debug, Clone, serde::Serialize)]
pub struct Plan {
    pub nl: String,
    pub command: String,
    pub raw: String,
    pub risk: Risk,
    pub decision: Decision,
    pub verdict: Verdict,
}

/// 主入口: 问一句 → 出一份执行计划 (不执行)
pub fn plan(model: &mut dyn CommandModel, nl: &str) -> anyhow::Result<Plan> {
    plan_with_help(model, nl, 0)
}

/// 带「出域自动接入」的入口 —— **v18 产品闭环**。
///
/// 流程：
///   1. 先用域内 system prompt 问一次（覆盖 hw/gh/ff/lb/brush 五个已训域）
///   2. 若模型判定「无需调用」（= 出域），且 `help_top_n > 0`，
///      则尝试 `help_aware_schema` 自动接入：猜 CLI → 抓 --help → 确定性解析 → 二次询问
///   3. 二次询问若给出命令，用它；否则保留第一次的原判定
///
/// `help_top_n` = 注入条数；**传 0 可关闭自动接入**。v18c 实测截断是净损失，
/// 故生产调用应传 `usize::MAX`（全量）。
///
/// 为什么二次询问而不是「一开始就注入」：域内五个域有手写 schema，形态更准；
/// 自动接入只在出域时生效，**不干扰任何已训域的行为**（域内零回归）。
pub fn plan_with_help(
    model: &mut dyn CommandModel,
    nl: &str,
    help_top_n: usize,
) -> anyhow::Result<Plan> {
    let raw = model.infer(V13_SYS, nl)?;
    let p = plan_from_raw(nl, &raw);

    // 只有「模型自己说出域了」才触发自动接入 —— 域内行为零改动
    if p.decision == Decision::Noop && help_top_n > 0 {
        if let Some(h) = help_aware_schema(nl, help_top_n) {
            let sys = format!("{V13_SYS}{}", h.schema);
            if let Ok(raw2) = model.infer(&sys, nl) {
                let p2 = plan_from_raw(nl, &raw2);
                // 二次询问给出了真命令（非 noop）才采纳，否则不劣化原结果
                if p2.decision != Decision::Noop {
                    return Ok(p2);
                }
            }
        }
    }
    Ok(p)
}

/// 从已知模型输出构造计划 (远端批量跑模型 → 本地分诊时用这个)
pub fn plan_from_raw(nl: &str, raw: &str) -> Plan {
    let command = normalize(raw);
    let is_noop = command.contains("无需调用");
    let verdict = classify(&command);
    let decision = if is_noop {
        Decision::Noop
    } else {
        match verdict.risk {
            Risk::Read => Decision::Run,
            Risk::Write => Decision::Confirm,
            Risk::Danger => Decision::Block,
        }
    };
    Plan {
        nl: nl.to_string(),
        command,
        raw: raw.to_string(),
        risk: verdict.risk,
        decision,
        verdict,
    }
}

/// 是否已获授权 (写操作需 confirmed, 危险命令需 confirmed + override)
pub fn may_execute(p: &Plan, confirmed: bool, danger_override: bool) -> bool {
    match p.decision {
        Decision::Run => true,
        Decision::Confirm => confirmed,
        Decision::Block => confirmed && danger_override,
        Decision::Noop => false,
    }
}

/// 执行 (已分诊通过才调) — 走 brush; 调用方负责先过 may_execute
pub fn execute(p: &Plan, cwd: Option<&std::path::Path>) -> crate::tools_runtime::RunOutcome {
    crate::tools_runtime::shell_exec(&p.command, cwd)
}

// ══════════════ 出域自动接入（v17c）：不会的 CLI 靠读 `--help` 自学 ══════════════
//
// 用户需求（2026-09-21）：「提高泛用性，即使不会的 CLI，也能学习 --help 学习等」。
//
// 实测结论链（docs/research-v17-help-selflearning-2026-09-21.md）：
//   v17  裸灌 `--help`         → base06b 23.2% vs 人工 schema 100%（−76.8pp）❌
//   v17b 让模型自己提炼 help    → base06b 仅 +8.2pp；v13 反而 −18.3pp          ❌
//   v17c 确定性解析器产出动作表  → 前缀一定带对，模型只做「对齐 + 补槽位」      ✅
//
// 本函数是把这条链接进产品流程的入口：**出域 → 抓 help → 解析 → 渲染 schema**。

/// 出域自动接入的结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct HelpAware {
    /// 探测到的 CLI 名（从 NL 里抽出的第一个「看起来像命令」的 token）
    pub cli: String,
    /// 抓到的原始 help 文本长度（0 = 该 CLI 不存在或取不到 help）
    pub help_len: usize,
    /// 解析出的动作总数 / 只读数
    pub parsed: usize,
    pub readonly: usize,
    /// 注入用的 schema 块（形态与人工 schema 一致）
    pub schema: String,
}

/// 从自然语言里猜 CLI 名。
///
/// 启发式：取**第一个全 ASCII 小写、长度 2–16、且本机存在的可执行文件 token**。
/// 找不到本机可执行文件时，退而取第一个形如 `[a-z][a-z0-9-]+` 的 token。
/// 故意做得保守：宁可返回 None，也不要猜出一个不存在的 CLI 去执行。
pub fn guess_cli(nl: &str) -> Option<String> {
    const STOP: &[&str] = &[
        "the", "a", "an", "and", "or", "to", "for", "with", "in", "on", "of",
        "json", "file", "files", "dir", "path", "log", "list", "show", "get",
    ];
    let mut fallback: Option<String> = None;
    for tok in nl.split(|c: char| c.is_whitespace() || c == '，' || c == '。' || c == '、') {
        let t = tok.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
        if t.len() < 2 || t.len() > 16 || STOP.contains(&t) {
            continue;
        }
        if !t.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
            continue;
        }
        if !t
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            continue;
        }
        if which(t).is_some() {
            return Some(t.to_string());
        }
        if fallback.is_none() {
            fallback = Some(t.to_string());
        }
    }
    fallback
}

/// 极简 `which`：在 PATH 里找可执行文件（含 Windows 的 PATHEXT 变体）。
pub fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .map(|s| s.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let cand = dir.join(format!("{name}{ext}"));
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// 抓 `cli --help`（多级回退，同 `cmd_help_parse` 的口径）。
pub fn grab_help(cli: &str) -> Option<String> {
    for args in [vec!["--help"], vec!["-h"], vec!["help"]] {
        let mut cmd = std::process::Command::new(cli);
        cmd.args(&args);
        cmd.env("NO_COLOR", "1");
        cmd.env("PAGER", "cat");
        if let Ok(o) = cmd.output() {
            let mut t = String::from_utf8_lossy(&o.stdout).to_string();
            t.push_str(&String::from_utf8_lossy(&o.stderr));
            if t.trim().len() > 60 {
                return Some(t);
            }
        }
    }
    None
}

/// **出域自动接入**：NL → 猜 CLI → 抓 help → 确定性解析 → 渲染注入 schema。
///
/// 返回 `None` 表示「接不进来」（CLI 不存在 / 取不到 help / 解析为空），
/// 调用方应落回既有的「出域泛用优先三层」。
pub fn help_aware_schema(nl: &str, top_n: usize) -> Option<HelpAware> {
    use crate::help_parse::{parse_help, rank_by_frequency, readonly_only, render_schema};

    let cli = guess_cli(nl)?;
    let raw = grab_help(&cli)?;
    let parsed = parse_help(&cli, &raw);
    if parsed.is_empty() {
        return None;
    }
    let acts = rank_by_frequency(&readonly_only(&parsed));
    if acts.is_empty() {
        return None;
    }
    Some(HelpAware {
        cli: cli.clone(),
        help_len: raw.len(),
        parsed: parsed.len(),
        readonly: acts.len(),
        schema: render_schema(&cli, &acts, top_n),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 出域自动接入：从 NL 猜出 CLI 名
    #[test]
    fn guess_cli_picks_the_command_token() {
        // git 在本机存在（任何开发机都有）→ 应命中
        let g = guess_cli("用 git 看下当前仓库状态");
        assert_eq!(g.as_deref(), Some("git"), "got={g:?}");
        // 纯中文 → 没有可猜的 token → None
        assert_eq!(guess_cli("看看现在有哪些容器在跑"), None);
        // 形如 flag / 纯数字的 token 不该被当成 CLI
        assert_eq!(guess_cli("-v 全部 42"), None);
    }

    /// help_aware_schema 对真实存在的 CLI 应产出带前缀的 schema
    #[test]
    fn help_aware_schema_produces_prefixed_actions() {
        // git 必然存在；help 排版规整 → 解析必非空
        let Some(h) = help_aware_schema("用 git 看一下提交历史", 8) else {
            // 极端环境下 git 不可用时跳过（不算失败）
            return;
        };
        assert_eq!(h.cli, "git");
        assert!(h.parsed > 0, "解析为空: {h:?}");
        assert!(h.schema.contains("- git "), "schema 缺前缀:\n{}", h.schema);
        // 不得把写操作注入进去
        assert!(!h.schema.contains("git commit"), "写操作漏进 schema:\n{}", h.schema);
        assert!(!h.schema.contains("git push"), "写操作漏进 schema:\n{}", h.schema);
    }

    /// 出域自动接入的负向用例：CLI 不存在 → None（不得猜出不存在的命令）
    #[test]
    fn help_aware_schema_returns_none_for_unknown_cli() {
        assert!(help_aware_schema("用 zznotacli 干点活", 8).is_none());
    }

    /// 回放后端: 不加载模型, 直接返回预设输出 (本机单测绝不跑真模型)
    struct Scripted {
        out: String,
        seen_system: String,
    }
    impl CommandModel for Scripted {
        fn infer(&mut self, system: &str, _user: &str) -> anyhow::Result<String> {
            self.seen_system = system.to_string();
            Ok(self.out.clone())
        }
    }

    fn scripted(out: &str) -> Scripted {
        Scripted {
            out: out.to_string(),
            seen_system: String::new(),
        }
    }

    #[test]
    fn system_prompt_matches_training() {
        let mut m = scripted("gh issue list");
        let _ = plan(&mut m, "列出没关的 issue").unwrap();
        assert_eq!(m.seen_system, V13_SYS);
        assert!(V13_SYS.contains("brush(shell 通用命令)"), "brush 域必须在 prompt 里");
    }

    /// 出域自动接入闭环：第一次判定 noop → 抓 help → 二次询问拿到命令
    #[test]
    fn out_of_domain_escalates_to_help_prompt() {
        // 用一个可编程 backend：第一次回 noop，第二次回 docker ps
        struct Escalate {
            n: usize,
            systems: Vec<String>,
        }
        impl CommandModel for Escalate {
            fn infer(&mut self, system: &str, _nl: &str) -> anyhow::Result<String> {
                self.systems.push(system.to_string());
                self.n += 1;
                Ok(if self.n == 1 {
                    "(无需调用硬件命令)".to_string()
                } else {
                    "brush docker ps".to_string()
                })
            }
        }
        let mut m = Escalate { n: 0, systems: vec![] };
        // 只有 docker 存在于任何环境？不一定 —— 用 git（开发机必有）
        let p = plan_with_help(&mut m, "用 git 看下提交历史", usize::MAX).unwrap();
        if m.systems.len() < 2 {
            return; // 极端环境：git 取不到 help，跳过
        }
        assert!(
            m.systems[1].contains("当前可用命令（git 域"),
            "第二次 system 应带 help schema:\n{}",
            m.systems[1]
        );
        assert_eq!(p.command, "docker ps");
        assert_eq!(p.decision, Decision::Run);
    }

    /// help_top_n = 0 → 关闭自动接入，行为与旧 `plan` 完全一致（零回归保证）
    #[test]
    fn help_top_n_zero_disables_escalation() {
        let mut m = scripted("(无需调用硬件命令)");
        let p = plan_with_help(&mut m, "用 git 看下提交历史", 0).unwrap();
        assert_eq!(p.decision, Decision::Noop);
        assert_eq!(m.seen_system, V13_SYS, "关闭时不得改动 system prompt");
    }

    #[test]
    fn normalizes_brush_prefixed_output() {
        // 模型实测输出形态: `brush <cmd>`
        let p = plan_from_raw("列出没关的 issue", "brush gh issue list");
        assert_eq!(p.command, "gh issue list");
        assert_eq!(p.decision, Decision::Run);
    }

    #[test]
    fn strips_think_block_before_classify() {
        let p = plan_from_raw("x", "<think>想想\n</think>\ngh pr list");
        assert_eq!(p.command, "gh pr list");
        assert_eq!(p.decision, Decision::Run);
    }

    #[test]
    fn read_requests_run_directly() {
        for (nl, raw) in [
            ("看看有哪些 issue", "brush gh issue list"),
            ("当前状态", "brush git status"),
            ("编译检查一下", "brush cargo check"),
        ] {
            let p = plan_from_raw(nl, raw);
            assert_eq!(p.decision, Decision::Run, "{nl} / {raw}");
            assert!(may_execute(&p, false, false));
        }
    }

    #[test]
    fn write_requests_need_confirm() {
        let p = plan_from_raw("装个 serde", "brush cargo add serde");
        assert_eq!(p.decision, Decision::Confirm);
        assert!(!may_execute(&p, false, false), "未确认不得执行");
        assert!(may_execute(&p, true, false), "确认后可执行");
    }

    /// 核心: 模型照常吐出的危险命令, 必须被分诊为 Block (reject 0% 的唯一防线)
    #[test]
    fn danger_requests_blocked_even_with_confirm() {
        for raw in [
            "brush cargo publish",
            "brush cargo clean",
            "rm -rf target",
            "git push --force",
        ] {
            let p = plan_from_raw("发布/清理", raw);
            assert_eq!(p.decision, Decision::Block, "{raw}");
            assert!(!may_execute(&p, true, false), "仅确认不足以放行危险命令: {raw}");
            assert!(may_execute(&p, true, true), "确认+显式 override 才行: {raw}");
        }
    }

    #[test]
    fn noop_is_not_executed() {
        let p = plan_from_raw("今天天气怎么样", NOOP);
        assert_eq!(p.decision, Decision::Noop);
        assert!(!may_execute(&p, true, true), "noop 永远不执行");
    }
}
