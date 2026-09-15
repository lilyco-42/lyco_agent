//! learn_cli — Rust 原生 CLI 手册索引 (零 Python 依赖)
//!
//! 对 Python cli_indexer.py 的 Rust 移植:
//!   tool --help → 解析 Commands 段 → 逐子命令 help → 知识包 build
//!   lilyco 应用: --schema JSON 直接入库
//!
//! 输出与 learn.rs build_cues 同 schema (sqlite + FTS5), 共享 pack 基础设施。

use crate::learn::Cue;
use std::path::Path;

/// 解析 --help 的命令条目, 兼容 clap / adb 同行式 / adb 换行式三种格式。
/// 段落: 顶格且以 ':' 结尾 = 段标题; 含 option/usage/variable/example/argument/
/// shortcut/status 的标题视为非命令段 (排除选项表、按键表、退出码、环境变量)。
/// 命令两种排版:
///   A 同行式 "devices [-l]   list connected..." → 列分隔 (≥2 空格或 tab) 前为命令;
///   B 换行式 "shell [-e] [COMMAND...]" + 下一行更深缩进的描述 → 取下一行为描述。
/// 命令名须字母开头 (排除退出码 1 / 按键 MOD+q / 选项 -a)。
/// B 的"更深缩进"要求天然排除 prose 续行 (同缩进单空格, 如 scrcpy Shortcuts 说明)。
pub fn parse_commands(help_text: &str) -> Vec<(String, String)> {
    let mut commands = Vec::new();
    let mut in_section = false;
    const META: &[&str] = &[
        "option", "usage", "variable", "example", "argument", "shortcut", "status",
    ];
    let lines: Vec<&str> = help_text.lines().collect();
    let mut skip_next = false;
    for (idx, raw) in lines.iter().enumerate() {
        let line = *raw;
        if skip_next {
            skip_next = false;
            continue;
        }
        let is_header = !line.starts_with(char::is_whitespace)
            && !line.trim().is_empty()
            && line.trim_end().ends_with(':');
        if is_header {
            let h = line.to_lowercase();
            in_section = !META.iter().any(|m| h.contains(m));
            continue;
        }
        if !in_section || !line.starts_with(char::is_whitespace) {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let first_tok = trimmed.split_whitespace().next().unwrap_or("");
        let name = first_tok.trim_end_matches(',');
        if name.is_empty()
            || !name
                .chars()
                .next()
                .map(|c| c.is_ascii_alphabetic())
                .unwrap_or(false)
        {
            continue; // 数字/符号开头: 退出码、按键 (MOD+q)、选项 (-a)
        }
        // A: 同行列对齐
        if let Some((cmd_field, desc)) = split_first_column(trimmed) {
            let _ = cmd_field;
            if !desc.is_empty() {
                commands.push((name.to_string(), desc.to_string()));
            }
            continue;
        }
        // B: 换行式 —— 下一行缩进更深视为该命令的描述
        if let Some(next) = lines.get(idx + 1) {
            let cur_indent = line.len() - line.trim_start().len();
            let nindent = next.len() - next.trim_start().len();
            let ntrim = next.trim();
            if !ntrim.is_empty() && nindent > cur_indent {
                commands.push((name.to_string(), ntrim.to_string()));
                skip_next = true;
            }
        }
    }
    commands
}

/// 在行内找第一个 ≥2 连续空格 (或 tab) 作为列分隔, 返回 (命令字段, 描述)。
/// 找不到 (纯单空格 prose) → None。空格为 ASCII, 字节切片安全。
fn split_first_column(line: &str) -> Option<(&str, &str)> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\t' => return Some((line[..i].trim_end(), line[i + 1..].trim_start())),
            b' ' => {
                let mut j = i;
                while j < b.len() && b[j] == b' ' {
                    j += 1;
                }
                if j - i >= 2 {
                    return Some((line[..i].trim_end(), line[j..].trim_start()));
                }
                i = j;
            }
            _ => i += 1,
        }
    }
    None
}

