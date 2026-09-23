//! lbrush —— **人机闭环采集器**（人先当模型，判定式脊的第一个真实调用者）。
//!
//! ## 为什么需要它（这是产品缺口，不是工程洁癖）
//!
//! 2026-09-23 的接线审计发现三件事同时成立：
//!
//! 1. `router::plan` / `plan_with_help` —— **全仓零产品调用者**。所有跨域 41.7% / 85.0% 的分数
//!    都来自评测脚本，产品里**从来没有一条路径**走过「新 CLI → help → 判定 → 执行」。
//! 2. `choices.rs`（判定式脊）—— 唯一调用者是它自己的测试 ⇒ 孤岛。
//! 3. `t1gate::classify` —— 唯一真实消费者是 `lycore t1 check`（**自己打印风险等级**），
//!    而 `Executor::execute("shell_exec")` 与 `lycore run` 都不过门。
//!
//! 更要紧的是：我们从来没有**自己的数据**。v13 的训练语料由 `datagen.rs` 合成，
//! 里面装的是「我们的想象」（含我们的错误），没有一条带**真实执行反馈**。
//!
//! ## 这个模块要解决什么
//!
//! 把「模型」这一环换成**人**，其余全部复用已有的确定性代码：
//!
//! ```text
//! 人话 ──→ guess_cli → grab_help → parse_help          （确定性，已有）
//!      ──→ build_choices → 编号候选                     （确定性，已有 choices.rs）
//!      ──→ 【人 选编号】← 未来这里换成 0.6B / Needle       （唯一需要智能的一环）
//!      ──→ find_placeholders → 【人 填槽位】              （不猜，填不满就问）
//!      ──→ assemble                                     （确定性，已有）
//!      ──→ T1 门 + 缺参检查                              （确定性，已有 t1gate/paramcheck）
//!      ──→ 执行 → **真实 exit_code**                     （这是我们所有训练数据都缺的信号）
//!      ──→ ndjson 落盘 = 金标准语料
//! ```
//!
//! 于是**同一份代码**同时是：
//! * **产品本体** —— 人现在就能用（不依赖任何模型质量）；
//! * **采集器** —— 每次成功 = 一条人确认过的 `(nl, cmd, exit_code)`；
//! * **训练素材源** —— 将来把 `judge` 从 `human` 换成 `model`，就是评测集。
//!
//! 判据（可测）：`lycore do` 跑过一次真实调用后，lbrush 文件里必须出现一条
//! `exit_code == 0` 且 `judge == "human"` 的记录。

use crate::choices::{assemble, Assembled};
use crate::help_parse::HelpAction;
use crate::paramcheck;
use crate::t1gate::{self, Risk};
use std::io::Write;
use std::path::{Path, PathBuf};

/// 平台的分诊决策 —— 与 `router::Decision` **同序**（危险优先 → 缺参 → 写确认 → 只读）。
///
/// 顺序必须一致，否则「评测脚本里能过、平台里过不了」会再次出现。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    /// 只读 —— 直接执行
    Run,
    /// 写操作 —— 需确认
    Confirm,
    /// 破坏性 —— 拦截（需显式 override）
    Block,
    /// 缺槽位 / 缺必需参数 —— 不执行（空跑会静默失效）
    NeedsParam,
}

impl GateDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            GateDecision::Run => "run",
            GateDecision::Confirm => "confirm",
            GateDecision::Block => "block",
            GateDecision::NeedsParam => "needs_param",
        }
    }
}

/// 人（或未来的模型）选定之后、执行之前的完整计划。**纯数据，可序列化，可测。**
#[derive(Debug, Clone, serde::Serialize)]
pub struct HumanPlan {
    pub cli: String,
    /// 候选模板（help 里的动作原文）
    pub action: String,
    /// 组装后的完整命令
    pub cmd: String,
    /// 模板里填不满的槽位（占位符原文）
    pub missing: Vec<String>,
    pub risk: String,
    pub matched: Option<String>,
    pub reason: String,
    pub decision: String,
    pub param_status: String,
    pub param_missing: Vec<String>,
}

