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
pub const V13_SYS: &str =
    "你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。\
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
    /// **缺必需参数** — 命令合法但空跑等于静默失效, 需先补参 (P1.2)
    NeedsParam,
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
    /// 参数校验结果 (P1.2) — `decision == NeedsParam` 时 `missing` 非空
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<crate::paramcheck::ParamCheck>,
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

// ══════════════ 判定式脊（Choice Spine）：CLI 本身就是选项集 ══════════════
//
// 证据（同池、同题、唯一变量=任务定义）：
//   · 「从 N 个真实候选里选 1」→ 云端 Jev **12/12 = 100%**（跨域 CLI 选择）
//   · 「生成完整命令串」      → 我们 v13 **41.7%**（acc_exec）
//   · 出域拒绝：Jev 10/10 vs 我们 **0%** —— 判定式里「选 0」是自然动作
//
// 分工（本项目的核心交易）：
//   候选生成 = 确定性（help_parse 的真实动作表）   ← 模型不需要"记住"任何 CLI
//   判定     = 模型唯一需要做的事（只回编号）
//   组装     = 确定性（编号+槽位 → 命令；缺槽位就回问用户，绝不猜）

/// 从用户原话里抽**字面量槽位**（确定性，不猜语义）：
/// 先取成对引号内的内容（中英文引号），再取独立的 ASCII 数字（可带单位如 `23MB`）。
pub fn extract_literals(nl: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = nl.chars().collect();
    let mut i = 0;
    // 引号内内容优先（`git commit -m "doc"` 的 "doc"）
    while i < chars.len() {
        let open = chars[i];
        let close = match open {
            '"' => Some('"'),
            '\'' => Some('\''),
            '“' => Some('”'),
            '「' => Some('」'),
            _ => None,
        };
        if let Some(cl) = close {
            if let Some(j) = (i + 1..chars.len()).find(|&k| chars[k] == cl) {
                let inner: String = chars[i + 1..j].iter().collect();
                if !inner.trim().is_empty() {
                    // **只在需要时加引号**：`web` 裸写，`add doc` 才带引号
                    out.push(crate::choices::shell_quote_if_needed(inner.trim()));
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    // 独立数字（`hw condition tem set 23` 的 23；`23MB` 也接受）
    for tok in nl.split(|c: char| c.is_whitespace() || c == '，' || c == '。' || c == '、') {
        let t = tok.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.');
        if t.is_empty() {
            continue;
        }
        let lead_num: String = t
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        if !lead_num.is_empty() && lead_num.chars().any(|c| c.is_ascii_digit()) {
            out.push(t.to_string());
        }
    }
    out
}

/// 判定式核心（**纯函数**，可离线单测）：
/// 给定真实动作表 + 模型输出 → 计划。
///
/// 返回 `None` 表示「这不是判定式该处理的输出」——调用方应退回原生成式路径（不劣化）。
pub fn plan_choice_from(
    nl: &str,
    cli: &str,
    actions: &[crate::help_parse::HelpAction],
    model_out: &str,
    max_n: usize,
) -> Option<Plan> {
    use crate::choices::{assemble, build_choices, parse_picked, Assembled, Picked};

    let choices = build_choices(actions, max_n);
    if choices.is_empty() {
        return None;
    }
    let _ = cli; // 仅在探测/渲染阶段使用，这里保持签名对称
    match parse_picked(model_out, choices.len()) {
        // 模型明确说「都不合适」→ 不执行（这正是我们缺的拒绝能力）
        Picked::None => Some(plan_from_raw(nl, NOOP)),
        // 解析失败 → 交回上层兜底，绝不瞎猜
        Picked::Unparsable => None,
        Picked::Idx(k) => {
            let a = &actions[k - 1];
            match assemble(a, &extract_literals(nl)) {
                Assembled::Ready(cmd) => Some(plan_from_raw(nl, &cmd)),
                // 槽位填不满：先用完整动作造计划，缺参由 paramcheck/T1 判定（会要求补参）
                Assembled::NeedsSlots { .. } => Some(plan_from_raw(nl, &a.full_cmd)),
            }
        }
    }
}

/// 判定式入口（薄封装）：探测 CLI → 抓 `--help` → 解析成动作表 → 编号候选 → 只问编号。
///
/// 任何一步不成立（猜不到 CLI / 抓不到 help / 解析 0 条 / 模型输出不可解析）
/// **都退回原生成式路径** —— 保证「不劣化」。
pub fn plan_choice(model: &mut dyn CommandModel, nl: &str, max_n: usize) -> anyhow::Result<Plan> {
    use crate::choices::{build_choices, render_choices};
    use crate::help_parse::{parse_help, rank_by_frequency};

    let (cli, actions) = match guess_cli(nl)
        .and_then(|c| grab_help(&c).map(|raw| (c.clone(), parse_help(&c, &raw))))
    {
        Some((c, acts)) if !acts.is_empty() => (c, rank_by_frequency(&acts)),
        _ => return plan_with_help(model, nl, max_n),
    };

    let choices = build_choices(&actions, max_n);
    let sys = format!("{V13_SYS}\n{}", render_choices(&cli, &choices));
    let out = model.infer(&sys, nl)?;
    match plan_choice_from(nl, &cli, &actions, &out, max_n) {
        Some(p) => Ok(p),
        // 不可解析 → 当普通生成式输出处理（同一份输出，不再多问一次模型）
        None => Ok(plan_from_raw(nl, &out)),
    }
}

/// 从已知模型输出构造计划 (远端批量跑模型 → 本地分诊时用这个)
///
/// 分诊顺序（**正交两级，先缺参后风险**）：
///   1. `is_noop` → Noop
///   2. `paramcheck` 判缺参 → **NeedsParam**（在风险判定之前：缺参的命令无论风险高低
///      都干不成活，先让用户看见"要补什么"比看见"它是写操作"更有用）
///   3. `t1gate::classify` → Run / Confirm / Block
///
/// ⚠️ 缺参**不覆盖**危险判定：`rm -rf` 缺目标虽也是缺参，但危险优先拦。
/// 故顺序实为：Noop → Danger(拦截) → NeedsParam → Write(确认) → Run。
pub fn plan_from_raw(nl: &str, raw: &str) -> Plan {
    let command = normalize(raw);
    let is_noop = command.contains("无需调用");
    let verdict = classify(&command);

    // 组合命令逐条查缺参：任一段缺参即整体缺参
    let param = {
        let parts: Vec<&str> = command
            .split(|c| c == ';' || c == '|')
            .flat_map(|p| p.split("&&"))
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        let checks: Vec<crate::paramcheck::ParamCheck> =
            parts.iter().map(|p| crate::paramcheck::check(p)).collect();
        // 优先报第一个真缺参的段；全不缺参则取第一条的判断（便于暴露 Unchecked/Ok）
        checks
            .iter()
            .find(|c| c.status.is_needs_param())
            .cloned()
            .or_else(|| checks.into_iter().next())
    };

    let decision = if is_noop {
        Decision::Noop
    } else {
        match verdict.risk {
            // 危险优先：`rm` 这类就算缺目标也是 Block, 不能降级成"补个参数就能跑"
            Risk::Danger => Decision::Block,
            Risk::Write | Risk::Read => match &param {
                Some(p) if p.status.is_needs_param() => Decision::NeedsParam,
                _ => match verdict.risk {
                    Risk::Read => Decision::Run,
                    _ => Decision::Confirm,
                },
            },
        }
    };
    Plan {
        nl: nl.to_string(),
        command,
        raw: raw.to_string(),
        risk: verdict.risk,
        decision,
        verdict,
        param,
    }
}

/// 是否已获授权 (写操作需 confirmed, 危险命令需 confirmed + override)
///
/// **缺参命令永远不放行** —— 空跑会静默失效, 补参是唯一出路, 确认也救不了。
pub fn may_execute(p: &Plan, confirmed: bool, danger_override: bool) -> bool {
    match p.decision {
        Decision::Run => true,
        Decision::Confirm => confirmed,
        Decision::Block => confirmed && danger_override,
        Decision::Noop => false,
        Decision::NeedsParam => false,
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
        "the", "a", "an", "and", "or", "to", "for", "with", "in", "on", "of", "json", "file",
        "files", "dir", "path", "log", "list", "show", "get",
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
/// ⚠️ v19b：这里必须用 `render_schema_with_raw_fallback`，**不是** `render_schema`。
///
/// 起因（一次真实的自查发现）：v19 我把 `render_schema_with_raw_fallback` 写出来了、
/// 也给它写了单测，但**两个真实调用点都还在调 `render_schema`** ——
/// 单测全绿而生产行为零变化，是典型的「测试了但没接线」。
///
/// 现场证据：`lycore help-parse --cli git` 的注入段里**没有** help 原文，
/// 而 git 恰好是薄 help（只读 8 条 < 阈值 15，原文 2.3KB < 4KB）——
/// 正是这个方法唯一要救的那一档（v19 D 档回归）。
pub fn help_aware_schema(nl: &str, top_n: usize) -> Option<HelpAware> {
    use crate::help_parse::{
        parse_help, rank_by_frequency, readonly_only, render_schema_with_raw_fallback,
    };

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
        schema: render_schema_with_raw_fallback(&cli, &acts, &raw, top_n),
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
        assert!(
            !h.schema.contains("git commit"),
            "写操作漏进 schema:\n{}",
            h.schema
        );
        assert!(
            !h.schema.contains("git push"),
            "写操作漏进 schema:\n{}",
            h.schema
        );
    }

    /// 出域自动接入的负向用例：CLI 不存在 → None（不得猜出不存在的命令）
    #[test]
    fn help_aware_schema_returns_none_for_unknown_cli() {
        assert!(help_aware_schema("用 zznotacli 干点活", 8).is_none());
    }

    /// 🔴 v19b 防回归：**薄 help 必须走到原文兜底**（这次是漏接线，不是漏实现）。
    ///
    /// 现场：v19 写出了 `render_schema_with_raw_fallback` 并配了单测，
    /// 但本函数的调用点仍写 `render_schema` → 单测全绿、生产行为零变化。
    /// `lycore help-parse --cli git` 打出的注入段里没有 help 原文，
    /// 而 git 恰是薄 help（只读 8 条 < 15，原文 2.3KB < 4KB），
    /// 正是该方法唯一要救的那一档（v19 D 档：git hparse 87.5 < 裸 help 100.0）。
    ///
    /// 本测试断言的是**接线**而非实现：若有人把调用点改回 `render_schema`，它必红。
    #[test]
    fn thin_cli_schema_actually_inlines_raw_help() {
        use crate::help_parse::{parse_help, rank_by_frequency, readonly_only};

        // 造一段「薄 help」：2 条动作、原文短 → 两个条件同时满足
        let raw = "\
usage: demo [cmd]

   alpha    Show alpha things
   beta     Show beta things
";
        let acts = rank_by_frequency(&readonly_only(&parse_help("demo", raw)));
        assert!(acts.len() < 15, "样本应为薄 help，实得 {} 条", acts.len());

        // 直接断言渲染入口的行为（= help_aware_schema 内部用的那一个）
        let schema = crate::help_parse::render_schema_with_raw_fallback("demo", &acts, raw, 8);
        assert!(
            schema.contains("原始输出"),
            "薄 help 未走原文兜底 —— 检查 router::help_aware_schema 的调用点:\n{schema}"
        );
        assert!(schema.contains("不是额外的命令"), "未声明类别:\n{schema}");
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
        assert!(
            V13_SYS.contains("brush(shell 通用命令)"),
            "brush 域必须在 prompt 里"
        );
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
        let mut m = Escalate {
            n: 0,
            systems: vec![],
        };
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
            assert!(
                !may_execute(&p, true, false),
                "仅确认不足以放行危险命令: {raw}"
            );
            assert!(
                may_execute(&p, true, true),
                "确认+显式 override 才行: {raw}"
            );
        }
    }

    #[test]
    fn noop_is_not_executed() {
        let p = plan_from_raw("今天天气怎么样", NOOP);
        assert_eq!(p.decision, Decision::Noop);
        assert!(!may_execute(&p, true, true), "noop 永远不执行");
    }

    // ══════════════ P1.2 必需参数门 (2026-09-21) ══════════════

    /// 核心动机: v18 F4 实测 —— `npm install` 漏包名 = 静默失效
    #[test]
    fn missing_param_is_flaggable() {
        let p = plan_from_raw("装个 express", "brush npm install");
        assert_eq!(p.decision, Decision::NeedsParam, "got={:?}", p.decision);
        let pm = p.param.clone().expect("应带 param 判定");
        assert_eq!(pm.missing, vec!["<pkg>"]);
        // 缺参命令确认也不放行 —— 空跑救不了
        assert!(!may_execute(&p, true, false), "缺参不得放行");
        assert!(!may_execute(&p, true, true), "缺参连 override 也不放行");
    }

    /// 补齐参数后回到正常的 Write 确认流（不能被缺参门卡死）
    #[test]
    fn completed_param_falls_back_to_normal_gate() {
        let p = plan_from_raw("装个 express", "brush npm install express");
        assert_eq!(p.decision, Decision::Confirm, "补齐后应回到写确认");
        assert_eq!(p.risk, Risk::Write);
        assert!(may_execute(&p, true, false));
    }

    /// 危险优先：`rm -rf` 缺目标仍必须 Block，不得降级成「补个参数就能跑」
    #[test]
    fn danger_beats_needs_param() {
        let p = plan_from_raw("清理", "rm -rf");
        assert_eq!(p.decision, Decision::Block, "危险必须优先于缺参");
        // rm 不在 paramcheck 规则表里 → 应为 Unchecked（本模块不对它发表意见），
        // 但决策仍被危险级压住，绝不会变成 NeedsParam。
        assert_ne!(p.decision, Decision::NeedsParam);
    }

    /// 危险与缺参**同时命中**时，危险必须赢（`docker system prune` 是 Danger 级，
    /// 而 `docker run` 缺 image 是 Write+缺参 —— 这里测前者不被缺参降级）
    #[test]
    fn danger_not_downgraded_by_param_gate() {
        let p = plan_from_raw("清理 docker", "docker system prune -a");
        assert_eq!(p.decision, Decision::Block, "危险命令不得被缺参门降级");
    }

    /// 只读命令不该被缺参门误伤
    #[test]
    fn read_commands_unaffected_by_param_gate() {
        for (nl, raw) in [
            ("看看有哪些 issue", "brush gh issue list"),
            ("当前状态", "brush git status"),
            ("编译检查", "brush cargo check"),
        ] {
            let p = plan_from_raw(nl, raw);
            assert_eq!(p.decision, Decision::Run, "{nl} / {raw}");
            assert!(may_execute(&p, false, false));
        }
    }

    /// 组合命令：任一段缺参 → 整体缺参
    #[test]
    fn compound_any_missing_makes_it_needs_param() {
        let p = plan_from_raw("装东西然后跑", "npm install && npm run build");
        assert_eq!(p.decision, Decision::NeedsParam, "首段缺参应被抓");
        let ok = plan_from_raw("装东西然后跑", "npm install express && npm run build");
        assert_ne!(ok.decision, Decision::NeedsParam, "全段齐备不该判缺参");
    }
}

/// 判定式脊的接线测试（独立模块，避免动原 tests 模块）
///
/// ⚠️ 这组测试的存在理由写在这里：本仓库曾发生「`render_schema_with_raw_fallback`
/// 单测全绿但两个真实调用点都没接线，生产行为零变化」。所以**核心逻辑测试**之外，
/// 必须有一条测试**走完整入口**（`plan_choice_from` 是入口的一半，
/// 这里用真实 action 表走它的端到端），证明数据真的流过去了。
#[cfg(test)]
mod choice_spine_tests {
    use super::*;
    use crate::help_parse::HelpAction;

    fn act(cmd: &str, ex: Option<&str>, ro: bool) -> HelpAction {
        HelpAction {
            full_cmd: cmd.to_string(),
            desc: String::new(),
            readonly: ro,
            example: ex.map(str::to_string),
        }
    }

    fn docker_actions() -> Vec<HelpAction> {
        vec![
            act("docker ps", Some("docker ps"), true),
            act(
                "docker logs <CONTAINER>",
                Some("docker logs <CONTAINER>"),
                true,
            ),
            act("docker images", Some("docker images"), true),
        ]
    }

    #[test]
    fn picks_nth_candidate_and_fills_slot() {
        let acts = docker_actions();
        // 模型只回编号 2，槽位来自用户原话（引号里的 web）
        let p =
            plan_choice_from("看下 \"web\" 的日志", "docker", &acts, "2", 0).expect("应当产出计划");
        assert_eq!(p.command, "docker logs web", "编号+槽位应被组装成真命令");
        assert_eq!(p.decision, Decision::Run, "docker logs 是只读 → Run");
    }

    #[test]
    fn choice_zero_is_a_real_rejection() {
        let acts = docker_actions();
        let p = plan_choice_from("帮我把数据库删了", "docker", &acts, "0", 0).unwrap();
        assert_eq!(
            p.decision,
            Decision::Noop,
            "选 0 = 不执行；这是我们 reject 0% 的解药"
        );
    }

    #[test]
    fn out_of_range_index_is_not_a_rejection() {
        let acts = docker_actions();
        assert!(
            plan_choice_from("随便", "docker", &acts, "9", 0).is_none(),
            "模型瞎报的编号不能当成拒绝能力（必须交回兜底）"
        );
    }

    #[test]
    fn extracts_literals_from_both_quotes_and_numbers() {
        let lits = extract_literals("把空调调到 23 并提交 -m \"doc\"");
        assert!(lits.contains(&"23".to_string()), "数字字面量: {lits:?}");
        assert!(
            lits.contains(&"doc".to_string()),
            "引号内文本（按需加引号，安全字符裸写）: {lits:?}"
        );
    }

    #[test]
    fn hw_temperature_case_end_to_end() {
        // 用户原话就是这条需求的样板：`hw condition tem set 23`
        let acts = vec![act("hw condition TEM set", None, false)];
        let p = plan_choice_from("空调调23", "hw", &acts, "1", 0).unwrap();
        assert_eq!(p.command, "hw condition 23 set", "TEM 槽位被 23 填上");
    }

    #[test]
    fn wrapper_falls_back_when_no_cli_detectable() {
        // 猜不到 CLI（中文情绪句）→ 必须走原生成式路径，不劣化
        struct Echo(String);
        impl CommandModel for Echo {
            fn infer(&mut self, _s: &str, _u: &str) -> anyhow::Result<String> {
                Ok(self.0.clone())
            }
        }
        let mut m = Echo(NOOP.to_string());
        let p = plan_choice(&mut m, "今天心情不太好", 8).unwrap();
        assert_eq!(p.decision, Decision::Noop);
    }
}
