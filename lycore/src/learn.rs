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
    let work = tempfile_dir()?;
    std::fs::create_dir_all(&work)?;
    let wav = work.join("lyv_audio.wav");
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-y",
            "-v",
            "error",
            "-i",
            &video.to_string_lossy(),
            "-vn",
            "-ar",
            "16000",
            "-ac",
            "1",
            &wav.to_string_lossy(),
        ])
        .status()?;
    anyhow::ensure!(status.success(), "ffmpeg 音频提取失败");

    let bin = std::env::var("LYV_WHISPER_BIN").unwrap_or_else(|_| "whisper-cli".into());
    let model = std::env::var("LYV_WHISPER_MODEL").unwrap_or_else(|_| "ggml-base.bin".into());
    let out_base = work.join("lyv_asr");
    let output = std::process::Command::new(&bin)
        .args([
            "-m",
            &model,
            "-f",
            &wav.to_string_lossy(),
            "-l",
            lang,
            "-oj",
            "-of",
            &out_base.to_string_lossy(),
        ])
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "whisper-cli 失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );

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

/// 幂等重建: 清掉旧派生产物 (sqlite/fts/rules/帧/ocr), 保留运行时学习队列
/// (learning_queue.jsonl 是 FEE 飞轮的记忆, 重建知识包不该清空已收集的待学任务)。
/// 只删文件, 目录由随后 create_dir_all 兜底。返回删除条目数 (供测试断言)。
/// 修的是: build_cues 用 Connection::open + CREATE TABLE, 重跑进已有 pack 会撞
/// "table segments already exists" 直接崩 (demo.sh 靠前置 rm -rf 掩盖)。
pub fn reset_pack_for_rebuild(pack_dir: &Path) -> usize {
    let mut n = 0;
    for f in [
        "index/knowledge.sqlite",
        "index/knowledge.sqlite-wal",
        "index/knowledge.sqlite-shm",
        "rules.json",
    ] {
        if std::fs::remove_file(pack_dir.join(f)).is_ok() {
            n += 1;
        }
    }
    for d in ["frames", "ocr"] {
        if let Ok(rd) = std::fs::read_dir(pack_dir.join(d)) {
            for e in rd.flatten() {
                if std::fs::remove_file(e.path()).is_ok() {
                    n += 1;
                }
            }
        }
    }
    n
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
    // 幂等重建: 先清旧派生产物 (保留 learning_queue.jsonl), 否则 CREATE TABLE 撞已存在崩
    reset_pack_for_rebuild(pack_dir);
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

    // 自学习词表 (零硬编码): 从 strong (ASR∩OCR 跨模态互证) 派生领域实体,
    // 给内置词典未覆盖的 misc.talk 单元重打 {实体}.misc intent; 学习规则与已有
    // rules.json 合并 (手写领域规则优先), 检索侧 Pack::open 优先加载 rules.json。
    let learned = learn_vocabulary(&mut units);
    if !learned.is_empty() {
        let path = pack_dir.join("rules.json");
        let mut merged: Vec<serde_json::Value> = Vec::new();
        let mut intents: std::collections::HashSet<String> = Default::default();
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Some(list) = serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .and_then(|v| v.get("rules").and_then(|r| r.as_array().cloned()))
            {
                for r in list {
                    if let Some(it) = r.get("intent").and_then(|x| x.as_str()) {
                        if intents.insert(it.to_string()) {
                            merged.push(r);
                        }
                    }
                }
            }
        }
        for r in learned {
            if let Some(it) = r.get("intent").and_then(|x| x.as_str()) {
                if intents.insert(it.to_string()) {
                    merged.push(r);
                }
            }
        }
        if !merged.is_empty() {
            let doc = serde_json::to_string_pretty(&serde_json::json!({ "rules": merged }))
                .map_err(|e| anyhow::anyhow!("rules.json 序列化失败: {e}"))?;
            std::fs::write(&path, doc)?;
        }
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
         CREATE VIRTUAL TABLE seg_fts USING fts5(id, text, entities, intent, strong, ocr);",
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
                u["weak"]
                    .as_array()
                    .map(|a| a
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(" "))
                    .unwrap_or_default(),
            ],
        )?;
        // 索引侧分词: 与查询侧 tokens() 同一实现 (对齐保证)
        let fts_text = tokenize(text).join(" ");
        let fts_entities = tokenize(command).join(" ");
        let fts_strong = tokenize(&strong_str.join(" ")).join(" ");
        // OCR 入索引: ASR 听漏的技术词 (如 scrcpy) 常在屏幕 OCR 里, 表级 MATCH 自动召回
        let fts_ocr = tokenize(u["ocr"].as_str().unwrap_or("")).join(" ");
        db.execute(
            "INSERT INTO seg_fts VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![
                u["id"].as_str().unwrap_or(""),
                fts_text,
                fts_entities,
                u["intent"].as_str().unwrap_or(""),
                fts_strong,
                fts_ocr
            ],
        )?;
    }
    db.execute_batch("CREATE INDEX idx_intent ON segments(intent);")?;

    let meta = serde_json::json!({
        "format": "LYV 0.1", "units": units.len(),
        "builder": "lycore-learn-rs",
    });
    std::fs::write(
        pack_dir.join("meta.json"),
        serde_json::to_string_pretty(&meta)?,
    )?;
    Ok(units.len())
}