/// 纯函数：候选 + 人选 + 槽位值 → 计划。**不做任何 IO，不执行任何命令。**
///
/// 分诊顺序与 `bin/lycore.rs::cmd_t1` 逐条对齐：
/// `模板缺槽 → 危险 → 缺必需参数 → 写确认 → 只读`。
pub fn plan_human(cli: &str, action: &HelpAction, slots: &[String]) -> HumanPlan {
    let (cmd, missing) = match assemble(action, slots) {
        Assembled::Ready(c) => (c, Vec::new()),
        Assembled::NeedsSlots { cmd, missing } => (cmd, missing),
    };

    let v = t1gate::classify(&cmd);
    let p = paramcheck::check(&cmd);

    // 组合命令逐段查缺参（与 `t1gate::classify` 的拆分口径一致）
    let segs: Vec<&str> = v
        .command
        .split(|c| c == ';' || c == '|')
        .flat_map(|x| x.split("&&"))
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .collect();
    let params_ok = segs
        .iter()
        .all(|s| !paramcheck::check(s).status.is_needs_param());

    let decision = if !missing.is_empty() {
        GateDecision::NeedsParam
    } else if matches!(v.risk, Risk::Danger) {
        GateDecision::Block
    } else if !params_ok {
        GateDecision::NeedsParam
    } else if matches!(v.risk, Risk::Write) {
        GateDecision::Confirm
    } else {
        GateDecision::Run
    };

    HumanPlan {
        cli: cli.to_string(),
        action: action.full_cmd.clone(),
        cmd: v.command.clone(),
        missing,
        risk: v.risk.as_str().to_string(),
        matched: v.matched.clone(),
        reason: v.reason.clone(),
        decision: decision.as_str().to_string(),
        param_status: p.status.as_str().to_string(),
        param_missing: p.missing.clone(),
    }
}

/// 执行前的放行判定（与 `router::may_execute` 同语义）。
pub fn may_execute(decision: GateDecision, confirmed: bool, danger_override: bool) -> bool {
    match decision {
        GateDecision::Run => true,
        GateDecision::Confirm => confirmed,
        GateDecision::Block => confirmed && danger_override,
        GateDecision::NeedsParam => false,
    }
}

/// 一条采集记录 —— **训练素材的最小单元**。
///
/// 与文档里 `lbrush`（NL→cmd→exit_code）的口径一致，另加三项归因必需字段：
/// `risk`（哪一档分诊）、`decision`（过没过）、`judge`（谁判的）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct LbrushRecord {
    /// unix 秒（UTC）
    pub ts: u64,
    /// 用户原话
    pub nl: String,
    pub cli: String,
    /// 人选编号（**0 = 一个都不合适**，这是"拒绝"的正样本）
    pub idx: usize,
    /// 候选模板原文
    pub action: String,
    /// 最终命令
    pub cmd: String,
    /// 人填的槽位值
    pub slots: Vec<String>,
    pub risk: String,
    pub decision: String,
    /// 真实退出码（`None` = 没执行 / 超时 / 被信号终止）
    pub exit_code: Option<i32>,
    /// 是否真的执行了（`false` 含 dry-run 与各类拦截）
    pub executed: bool,
    /// 判定者：现在恒为 `human`；将来接 0.6B / Needle 时改为 `model`
    pub judge: String,
}

impl LbrushRecord {
    pub fn new(nl: &str, cli: &str, idx: usize, plan: &HumanPlan, slots: &[String]) -> Self {
        Self {
            ts: now_unix(),
            nl: nl.to_string(),
            cli: cli.to_string(),
            idx,
            action: plan.action.clone(),
            cmd: plan.cmd.clone(),
            slots: slots.to_vec(),
            risk: plan.risk.clone(),
            decision: plan.decision.clone(),
            exit_code: None,
            executed: false,
            judge: "human".to_string(),
        }
    }
}

/// 采集目录：`$LYCO_LBRUSH_DIR`，否则 `~/.lbrush`
pub fn record_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("LYCO_LBRUSH_DIR") {
        return PathBuf::from(d);
    }
    home_dir().join(".lbrush")
}

/// 当日记录文件：`~/.lbrush/YYYY-MM-DD.ndjson`（按北京时间分日，UTC+8）
pub fn default_record_path() -> PathBuf {
    record_dir().join(format!("{}.ndjson", today_ymd_cn()))
}

/// 追加一条记录（**append-only**，绝不改写历史）。
pub fn append(rec: &LbrushRecord, path: &Path) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        if !p.as_os_str().is_empty() {
            std::fs::create_dir_all(p)?;
        }
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(rec)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    writeln!(f, "{line}")?;
    Ok(())
}

fn home_dir() -> PathBuf {
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h);
    }
    if let Some(u) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(u);
    }
    PathBuf::from(".")
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 今天的日期（北京时间，UTC+8）—— 端侧不引入 chrono，用 Hinnant 的 civil_from_days。
pub fn today_ymd_cn() -> String {
    let (y, m, d) = ymd_from_unix(now_unix() + 8 * 3600);
    format!("{y:04}-{m:02}-{d:02}")
}

