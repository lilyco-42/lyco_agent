//! learn — 端侧知识包生产 (lyv build 的 Rust 移植, SRT 驱动路径)
//!
//! 端侧自主学习路径 (对应 DESIGN.md 知识生产管线, ASR 走远端/云端, 端侧只做确定性编排):
//!   视频 [+SRT] → ffmpeg 场景切分 → 逐句抽帧 → tesseract OCR → 强/弱关联挖掘
//!                → sqlite/FTS 索引 → 知识包
//!
//! Rust 版 v0 范围: SRT 驱动 (用户已有字幕) — ASR 归训练侧 (CloudStudio)。
//! 事件挖掘复用 Python 侧验证过的 SUBCMDS 上下文纠错逻辑 (精简版)。

use crate::tokens::tokens as tokenize;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// SRT 条目
#[derive(Debug, Clone)]
pub struct Cue {
    pub t0: f64,
    pub t1: f64,
    pub text: String,
}

/// 解析 SRT (00:00:01,800 --> 格式)。容忍 CRLF/LF (Windows 记事本写的字幕常见 CRLF)
pub fn parse_srt(srt: &str) -> Vec<Cue> {
    let normalized = srt.replace("\r\n", "\n");
    let mut cues = Vec::new();
    for block in normalized.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() < 2 {
            continue;
        }
        let timing = match lines.iter().find(|l| l.contains("-->")) {
            Some(l) => *l,
            None => continue,
        };
        let (a, z) = match timing.split_once("-->") {
            Some(x) => x,
            None => continue,
        };
        let (Some(t0), Some(t1)) = (parse_ts(a.trim()), parse_ts(z.trim())) else {
            continue;
        };
        let text: String = lines[lines.len() - 1].trim().to_string();
        if !text.is_empty() {
            cues.push(Cue { t0, t1, text });
        }
    }
    cues
}

fn parse_ts(s: &str) -> Option<f64> {
    let (hms, ms) = s.split_once(',')?;
    let mut it = hms.split(':');
    let h: f64 = it.next()?.parse().ok()?;
    let m: f64 = it.next()?.parse().ok()?;
    let sec: f64 = it.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec + ms.parse::<f64>().ok()? / 1000.0)
}

/// 强/弱关联挖掘 (lyv.mine_event 精简移植)
/// 强关联 = 文本与 OCR 共同出现的词 (跨模态互证)
pub fn mine_event(text: &str, ocr_text: &str) -> (Vec<String>, Vec<String>) {
    let text_toks: std::collections::HashSet<String> = tokenize(text).into_iter().collect();
    let ocr_toks: std::collections::HashSet<String> = tokenize(ocr_text).into_iter().collect();
    let mut strong = Vec::new();
    let mut weak = Vec::new();
    for t in text_toks {
        if t.chars().count() < 2 || t.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if ocr_toks.contains(&t) {
            strong.push(t);
        } else if t.is_ascii() {
            weak.push(t); // 中文 bigram 太碎, 只收 ASCII 弱关联
        }
    }
    strong.sort();
    weak.sort();
    (strong, weak.into_iter().take(12).collect())
}

/// 场景帧: ffmpeg 从视频抽一帧
fn extract_frame(ffmpeg: &str, video: &Path, at: f64, out: &Path) -> std::io::Result<()> {
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-y",
            "-v",
            "error",
            "-ss",
            &format!("{at:.3}"),
            "-i",
            &video.to_string_lossy(),
            "-frames:v",
            "1",
            &out.to_string_lossy(),
        ])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("ffmpeg frame extract failed"))
    }
}