/// 自学习词表 (零硬编码): 实体从素材自身的 strong token (ASR∩OCR 跨模态互证) 派生。
/// 1) 候选 = ASCII、≥2 字符、非纯数字、跨 ≥2 段互证 (单段共现易巧合);
/// 2) 仅重打内置词典未覆盖 (misc.talk) 的单元 → `{实体}.misc`;
/// 3) 每实体产一条 left-only 规则供检索侧 rules.json。
///    平票取字典序小者 (确定性; 工具名通常短于动词, 如 adb < install)。
pub fn learn_vocabulary(units: &mut [serde_json::Value]) -> Vec<serde_json::Value> {
    use std::collections::HashMap;
    let mut freq: HashMap<String, usize> = HashMap::new();
    for u in units.iter() {
        let mut seen: std::collections::HashSet<String> = Default::default();
        let Some(strong) = u["strong"].as_array() else {
            continue;
        };
        for s in strong.iter().filter_map(|v| v.as_str()) {
            let t = s.trim().to_lowercase();
            if t.len() >= 2 && t.is_ascii() && !t.chars().all(|c| c.is_ascii_digit()) {
                seen.insert(t);
            }
        }
        for t in seen {
            *freq.entry(t).or_insert(0) += 1;
        }
    }
    freq.retain(|_, n| *n >= 2);
    if freq.is_empty() {
        return Vec::new();
    }
    let mut learned: std::collections::HashSet<String> = Default::default();
    for u in units.iter_mut() {
        if u["intent"].as_str() != Some("misc.talk") {
            continue;
        }
        let Some(strong) = u["strong"].as_array() else {
            continue;
        };
        let mut cands: Vec<(usize, String)> = strong
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_lowercase())
            .filter(|t| freq.contains_key(t))
            .map(|t| (freq[&t], t))
            .collect();
        cands.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        if let Some((_, entity)) = cands.into_iter().next() {
            learned.insert(entity.clone());
            u["intent"] = serde_json::Value::String(format!("{entity}.misc"));
        }
    }
    // 只发实际学到的实体 (cargo 等已被内置词典命中的单元不会进 learned, 避免
    // 生成 left-only 宽规则去劫持内置 intent 路由)
    let mut entities: Vec<String> = learned.into_iter().collect();
    entities.sort();
    entities
        .into_iter()
        .map(|e| serde_json::json!({"left": [e], "right": [], "intent": format!("{e}.misc")}))
        .collect()
}