/// 执行 tool --help 获取全文
fn run_help(tool: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(tool).args(args).output().ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stdout.trim().is_empty() {
        Some(stderr.to_string())
    } else {
        Some(stdout.to_string())
    }
}

/// 索引 CLI 工具: --help → 子命令 → help 全文 → Cue 列表
pub fn index_tool(tool: &str) -> anyhow::Result<Vec<Cue>> {
    let help_text = run_help(tool, &["--help"])
        .ok_or_else(|| anyhow::anyhow!("{} --help 执行失败", tool))?;

    let mut cues = vec![Cue {
        t0: 0.0,
        t1: 0.0,
        text: format!("{} : 命令行工具 : 用法见帮助", tool),
    }];

    let commands = parse_commands(&help_text);
    for (sub, desc) in commands.iter().take(60) {
        let full_help = run_help(tool, &["help", sub]).unwrap_or_default();
        let snippet: String = full_help.lines().take(1).collect();
        cues.push(Cue {
            t0: 0.0,
            t1: 0.0,
            text: format!("{} {} : {} : 用法: {}", tool, sub, desc, snippet),
        });
    }
    Ok(cues)
}

/// 索引 lilyco 应用: --schema JSON → Cue 列表
pub fn index_lilyco_schema(schema_exe: &str) -> anyhow::Result<Vec<Cue>> {
    let out = std::process::Command::new(schema_exe)
        .arg("--schema")
        .output()
        .map_err(|e| anyhow::anyhow!("{} --schema 失败: {}", schema_exe, e))?;
    anyhow::ensure!(out.status.success(), "--schema 执行失败");
    let schema: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let name = schema["name"].as_str().unwrap_or("unknown").to_lowercase();
    let about = schema["about"].as_str().unwrap_or("");

    let mut cues = vec![Cue {
        t0: 0.0,
        t1: 0.0,
        text: format!("{} : {} : 用法见帮助", name, about),
    }];
    for arg in schema["args"].as_array().unwrap_or(&vec![]) {
        let aname = arg["name"].as_str().unwrap_or("");
        let aabout = arg["about"].as_str().unwrap_or("");
        let req = if arg["required"].as_bool().unwrap_or(false) { "必填" } else { "可选" };
        cues.push(Cue {
            t0: 0.0,
            t1: 0.0,
            text: format!("{} {} : 参数 {} ({}): {} : 用法见帮助", name, aname, aname, req, aabout),
        });
    }
    Ok(cues)
}

/// 构建知识包 (轻量版, 不抽帧 — CLI 工具无视频帧)
pub fn build_cli_pack(cues: &[Cue], pack_dir: &Path) -> anyhow::Result<usize> {
    use rusqlite::Connection;
    for d in ["knowledge", "index"] {
        std::fs::create_dir_all(pack_dir.join(d))?;
    }

    let mut units = Vec::new();
    for (i, cue) in cues.iter().enumerate() {
        let uid = format!("u{:03}", i);
        units.push(serde_json::json!({
            "id": uid, "t0": cue.t0, "t1": cue.t1, "text": cue.text,
            "frame": "", "frame_t": 0.0,
            "ocr": "", "ocr_conf": 1.0,
            "strong": [], "weak": [],
            "intent": "", "command": "",
        }));
    }

    let mut jsonl = String::new();
    for u in &units {
        jsonl.push_str(&serde_json::to_string(u)?);
        jsonl.push('\n');
    }
    std::fs::write(pack_dir.join("knowledge/segments.jsonl"), jsonl)?;

    let db = Connection::open(pack_dir.join("index/knowledge.sqlite"))?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS segments(id TEXT PRIMARY KEY, t0 REAL, t1 REAL,
         text TEXT, intent TEXT, command TEXT, frame TEXT, ocr TEXT, ocr_conf REAL,
         strong TEXT, weak TEXT);
         CREATE VIRTUAL TABLE IF NOT EXISTS seg_fts USING fts5(id, text, entities, intent, strong);",
    )?;
    // 真正写入 (原孤儿版只建表不插入 → 空包, 且 db.commit() 编译不过)
    for u in &units {
        let text = u["text"].as_str().unwrap_or("");
        let intent = crate::learn::detect_intent(text).0;
        db.execute(
            "INSERT OR REPLACE INTO segments VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            rusqlite::params![
                u["id"].as_str().unwrap_or(""),
                u["t0"].as_f64().unwrap_or(0.0),
                u["t1"].as_f64().unwrap_or(0.0),
                text, intent,
                u["command"].as_str().unwrap_or(""),
                u["frame"].as_str().unwrap_or(""),
                u["ocr"].as_str().unwrap_or(""),
                u["ocr_conf"].as_f64().unwrap_or(0.0),
                "", "",
            ],
        )?;
        // 索引侧分词与查询侧 tokens() 同一实现 (与 learn.rs build_cues 对齐)
        let fts_text = crate::tokens::tokens(text).join(" ");
        db.execute(
            "INSERT OR REPLACE INTO seg_fts VALUES(?1,?2,?3,?4,?5)",
            rusqlite::params![u["id"].as_str().unwrap_or(""), fts_text, "", intent, ""],
        )?;
    }
    Ok(units.len())
}

