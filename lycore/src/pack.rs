//! LVK 知识包检索 — lyv.py lookup() 的 Rust 移植
//!
//! 检索策略 (与 Python 版一致):
//!   1. intent 词典正则命中 → 按 ocr_conf 取最优 segment
//!   2. FTS5 兜底 (OR 语义, 中文 bigram 分词)
//!   3. 无命中 → None (调用方入学习队列)

use crate::Evidence;
use rusqlite::Connection;
use std::path::Path;

/// intent 词典: (正则, intent) — 与 lyv.py RULES 同步
pub const RULES: &[(&str, &str)] = &[
    (r"(创建|新建|建立|create|new).{0,8}(项目|project)", "rust.project.create"),
    (r"(运行|跑|run).{0,8}(项目|project|程序)", "rust.project.run"),
    (r"进入|chdir|\bcd\b", "fs.chdir"),
];

/// seg_zh 风格分词: ASCII 连续段保留为一个词, CJK 逐字, 再加相邻 bigram
/// (与 lyv.tokens() 对齐 — 索引侧写入的是 bigram+word 混合流)
pub fn tokens(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut ascii_buf = String::new();
    let mut cjk: Vec<char> = Vec::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            if !cjk.is_empty() {
                cjk.clear();
            }
            ascii_buf.push(ch.to_ascii_lowercase());
        } else {
            if !ascii_buf.is_empty() {
                words.push(std::mem::take(&mut ascii_buf));
            }
            if ('\u{4e00}'..='\u{9fff}').contains(&ch) {
                cjk.push(ch);
            }
        }
    }
    if !ascii_buf.is_empty() {
        words.push(ascii_buf);
    }
    // CJK bigram: 保留相邻字对 (与索引侧一致)
    let zh: String = cjk.iter().collect();
    let chars: Vec<char> = zh.chars().collect();
    for w in chars.windows(2) {
        words.push(w.iter().collect());
    }
    words
}

pub struct Pack {
    conn: Connection,
    /// 领域规则 (rules.json 优先, 回退内置 RULES) — A1 领域配置化
    rules: Vec<Rule>,
}

/// 可配置领域规则: left/right 词对共现判定 (替换硬编码正则词典)
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Rule {
    pub left: Vec<String>,
    #[serde(default)]
    pub right: Vec<String>,
    pub intent: String,
}

impl Pack {
    /// 打开知识包 (pack/index/knowledge.sqlite)。
    /// 若 pack/rules.json 存在则加载为领域规则, 否则用内置 RULES。
    pub fn open(pack_dir: &Path) -> rusqlite::Result<Self> {
        let db_path = pack_dir.join("index").join("knowledge.sqlite");
        let conn = Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let rules = Self::load_rules(pack_dir);
        Ok(Self { conn, rules })
    }

    /// 规则来源优先级: rules.json > 内置 RULES
    fn load_rules(pack_dir: &Path) -> Vec<Rule> {
        let rules_json = pack_dir.join("rules.json");
        if let Ok(raw) = std::fs::read_to_string(&rules_json) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(list) = v.get("rules").and_then(|r| r.as_array()) {
                    let parsed: Vec<Rule> = list
                        .iter()
                        .filter_map(|r| serde_json::from_value(r.clone()).ok())
                        .collect();
                    if !parsed.is_empty() {
                        return parsed;
                    }
                }
            }
        }
        RULES
            .iter()
            .map(|(pat, intent)| parse_legacy_rule(pat, intent))
            .collect()
    }

    fn by_intent(&self, intent: &str) -> rusqlite::Result<Option<Evidence>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, t0, t1, text, intent, command, frame, ocr, ocr_conf, strong, weak
             FROM segments WHERE intent = ?1 ORDER BY ocr_conf DESC LIMIT 1",
        )?;
        let mut rows = stmt.query([intent])?;
        match rows.next()? {
            Some(row) => Ok(Some(Self::row_to_evidence(row, &format!("intent-dict:{intent}")))),
            None => Ok(None),
        }
    }

    fn by_fts(&self, query: &str) -> rusqlite::Result<Option<Evidence>> {
        let toks = tokens(query);
        if toks.is_empty() {
            return Ok(None);
        }
        let match_expr = toks.join(" OR ");
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.t0, s.t1, s.text, s.intent, s.command, s.frame, s.ocr,
                    s.ocr_conf, s.strong, s.weak
             FROM seg_fts f JOIN segments s ON s.id = f.id
             WHERE seg_fts MATCH ?1 ORDER BY rank LIMIT 1",
        )?;
        let mut rows = stmt.query([&match_expr])?;
        match rows.next()? {
            Some(row) => Ok(Some(Self::row_to_evidence(row, "fts-fallback"))),
            None => Ok(None),
        }
    }

    fn row_to_evidence(row: &rusqlite::Row<'_>, retrieval: &str) -> Evidence {
        let strong: String = row.get(9).unwrap_or_default();
        let weak: String = row.get(10).unwrap_or_default();
        Evidence {
            retrieval: retrieval.to_string(),
            intent: row.get(4).unwrap_or_default(),
            command: row.get(5).ok(),
            t0: row.get(1).unwrap_or(0.0),
            t1: row.get(2).unwrap_or(0.0),
            frame: row.get(6).unwrap_or_default(),
            frame_t: 0.0,
            text: row.get(3).unwrap_or_default(),
            strong: strong.split_whitespace().map(String::from).collect(),
            weak: weak.split_whitespace().map(String::from).collect(),
            prereq: Vec::new(),
            ocr_conf: row.get(8).unwrap_or(0.0),
        }
    }

    /// 混合检索: 领域规则 → FTS 兜底 → None
    pub fn lookup(&self, query: &str) -> rusqlite::Result<Option<Evidence>> {
        for rule in &self.rules {
            if rule_matches(rule, query) {
                if let Some(ev) = self.by_intent(&rule.intent)? {
                    return Ok(Some(ev));
                }
            }
        }
        self.by_fts(query)
    }
}

