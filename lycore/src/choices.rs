//! 判定式脊（Choice Spine）：把「生成一条命令」改成「从真实候选里挑一个 + 填槽位」。
//!
//! ## 为什么这么做（证据，不是偏好）
//!
//! 1. **同一任务定义、同一批题目，判定式 vs 生成式差 58pp**：
//!    云端 Jev（只做「从 N 个选项选 1」，不出命令）跨域 CLI 选择 **12/12 = 100%**；
//!    我们自研路由器（让模型**生成完整命令串**）同池 **acc_exec = 41.7%**
//!    （归档 `p2-lyco_ops/cloudstudio/jev_probe/REPORT.md`）。
//! 2. **出域拒绝**：Jev 10/10 = 100%，我们 **0%** —— 而「给不出的选项就选空」在判定式里是自然动作。
//! 3. 第三方同向：Cactus Needle 3 的设计是 **argument must be grounded** + grammar 约束，
//!    且「无匹配 tool 时输出空列表」；工具幻觉论文则说明**规模不解决幻觉**（675B 同样会造不存在的 tool）。
//!
//! ## 分工（这是本模块的全部意义）
//!
//! ```text
//! 候选生成（确定性，本地）：help_parse 的真实动作表 → 编号候选
//! 判定（模型，唯一需要模型的环节）：只回「编号 + 槽位」
//! 组装（确定性，本地）：编号 + 槽位 → 命令串；槽位填不满 → 回问用户（不猜）
//! ```
//!
//! 也就是说：**CLI 本身就是天然的选项集**，模型不需要"记住"任何 CLI，
//! 它只需要在给定候选里做一次选择——这正是我们跨域 41.7% 的解药。

use crate::help_parse::HelpAction;

/// 一个带编号的候选（编号从 1 开始，与模型输出对齐）
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Choice {
    pub idx: usize,
    pub full_cmd: String,
    pub example: Option<String>,
    pub readonly: bool,
}

/// 把真实动作表渲染成**编号候选**。
///
/// 与 `render_schema` 的区别是本质的：那个是「给你命令清单，你去对齐」，
/// 这个是「给你带编号的选项，你只回编号」。后者才是判定式。
///
/// `max_n` 为 0 表示不截断（v18 实测：截断是净损失）。
pub fn build_choices(actions: &[HelpAction], max_n: usize) -> Vec<Choice> {
    let n = if max_n == 0 {
        actions.len()
    } else {
        max_n.min(actions.len())
    };
    actions
        .iter()
        .take(n)
        .enumerate()
        .map(|(i, a)| Choice {
            idx: i + 1,
            full_cmd: a.full_cmd.clone(),
            example: a.example.clone(),
            readonly: a.readonly,
        })
        .collect()
}

/// 渲染成给模型看的提示块。**只输出编号 + 命令 + 示例**，并要求回编号。
pub fn render_choices(cli: &str, choices: &[Choice]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "【{cli} 候选动作】（只回编号；一个都不合适就回 0；不要输出命令本身）\n"
    ));
    for c in choices {
        s.push_str(&format!("{}. {}", c.idx, c.full_cmd));
        if let Some(e) = &c.example {
            s.push_str(&format!("   例: {e}"));
        }
        s.push('\n');
    }
    s.push_str("0. 以上都不合适（不执行）\n");
    s
}

/// 模型判定的结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    /// 选中第 idx 个候选（1-based，已校验在范围内）
    Idx(usize),
    /// 模型明确选了 0 / 说了"都不合适" → 不执行（这正是我们缺的拒绝能力）
    None,
    /// 输出里找不到合法编号 → 调用方应视为解析失败（**不许瞎猜**）
    Unparsable,
}

/// 从模型输出里**确定性**地取编号。
///
/// 规则（窄而严，宁可不解析也不猜）：
/// * 只看第一行/首个出现的整数；
/// * 超范围 → `Unparsable`（不是 `None`！模型瞎报的号不算"拒绝"）；
/// * `0` → `None`。
pub fn parse_picked(text: &str, n_choices: usize) -> Picked {
    let mut num: Option<usize> = None;
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            cur.push(ch);
        } else if !cur.is_empty() {
            num = cur.parse::<usize>().ok();
            break;
        }
    }
    if num.is_none() {
        num = cur.parse::<usize>().ok();
    }
    match num {
        None => Picked::Unparsable,
        Some(0) => Picked::None,
        Some(k) if k <= n_choices => Picked::Idx(k),
        Some(_) => Picked::Unparsable,
    }
}