/// 端到端: 索引 CLI 工具并构建 tool-scoped 知识包 (零 Python)。
/// intent = `{tool}.main` / `{tool}.{sub}`,uid = `{tool}_{i:03d}`,
/// 重建前删同工具旧条目 (幂等, 多工具共存一包不互相污染)。
/// 索引侧分词用 tokens() —— 与 Rust reader (Pack::by_fts) 查询侧同一实现,
/// 故不复制 Python 的 seg() (两者不同源会导致查不到)。help_fts 不移植:
/// 无任何 reader 查询它 (Python 侧亦只写不读)。
pub fn index_and_build(tool: &str, pack_dir: &Path) -> anyhow::Result<usize> {
    use rusqlite::Connection;
    let help_text = run_help(tool, &["--help"])
        .ok_or_else(|| anyhow::anyhow!("{} --help 执行失败", tool))?;
    let commands = parse_commands(&help_text);

    // (sub, desc, full_help): sub="" 表示工具主条目
    let mut entries: Vec<(String, String, String)> =
        vec![("".into(), "命令行工具".into(), help_text.clone())];
    for (sub, desc) in commands.iter().take(60) {
        let full = run_help(tool, &["help", sub]).unwrap_or_default();
        entries.push((sub.clone(), desc.clone(), full));
    }

    for d in ["knowledge", "index"] {
        std::fs::create_dir_all(pack_dir.join(d))?;
    }
    let db = Connection::open(pack_dir.join("index/knowledge.sqlite"))?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS segments(id TEXT PRIMARY KEY, t0 REAL, t1 REAL,
         text TEXT, intent TEXT, command TEXT, frame TEXT, ocr TEXT, ocr_conf REAL,
         strong TEXT, weak TEXT);
         CREATE VIRTUAL TABLE IF NOT EXISTS seg_fts USING fts5(id, text, entities, intent, strong);",
    )?;
    // 幂等: 删同工具旧条目 (含 fts)
    {
        let mut stmt = db.prepare("SELECT id FROM segments WHERE intent LIKE ?1")?;
        let old: Vec<String> = stmt
            .query_map([format!("{tool}.%")], |r| r.get::<_, String>(0))?
            .collect::<Result<_, _>>()?;
        for id in old {
            db.execute("DELETE FROM seg_fts WHERE id = ?1", [&id])?;
            db.execute("DELETE FROM segments WHERE id = ?1", [&id])?;
        }
    }
    let mut jsonl = String::new();
    for (i, (sub, desc, full)) in entries.iter().enumerate() {
        let uid = format!("{tool}_{i:03}", i = i);
        let intent = if sub.is_empty() {
            format!("{tool}.main")
        } else {
            format!("{tool}.{sub}")
        };
        let command = if sub.is_empty() {
            tool.to_string()
        } else {
            format!("{tool} {sub}")
        };
        let text = if sub.is_empty() {
            format!("{command} : {desc} : 用法见帮助")
        } else {
            let snippet: String = full.lines().next().unwrap_or("").to_string();
            format!("{command} : {desc} : 用法: {snippet}")
        };
        let strong = crate::tokens::tokens(&command).join(" ");
        let fts_text = crate::tokens::tokens(&format!("{command} {desc}")).join(" ");
        // 按 char 边界截 200 (中文帮助文本按字节切会 panic)
        let ocr_snippet: String = full.chars().take(200).collect();
        db.execute(
            "INSERT OR REPLACE INTO segments VALUES(?1,0,0,?2,?3,?4,'',?5,1.0,?6,'')",
            rusqlite::params![&uid, &text, &intent, &command, &ocr_snippet, &strong],
        )?;
        db.execute(
            "INSERT OR REPLACE INTO seg_fts VALUES(?1,?2,?3,?4,?5)",
            rusqlite::params![&uid, &fts_text, &strong, &intent, &strong],
        )?;
        let u = serde_json::json!({
            "id": uid, "t0": 0.0, "t1": 0.0, "text": text, "intent": intent,
            "command": command, "frame": "", "ocr": "", "ocr_conf": 1.0,
            "strong": crate::tokens::tokens(&command), "weak": [],
        });
        jsonl.push_str(&serde_json::to_string(&u)?);
        jsonl.push('\n');
    }
    std::fs::write(pack_dir.join("knowledge/segments.jsonl"), jsonl)?;
    Ok(entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clap_commands() {
        let help = "Usage: jj [OPTIONS] <COMMAND>\n\nCommands:\n  log    Show revision history\n  new    Create a new change\n\nOptions:\n  -h, --help";
        let cmds = parse_commands(help);
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].0, "log");
        assert!(cmds[0].1.contains("history"));
    }

    #[test]
    fn parse_empty_help() {
        assert!(parse_commands("no commands here").is_empty());
    }

    /// clap 带短别名格式 ("build, b  Compile...") 不应产出 "build," 或把别名混进描述
    #[test]
    fn parse_clap_alias_format() {
        let help = "Usage: cargo [OPTIONS] [COMMAND]\n\nCommands:\n  build, b    Compile the current package\n  check, c    Analyze the current package\n  clean       Remove the target directory\n\nOptions:\n  -h, --help";
        let cmds = parse_commands(help);
        assert_eq!(cmds.len(), 3, "got {cmds:?}");
        assert_eq!(cmds[0].0, "build", "逗号别名应剥离, got {:?}", cmds[0].0);
        assert!(cmds[0].1.starts_with("Compile"), "短别名 b 不应进描述, got {:?}", cmds[0].1);
        assert_eq!(cmds[2].0, "clean", "无别名行也应正常");
    }

    /// adb 式多段格式 (用户真实目标): 多个非 "Commands:" 段标题都应被识别为命令段,
    /// 而 global options / environment variables 段应被排除。
    #[test]
    fn parse_adb_multisection() {
        let help = "Android Debug Bridge version 1.0.41\n\nglobal options:\n -a   listen on all interfaces\n -d   use USB device\n\ngeneral commands:\n devices [-l]   list connected devices\n help           show this help message\n version        show version num\n\nnetworking:\n connect HOST   connect via TCP/IP\n tcpip PORT     restart on TCP port\n\nenvironment variables:\n ANDROID_SERIAL  specify device";
        let cmds = parse_commands(help);
        let names: Vec<&str> = cmds.iter().map(|c| c.0.as_str()).collect();
        // general commands + networking 段命中; global options(-a/-d) 与 env vars 排除
        assert!(names.contains(&"devices"), "adb general 段应命中, got {names:?}");
        assert!(names.contains(&"connect"), "adb networking 段应命中, got {names:?}");
        assert!(!names.contains(&"-a"), "选项行不应进命令, got {names:?}");
        assert!(!names.contains(&"ANDROID_SERIAL"), "env var 段应排除, got {names:?}");
    }
}
