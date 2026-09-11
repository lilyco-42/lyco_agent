//! learn_cli — Rust 原生 CLI 手册索引 (零 Python 依赖)
//!
//! 对 Python cli_indexer.py 的 Rust 移植:
//!   tool --help → 解析 Commands 段 → 逐子命令 help → 知识包 build
//!   lilyco 应用: --schema JSON 直接入库
//!
//! 输出与 learn.rs build_cues 同 schema (sqlite + FTS5), 共享 pack 基础设施。

use crate::learn::Cue;
use std::path::Path;

/// 解析 --help 输出的 Commands 段 (clap 格式: "  name   描述")
pub fn parse_commands(help_text: &str) -> Vec<(String, String)> {
    let mut commands = Vec::new();
    let mut in_commands = false;
    for line in help_text.lines() {
        let trimmed = line.trim();
        if trimmed == "Commands:" || trimmed == "Subcommands:" {
            in_commands = true;
            continue;
        }
        if in_commands {
            if trimmed.is_empty() && !commands.is_empty() {
                break;
            }
            let mut parts = trimmed.splitn(2, char::is_whitespace);
            if let (Some(name), Some(desc)) = (parts.next(), parts.next()) {
                let name = name.trim();
                let desc = desc.trim();
                if !name.is_empty()
                    && name
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_alphanumeric())
                        .unwrap_or(false)
                {
                    commands.push((name.to_string(), desc.to_string()));
                }
            }
        }
    }
    commands
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
    db.commit();
    db.close();
    Ok(units.len())
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
}