/// 人话与一条候选的匹配分（0.0–1.0）。**确定性，不含模型。**
///
/// ## 它是"范围缩小器"，不是"判定器"
///
/// 判据很朴素：人话里有多少字符出现在「命令 + 中文说明」里。
/// 对"关灯"它能把 9 条候选缩到 4 条（都含"灯"字），但**区分不了 on/off** ——
/// 那是语义，该留给模型或人。
///
/// 刻意不做"更聪明"的启发式：
///   * 中文没有空格分词，任何"关键词切分"都是猜；
///   * 猜错的排序比不排序更危险 —— 它会把错误答案排到第一位，而用户一眼就信了。
///
/// 宁可并列，也不假装能分。
pub fn score_action(nl: &str, cmd: &str, desc_zh: &str) -> f64 {
    let hay: Vec<char> = format!("{cmd} {desc_zh}").chars().collect();
    let want: Vec<char> = nl.chars().filter(|c| !c.is_whitespace()).collect();
    if want.is_empty() {
        return 0.0;
    }
    let hit = want.iter().filter(|c| hay.contains(c)).count();
    hit as f64 / want.len() as f64
}

/// 只在**必要时**给槽位值加引号（shell 安全），避免把 `web` 变成 `"web"`。
///
/// 判据：全字符都在安全集内（字母数字与 `_ - . / @ + : , =`）→ 裸写；
/// 否则用双引号包裹，并转义内层 `"` 与 `\`。
pub fn shell_quote_if_needed(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '-' | '.' | '/' | '@' | '+' | ':' | ',' | '=')
        });
    if safe {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// 组装结果：要么给出可执行命令，要么明确"还缺什么"去回问用户。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assembled {
    Ready(String),
    /// 示例里仍有未填占位符 → 这些槽位要回问用户（**绝不猜值**）
    NeedsSlots {
        cmd: String,
        missing: Vec<String>,
    },
}

/// 从示例动作出发，用给定槽位值填占位符（确定性，无猜测）。
///
/// 占位符识别：`<...>`、`{...}`、`[...]`、以及全大写 token（如 `URL`/`NAME`）。
/// 填值顺序 = 槽位值出现顺序（与候选示例里的占位符顺序一一对应）。
///
/// 没有示例时：退回 `full_cmd`；若它本身含占位符 → 同样走 `NeedsSlots`。
pub fn assemble(action: &HelpAction, slots: &[String]) -> Assembled {
    let tpl = action
        .example
        .clone()
        .unwrap_or_else(|| action.full_cmd.clone());
    let ph = find_placeholders(&tpl);
    if ph.is_empty() {
        // 无占位符：把槽位值按顺序追加（例如 `git commit -m "doc"` 的 "doc"）
        if slots.is_empty() {
            return Assembled::Ready(tpl);
        }
        return Assembled::Ready(format!("{} {}", tpl, slots.join(" ")));
    }
    let mut out = tpl.clone();
    let mut missing = Vec::new();
    for (i, p) in ph.iter().enumerate() {
        match slots.get(i) {
            Some(v) => out = out.replacen(p, v, 1),
            None => missing.push(p.clone()),
        }
    }
    if missing.is_empty() {
        Assembled::Ready(out)
    } else {
        Assembled::NeedsSlots { cmd: out, missing }
    }
}