/// 端侧 ASR: ffmpeg 提取 16kHz wav → whisper-cli (whisper.cpp) 转写。
/// 返回 (t0, t1, text) 列表。whisper 二进制/模型可用环境变量覆盖:
///   LYV_WHISPER_BIN (默认 whisper-cli), LYV_WHISPER_MODEL (默认 ggml-base.bin)
pub fn asr_local(
    video: &Path,
    ffmpeg: &str,
    lang: &str,
) -> anyhow::Result<Vec<(f64, f64, String)>> {
    use std::io::Write;
    let work = tempfile_dir()?;
    std::fs::create_dir_all(&work)?;
    let wav = work.join("lyv_audio.wav");
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-y", "-v", "error", "-i", &video.to_string_lossy(),
            "-vn", "-ar", "16000", "-ac", "1",
            &wav.to_string_lossy(),
        ])
        .status()?;
    anyhow::ensure!(status.success(), "ffmpeg 音频提取失败");

    let bin = std::env::var("LYV_WHISPER_BIN").unwrap_or_else(|_| "whisper-cli".into());
    let model = std::env::var("LYV_WHISPER_MODEL")
        .unwrap_or_else(|_| "ggml-base.bin".into());
    let out_base = work.join("lyv_asr");
    let output = std::process::Command::new(&bin)
        .args([
            "-m", &model,
            "-f", &wav.to_string_lossy(),
            "-l", lang,
            "-oj", "-of", &out_base.to_string_lossy(),
        ])
        .output()?;
    anyhow::ensure!(output.status.success(), "whisper-cli 失败: {}", String::from_utf8_lossy(&output.stderr));

    let json_path = out_base.with_extension("json");
    let raw = std::fs::read_to_string(&json_path)?;
    let v: serde_json::Value = serde_json::from_str(&raw)?;
    let mut segs = Vec::new();
    if let Some(items) = v["transcription"].as_array() {
        for item in items {
            let t0 = ms_to_sec(item["offsets"]["from"].as_u64().unwrap_or(0));
            let t1 = ms_to_sec(item["offsets"]["to"].as_u64().unwrap_or(0));
            let text = item["text"].as_str().unwrap_or("").trim().to_string();
            if !text.is_empty() {
                segs.push((t0, t1, text));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    Ok(segs)
}

fn ms_to_sec(ms: u64) -> f64 {
    ms as f64 / 1000.0
}

fn tempfile_dir() -> std::io::Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok(std::env::temp_dir().join(format!("lyv_asr_{stamp}")))
}

/// 构建知识包 (SRT 驱动): video + srt → pack 目录
/// 帧质量评分: OCR 词命中数 (与 Python 冒烟一致的简化评分, sharpness 留给训练侧)
pub fn build(
    video: &Path,
    srt: &str,
    pack_dir: &Path,
    ffmpeg: &str,
    ocr: &crate::verify::Ocr,
    ocr_lang: &str,
) -> anyhow::Result<usize> {
    let cues = parse_srt(srt);
    anyhow::ensure!(!cues.is_empty(), "SRT 无有效字幕");
    build_cues(video, &cues, pack_dir, ffmpeg, ocr, ocr_lang)
}

/// Cue 列表驱动的构建核心 (SRT 或 ASR 来源都汇到这里)
pub fn build_cues(
    video: &Path,
    cues: &[Cue],
    pack_dir: &Path,
    ffmpeg: &str,
    ocr: &crate::verify::Ocr,
    ocr_lang: &str,
) -> anyhow::Result<usize> {
    for d in ["frames", "ocr", "knowledge", "index"] {
        std::fs::create_dir_all(pack_dir.join(d))?;
    }
    std::fs::copy(video, pack_dir.join("source.mp4"))?;

    let mut units = Vec::new();
    for (i, cue) in cues.iter().enumerate() {
        let uid = format!("u{i:03}");
        let mid = (cue.t0 + cue.t1) / 2.0;
        let frame = pack_dir.join("frames").join(format!("{uid}_f0.webp"));
        extract_frame(ffmpeg, video, mid, &frame)?;
        let (ocr_text, ocr_conf) = ocr
            .recognize(&frame, ocr_lang)
            .map(|o| (o.text, o.conf))
            .unwrap_or_else(|| (String::new(), 0.0));
        let (strong, weak) = mine_event(&cue.text, &ocr_text);
        let intent = detect_intent(&cue.text);
        units.push(serde_json::json!({
            "id": uid, "t0": cue.t0, "t1": cue.t1, "text": cue.text,
            "frame": format!("frames/{uid}_f0.webp"), "frame_t": mid,
            "ocr": ocr_text, "ocr_conf": ocr_conf,
            "strong": strong, "weak": weak,
            "intent": intent.0, "command": intent.1,
        }));
    }

    // segments.jsonl
    let mut jsonl = String::new();
    for u in &units {
        jsonl.push_str(&serde_json::to_string(u)?);
        jsonl.push('\n');
    }
    std::fs::write(pack_dir.join("knowledge/segments.jsonl"), jsonl)?;

    // sqlite + FTS (与 Python build 的 schema 完全一致)
    let db = Connection::open(pack_dir.join("index/knowledge.sqlite"))?;
    db.execute_batch(
        "CREATE TABLE segments(id TEXT PRIMARY KEY, t0 REAL, t1 REAL, text TEXT,
         intent TEXT, command TEXT, frame TEXT, ocr TEXT, ocr_conf REAL,
         strong TEXT, weak TEXT);
         CREATE VIRTUAL TABLE seg_fts USING fts5(id, text, entities, intent, strong);",
    )?;
    for u in &units {
        let strong = u["strong"].as_array().cloned().unwrap_or_default();
        let strong_str: Vec<String> = strong
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        let text = u["text"].as_str().unwrap_or("");
        let command = u["command"].as_str().unwrap_or("");
        db.execute(
            "INSERT INTO segments VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            rusqlite::params![
                u["id"].as_str().unwrap_or(""),
                u["t0"].as_f64().unwrap_or(0.0),
                u["t1"].as_f64().unwrap_or(0.0),
                text,
                u["intent"].as_str().unwrap_or(""),
                command,
                u["frame"].as_str().unwrap_or(""),
                u["ocr"].as_str().unwrap_or(""),
                u["ocr_conf"].as_f64().unwrap_or(0.0),
                strong_str.join(" "),
                u["weak"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(" ")).unwrap_or_default(),
            ],
        )?;
        // 索引侧分词: 与查询侧 tokens() 同一实现 (对齐保证)
        let fts_text = tokenize(text).join(" ");
        let fts_entities = tokenize(command).join(" ");
        let fts_strong = tokenize(&strong_str.join(" ")).join(" ");
        db.execute(
            "INSERT INTO seg_fts VALUES(?1,?2,?3,?4,?5)",
            rusqlite::params![u["id"].as_str().unwrap_or(""), fts_text, fts_entities,
                              u["intent"].as_str().unwrap_or(""), fts_strong],
        )?;
    }
    db.execute_batch(
        "CREATE INDEX idx_intent ON segments(intent);",
    )?;

    let meta = serde_json::json!({
        "format": "LYV 0.1", "units": units.len(),
        "builder": "lycore-learn-rs",
    });
    std::fs::write(pack_dir.join("meta.json"), serde_json::to_string_pretty(&meta)?)?;
    Ok(units.len())
}