/// unix 秒 → (年, 月, 日)。Howard Hinnant `civil_from_days`，纯整数运算。
fn ymd_from_unix(secs: u64) -> (i64, u32, u32) {
    let days = (secs / 86400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act(cmd: &str, ex: Option<&str>, ro: bool) -> HelpAction {
        HelpAction {
            full_cmd: cmd.to_string(),
            desc: String::new(),
            readonly: ro,
            example: ex.map(str::to_string),
        }
    }

    #[test]
    fn readonly_choice_runs_without_confirmation() {
        let a = act("docker ps", Some("docker ps"), true);
        let p = plan_human("docker", &a, &[]);
        assert_eq!(p.cmd, "docker ps");
        assert_eq!(p.decision, "run");
        assert!(may_execute(GateDecision::Run, false, false));
    }

    #[test]
    fn destructive_choice_is_blocked_even_when_confirmed_without_override() {
        // `docker rm web` —— 真实平台上人选完之后必须被拦
        let a = act(
            "docker rm <CONTAINER>",
            Some("docker rm <CONTAINER>"),
            false,
        );
        let p = plan_human("docker", &a, &["web".to_string()]);
        assert_eq!(p.cmd, "docker rm web");
        assert_eq!(p.decision, "block", "破坏性命令必须是 block");
        assert!(
            !may_execute(GateDecision::Block, true, false),
            "仅确认不足以放行"
        );
        assert!(
            may_execute(GateDecision::Block, true, true),
            "确认+显式 override 才放行"
        );
    }

    #[test]
    fn unfilled_template_slot_is_needs_param_not_a_guess() {
        // 模板有 <CONTAINER> 但人没填 → 绝不猜，直接 NeedsParam
        let a = act(
            "docker logs <CONTAINER>",
            Some("docker logs <CONTAINER>"),
            true,
        );
        let p = plan_human("docker", &a, &[]);
        assert_eq!(p.decision, "needs_param");
        assert_eq!(p.missing, vec!["<CONTAINER>".to_string()]);
        assert!(
            !may_execute(GateDecision::NeedsParam, true, true),
            "缺槽位连 override 都不放行"
        );
    }

    #[test]
    fn empty_required_param_is_needs_param() {
        // 模板无占位符但命令本身缺必需参数（`npm install` 没说装什么）
        let a = act("npm install", Some("npm install"), false);
        let p = plan_human("npm", &a, &[]);
        assert_eq!(p.decision, "needs_param");
        assert!(
            p.param_status.contains("needs_param")
                || p.param_missing.contains(&"<pkg>".to_string())
        );
    }

    #[test]
    fn write_choice_needs_confirmation() {
        let a = act("git commit -m <MSG>", Some("git commit -m <MSG>"), false);
        let p = plan_human("git", &a, &["\"doc\"".to_string()]);
        assert_eq!(p.cmd, "git commit -m \"doc\"");
        assert_eq!(p.decision, "confirm");
        assert!(!may_execute(GateDecision::Confirm, false, false));
        assert!(may_execute(GateDecision::Confirm, true, false));
    }

    #[test]
    fn hw_condition_end_to_end_sample() {
        // 用户给的原话样板：`空调调23` + 动作 `hw condition TEM set`
        let a = act("hw condition TEM set", None, false);
        let p = plan_human("hw", &a, &["23".to_string()]);
        assert_eq!(p.cmd, "hw condition 23 set");
    }

    #[test]
    fn record_serializes_with_all_attribution_fields() {
        let a = act("docker ps", Some("docker ps"), true);
        let p = plan_human("docker", &a, &[]);
        let mut r = LbrushRecord::new("看看有哪些容器在跑", "docker", 1, &p, &[]);
        r.executed = true;
        r.exit_code = Some(0);
        let j: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(j["nl"], "看看有哪些容器在跑");
        assert_eq!(j["exit_code"], 0);
        assert_eq!(j["judge"], "human");
        assert_eq!(j["decision"], "run");
        assert_eq!(j["idx"], 1);
    }

    #[test]
    fn rejection_is_recordable_with_idx_zero() {
        // idx=0（一个候选都不合适）必须能落盘 —— 这是我们唯一能拿到"拒绝"正样本的途径
        let a = act("docker ps", Some("docker ps"), true);
        let p = plan_human("docker", &a, &[]);
        let r = LbrushRecord::new("帮我把服务器装进冰箱", "docker", 0, &p, &[]);
        assert_eq!(r.idx, 0);
        assert!(!r.executed);
        assert_eq!(r.exit_code, None);
    }

    #[test]
    fn ymd_conversion_matches_known_dates() {
        // 2026-09-23 00:00:00 UTC = 1790121600（`date -u -d '2026-09-23 00:00:00' +%s` 实算）
        assert_eq!(ymd_from_unix(1_790_121_600), (2026, 9, 23));
        // 1970-01-01 = 0
        assert_eq!(ymd_from_unix(0), (1970, 1, 1));
    }

    #[test]
    fn record_path_is_ndjson_under_record_dir() {
        std::env::set_var("LYCO_LBRUSH_DIR", "/tmp/lbrush-test");
        let p = default_record_path();
        std::env::remove_var("LYCO_LBRUSH_DIR");
        assert_eq!(p.parent().unwrap().to_string_lossy(), "/tmp/lbrush-test");
        assert!(p.to_string_lossy().ends_with(".ndjson"));
    }
}