/// 规则匹配: left/right 词对共现 (顺序无关)。
/// right 为空的规则: 任一 left 词命中即可 (单侧 alternation 语义)。
fn rule_matches(rule: &Rule, q: &str) -> bool {
    let has_left = rule.left.iter().any(|l| q.contains(l.as_str()));
    if rule.right.is_empty() {
        return has_left;
    }
    has_left && rule.right.iter().any(|r| q.contains(r.as_str()))
}

/// 内置 RULES 的正则式词典 → Rule 词对 (一次性解析)
fn parse_legacy_rule(pat: &str, intent: &str) -> Rule {
    let (left, right) = split_alternations(pat);
    Rule {
        left: left.iter().map(|s| s.to_string()).collect(),
        right: right.iter().map(|s| s.to_string()).collect(),
        intent: intent.to_string(),
    }
}

/// 规则匹配: 每条规则独立判断 (对应 Python re.search(pat, q, re.I))
/// RULES 模式都是 "(词A|词B|wordC).{0,8}(词D|词E)" 形状 → 拆解为词对共现判定:
/// 两侧任选一词, 同时出现 (顺序无关近似, 距离限制由词典语义保证)
fn match_rule(pat: &str, q: &str) -> bool {
    // 粗解析: 按 | 拆 alternation, 取每侧的词字面量
    let words: Vec<&str> = pat.split('|')
        .flat_map(|alt| {
            // 去掉正则元字符残留 (.{0,8} 等), 只留 CJK/ASCII 词根
            alt.split(|c: char| !c.is_alphanumeric() && (c as u32) < 0x4e00)
                .filter(|s| !s.is_empty() && *s != "0" && *s != "8")
        })
        .collect();
    if words.len() < 2 {
        // 单词规则: 直接包含即命中 (如单侧 alternation 只剩一个有效词)
        let (left, right) = split_alternations(pat);
        if right.is_empty() {
            return left.iter().any(|l| q.contains(l));
        }
        return false;
    }
    // (A|B).{0,8}(C|D): 前半 alternation 与后半 alternation 各命中一词
    // 词典结构固定: 前侧 = 前 N1 个词, 后侧 = 剩余 (由 | 分组直接对应)
    let (left, right) = split_alternations(pat);
    if right.is_empty() {
        // 单侧 alternation (如 "进入|chdir|\bcd\b"): 任一词命中即可
        return left.iter().any(|l| q.contains(l));
    }
    left.iter().any(|l| q.contains(l)) && right.iter().any(|r| q.contains(r))
}

/// 把 "w1|w2.{0,8}w3|w4" 拆成 (前侧词, 后侧词)
fn split_alternations(pat: &str) -> (Vec<&str>, Vec<&str>) {
    // 找 '.{0,8}' 分隔
    let sep = ".{0,8}";
    let (l, r) = match pat.find(sep) {
        Some(i) => (&pat[..i], &pat[i + sep.len()..]),
        None => (pat, ""),
    };
    fn parse(s: &str) -> Vec<&str> {
        s.split('|')
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && (c as u32) < 0x4e00))
            .filter(|w| !w.is_empty())
            .collect()
    }
    (parse(l), parse(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_rule_parsing_and_matching() {
        let run = parse_legacy_rule(RULES[1].0, RULES[1].1);
        assert!(rule_matches(&run, "怎么运行项目"));
        let create = parse_legacy_rule(RULES[0].0, RULES[0].1);
        assert!(!rule_matches(&create, "怎么运行项目")); // create 规则不应命中
        assert!(rule_matches(&create, "怎么新建项目"));
    }

    #[test]
    fn configured_rules_load_from_rules_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("index")).unwrap();
        // 空库 — 只为测试规则加载
        let _ = rusqlite::Connection::open(dir.path().join("index/knowledge.sqlite"))
            .unwrap()
            .execute_batch("CREATE TABLE segments(id TEXT PRIMARY KEY);");
        std::fs::write(
            dir.path().join("rules.json"),
            r#"{"domain":"test","rules":[{"left":["画画"],"right":[],"intent":"art.draw"}]}"#,
        )
        .unwrap();
        let pack = Pack::open(dir.path()).unwrap();
        assert_eq!(pack.rules.len(), 1);
        assert_eq!(pack.rules[0].intent, "art.draw");
    }
}