/// intent 检测 (mine_event 的词典部分, 含 SUBCMDS 上下文纠错)
fn detect_intent(text: &str) -> (String, Option<String>) {
    let lower = text.to_lowercase();
    let word_after = |tool: &str| -> Option<String> {
        let idx = lower.find(tool)? + tool.len();
        let rest: String = lower[idx..].trim_start().chars().take(16).collect();
        // 只取 ASCII 词部分 (ASR 文本常见 'cargo run運行程序' — 中文紧跟无空格)
        let ascii_end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(rest.len());
        Some(rest[..ascii_end].to_lowercase())
    };
    // (工具, [子命令集], intent 前缀)
    const TOOLS: &[(&str, &[&str], &str)] = &[
        ("cargo", &["new", "run", "build", "test", "init"], "rust.project"),
        ("git", &["clone", "push", "pull", "commit", "init"], "git"),
        ("pip", &["install"], "py.pkg"),
        ("npm", &["install"], "node.pkg"),
    ];
    for (tool, subs, prefix) in TOOLS {
        if lower.contains(tool) {
            let sub = word_after(tool);
            let Some(sub) = sub else { continue };
            let corrected = if subs.contains(&sub.as_str()) {
                sub.clone()
            } else {
                // 编辑距离<=2 矫正 (no→new)
                let best = subs.iter().min_by_key(|s| crate::verify::lev(&sub, s));
                match best {
                    Some(b) if crate::verify::lev(&sub, b) <= 2 => b.to_string(),
                    _ => continue,
                }
            };
            let intent = match (*prefix, corrected.as_str()) {
                ("rust.project", "new") => "rust.project.create",
                ("rust.project", "run") => "rust.project.run",
                ("rust.project", "build") => "rust.project.build",
                ("rust.project", "init") => "rust.project.init",
                ("git", "clone") => "git.clone",
                ("git", "push") => "git.push",
                ("pip", "install") => "py.pkg.install",
                ("npm", "install") => "node.pkg.install",
                _ => "misc.talk",
            };
            return (intent.to_string(), Some(format!("{tool} {corrected}")));
        }
    }
    if lower.contains("cd ") || lower.contains("进入") {
        return ("fs.chdir".to_string(), None);
    }
    ("misc.talk".to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_parsing() {
        let srt = "1\n00:00:03,000 --> 00:00:09,800\n首先 cargo new hello_world 建立项目\n\n2\n00:00:10,000 --> 00:00:14,800\n然后 cd 进入项目目录\n";
        let cues = parse_srt(srt);
        assert_eq!(cues.len(), 2);
        assert!((cues[0].t0 - 3.0).abs() < 1e-6);
        assert!((cues[0].t1 - 9.8).abs() < 1e-6);
        assert!(cues[0].text.contains("cargo"));
    }

    #[test]
    fn srt_parsing_crlf() {
        let srt = "1\r\n00:00:03,000 --> 00:00:09,800\r\n首先 cargo new 建立项目\r\n\r\n2\r\n00:00:10,000 --> 00:00:14,800\r\ncd 进入目录\r\n";
        let cues = parse_srt(srt);
        assert_eq!(cues.len(), 2, "CRLF 字幕也应解析");
    }

    #[test]
    fn intent_detection_with_subcmd_correction() {
        let (intent, cmd) = detect_intent("首先, Cargo No Hello World 建立项目。");
        assert_eq!(intent, "rust.project.create"); // no → new 上下文纠错
        assert_eq!(cmd.as_deref(), Some("cargo new"));
        let (i2, _) = detect_intent("今天天气怎么样");
        assert_eq!(i2, "misc.talk");
    }

    #[test]
    fn strong_weak_mining() {
        let (strong, weak) = mine_event("首先 cargo new 建立项目", "PS> cargo new demo");
        assert!(strong.contains(&"cargo".into()) && strong.contains(&"new".into()));
        assert!(!weak.contains(&"cargo".into()));
    }
}