/// 找出模板里的占位符（保序、去重前的原样）
pub fn find_placeholders(tpl: &str) -> Vec<String> {
    let mut ph = Vec::new();
    let bytes: Vec<char> = tpl.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let close = match c {
            '<' => Some('>'),
            '{' => Some('}'),
            '[' => Some(']'),
            _ => None,
        };
        if let Some(cl) = close {
            if let Some(j) = (i + 1..bytes.len()).find(|&k| bytes[k] == cl) {
                ph.push(bytes[i..=j].iter().collect::<String>());
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    // 全大写 token（如 `URL`）——但只在没有尖括号占位符时启用，避免把命令名当占位符
    if ph.is_empty() {
        for tok in tpl.split_whitespace() {
            let t = tok.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if t.len() >= 2
                && t.chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
                && t.chars().any(|c| c.is_ascii_uppercase())
            {
                ph.push(t.to_string());
            }
        }
    }
    ph
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
    fn choices_are_numbered_from_one_and_keep_examples() {
        let acts = vec![
            act("docker ps", Some("docker ps"), true),
            act("docker logs <CONTAINER>", Some("docker logs web"), true),
        ];
        let cs = build_choices(&acts, 0);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].idx, 1);
        assert_eq!(cs[1].idx, 2);
        assert_eq!(cs[1].example.as_deref(), Some("docker logs web"));
        let txt = render_choices("docker", &cs);
        assert!(txt.contains("1. docker ps"));
        assert!(txt.contains("0. 以上都不合适"));
    }

    #[test]
    fn max_n_truncates_but_zero_means_no_truncation() {
        let acts: Vec<HelpAction> = (0..7).map(|i| act(&format!("x {i}"), None, true)).collect();
        assert_eq!(build_choices(&acts, 3).len(), 3);
        assert_eq!(build_choices(&acts, 0).len(), 7);
    }

    #[test]
    fn parse_picked_accepts_only_in_range_numbers() {
        assert_eq!(parse_picked("2", 5), Picked::Idx(2));
        assert_eq!(parse_picked("我选第 3 个", 5), Picked::Idx(3));
        assert_eq!(parse_picked("0", 5), Picked::None);
        // 超范围 = 解析失败，**不能**当成"拒绝"（模型瞎报的号不是拒绝能力）
        assert_eq!(parse_picked("9", 5), Picked::Unparsable);
        assert_eq!(parse_picked("都不合适", 5), Picked::Unparsable);
    }

    #[test]
    fn assemble_fills_placeholders_in_order() {
        let a = act(
            "docker logs <CONTAINER>",
            Some("docker logs <CONTAINER>"),
            true,
        );
        assert_eq!(
            assemble(&a, &["web".to_string()]),
            Assembled::Ready("docker logs web".to_string())
        );
    }

    #[test]
    fn assemble_reports_missing_slots_instead_of_guessing() {
        let a = act(
            "git commit -m <MSG> <PATH>",
            Some("git commit -m <MSG> <PATH>"),
            false,
        );
        match assemble(&a, &["\"doc\"".to_string()]) {
            Assembled::NeedsSlots { missing, .. } => {
                assert_eq!(missing, vec!["<PATH>".to_string()])
            }
            other => panic!("应当报缺槽位，实际 {other:?}"),
        }
    }

    #[test]
    fn assemble_appends_literal_slots_when_template_has_no_placeholder() {
        // 用户原话里的字面量（-m "doc"）在模板无占位符时按序追加
        let a = act("git commit", Some("git commit"), false);
        assert_eq!(
            assemble(&a, &["-m".to_string(), "\"doc\"".to_string()]),
            Assembled::Ready("git commit -m \"doc\"".to_string())
        );
    }

    #[test]
    fn uppercase_token_is_placeholder_when_no_brackets() {
        let ph = find_placeholders("hw condition TEM set");
        assert_eq!(ph, vec!["TEM".to_string()]);
        let a = act("hw condition TEM set", None, false);
        assert_eq!(
            assemble(&a, &["23".to_string()]),
            Assembled::Ready("hw condition 23 set".to_string())
        );
    }

    #[test]
    fn score_narrows_candidates_and_can_separate_off_from_on() {
        use crate::choices::score_action;
        // "关灯" 两字都出现在 led 相关的说明里 → 满分
        let led = score_action("关灯", "hw led blue off", "关闭 用户蓝灯");
        // temp 的说明里既没有"关"也没有"灯" → 0
        let temp = score_action("关灯", "hw temp", "CPU 温度");
        assert!(
            led > temp,
            "led 相关必须排在 temp 前面: led={led} temp={temp}"
        );
        assert_eq!(temp, 0.0);

        // 🎯 分得出 off / on —— 但这**不是字符串算法的功劳**，
        //    是 `expand_desc_branches` 把分支词语义补进了说明：
        //    「关灯」的「关」命中「关闭」，不命中「打开」。
        //    反过来说：没有那张 BRANCH_VERBS 词典，这里就只能并列。
        let on = score_action("关灯", "hw led blue on", "打开 用户蓝灯");
        assert!(led > on, "「关灯」应更匹配 off: off={led} on={on}");
    }

    #[test]
    fn score_is_zero_for_empty_input_and_full_for_exact_cover() {
        use crate::choices::score_action;
        assert_eq!(score_action("", "hw temp", "CPU 温度"), 0.0);
        assert_eq!(score_action("CPU", "hw temp", "CPU 温度"), 1.0);
    }

    #[test]
    fn no_placeholder_no_slots_is_ready_asis() {
        let a = act("docker ps", Some("docker ps"), true);
        assert_eq!(assemble(&a, &[]), Assembled::Ready("docker ps".to_string()));
    }
}

#[cfg(test)]
mod quoting_tests {
    use super::shell_quote_if_needed;

    #[test]
    fn bare_when_safe() {
        assert_eq!(shell_quote_if_needed("web"), "web");
        assert_eq!(shell_quote_if_needed("23"), "23");
        assert_eq!(
            shell_quote_if_needed("/var/log/app.log"),
            "/var/log/app.log"
        );
    }

    #[test]
    fn quoted_when_whitespace_or_specials() {
        assert_eq!(shell_quote_if_needed("add doc"), "\"add doc\"");
        assert_eq!(shell_quote_if_needed("a;rm -rf /"), "\"a;rm -rf /\"");
        assert_eq!(
            shell_quote_if_needed("he said \"hi\""),
            "\"he said \\\"hi\\\"\""
        );
    }

    #[test]
    fn empty_becomes_empty_quotes_not_bare() {
        assert_eq!(shell_quote_if_needed(""), "\"\"");
    }
}