/// intent 检测 (mine_event 的词典部分, 含 SUBCMDS 上下文纠错)
pub fn detect_intent(text: &str) -> (String, Option<String>) {
    let lower = text.to_lowercase();
    // 找带词边界的 tool 出现位置 (github 里的 git 不算), 取其后 ASCII 词
    let word_after = |tool: &str| -> Option<String> {
        let mut search = 0;
        let idx = loop {
            let i = lower[search..].find(tool)?;
            let i = search + i;
            let before = lower[..i]
                .chars()
                .last()
                .map(|c| !c.is_ascii_alphanumeric())
                .unwrap_or(true);
            let after = lower[i + tool.len()..]
                .chars()
                .next()
                .map(|c| c.is_ascii_whitespace() || c.is_ascii_uppercase() || c == '_')
                .unwrap_or(false);
            if before && after {
                break i + tool.len();
            }
            search = i + 1;
        };
        let rest: String = lower[idx..].trim_start().chars().take(16).collect();
        // 只取 ASCII 词部分 (ASR 文本常见 'cargo run運行程序' — 中文紧跟无空格)
        let ascii_end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(rest.len());
        Some(rest[..ascii_end].to_lowercase())
    };
    // (工具, [子命令集], intent 前缀)
    const TOOLS: &[(&str, &[&str], &str)] = &[
        (
            "cargo",
            &["new", "run", "build", "test", "init"],
            "rust.project",
        ),
        ("git", &["clone", "push", "pull", "commit", "init"], "git"),
        ("pip", &["install"], "py.pkg"),
        ("npm", &["install"], "node.pkg"),
    ];
    for (tool, subs, prefix) in TOOLS {
        // 词边界匹配: 'github' 不算 'git' 工具词 (借走闭包内 &str 生命周期问题 → 直接内联)
        // 遍历所有出现位置, 任一带词边界的出现即命中 ('github git push' 中后一个 git 有效)
        let mut found = false;
        let mut search_from = 0;
        while let Some(i) = lower[search_from..].find(tool) {
            let i = search_from + i;
            let before = lower[..i]
                .chars()
                .last()
                .map(|c| !c.is_ascii_alphanumeric())
                .unwrap_or(true);
            let after = lower[i + tool.len()..]
                .chars()
                .next()
                .map(|c| !c.is_ascii_alphanumeric())
                .unwrap_or(true);
            if before && after {
                found = true;
                break;
            }
            search_from = i + 1;
        }
        if found {
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
                ("git", "commit") => "git.commit",
                ("git", "clone") => "git.clone",
                ("git", "push") => "git.push",
                ("py.pkg", "install") => "py.pkg.install",
                ("node.pkg", "install") => "node.pkg.install",
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

    /// 幂等重建回归: 清派生产物 (sqlite/rules/帧/ocr) 但保留 learning_queue.jsonl
    /// (FEE 飞轮记忆)。修的是重跑 learn 进已有 pack 撞 CREATE TABLE 崩。
    #[test]
    fn reset_pack_clears_derived_keeps_queue() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        for d in ["index", "frames", "ocr", "knowledge"] {
            std::fs::create_dir_all(p.join(d)).unwrap();
        }
        std::fs::write(p.join("index/knowledge.sqlite"), b"x").unwrap();
        std::fs::write(p.join("rules.json"), b"{}").unwrap();
        std::fs::write(p.join("frames/u000_f0.webp"), b"x").unwrap();
        std::fs::write(p.join("ocr/u000.json"), b"x").unwrap();
        std::fs::write(p.join("knowledge/segments.jsonl"), b"{}\n").unwrap();
        std::fs::write(p.join("learning_queue.jsonl"), b"{\"query\":\"q\"}\n").unwrap();

        let n = reset_pack_for_rebuild(p);
        assert!(!p.join("index/knowledge.sqlite").exists(), "sqlite 应清");
        assert!(!p.join("rules.json").exists(), "rules 应清");
        assert!(!p.join("frames/u000_f0.webp").exists(), "帧应清");
        assert!(!p.join("ocr/u000.json").exists(), "ocr 应清");
        assert!(
            p.join("learning_queue.jsonl").exists(),
            "学习队列必须保留 (FEE 记忆不被重建清空)"
        );
        assert!(n >= 4, "至少清 4 类派生文件, got {n}");
    }

    /// 自学习词表回归: 跨段互证阈值 + 只重打 misc.talk + 只发实际学到的实体
    #[test]
    fn learn_vocabulary_derives_from_cross_modal_only() {
        let mk = |text: &str, intent: &str, strong: &[&str]| -> serde_json::Value {
            serde_json::json!({
                "id": "u", "t0": 0.0, "t1": 1.0, "text": text,
                "intent": intent, "strong": strong, "weak": [],
            })
        };
        let mut units = vec![
            // adb 跨 2 段互证 → 应学成 adb.misc
            mk("install adb", "misc.talk", &["adb", "install"]),
            mk("adb devices", "misc.talk", &["adb"]),
            // 单段孤词 (仅 1 次) → 不达阈值, 保持 misc.talk
            mk("hello_world 一下", "misc.talk", &["hello_world"]),
            // 内置词典已命中 (rust.project.create) → 不得被重打, 也不得发 cargo 规则
            mk(
                "cargo new demo",
                "rust.project.create",
                &["cargo", "new", "demo"],
            ),
        ];
        let rules = learn_vocabulary(&mut units);

        assert_eq!(units[0]["intent"], "adb.misc");
        assert_eq!(units[1]["intent"], "adb.misc");
        assert_eq!(units[2]["intent"], "misc.talk", "单段孤词不该被学成实体");
        assert_eq!(
            units[3]["intent"], "rust.project.create",
            "内置词典命中的单元不得被重打"
        );

        let intents: Vec<&str> = rules
            .iter()
            .map(|r| r["intent"].as_str().unwrap_or(""))
            .collect();
        assert!(
            intents.contains(&"adb.misc"),
            "adb 应产出规则, got {intents:?}"
        );
        assert!(
            !intents.contains(&"cargo.misc"),
            "cargo 未产生重打却发规则 = 劫持内置路由的隐患, got {intents:?}"
        );
        assert!(
            !intents.contains(&"hello_world.misc"),
            "未达阈值的实体不该有规则, got {intents:?}"
        );
    }
}
