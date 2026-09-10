//! lernen (学习) — 学习队列 → 训练数据回流 (R2 闭环的 Rust 侧收口)
//!
//! 数据流 (DESIGN.md 学习闭环):
//!   端侧 agent 运行时 → learning_queue.jsonl (NO_HIT/工具缺失)
//!                     → lernen::digest() → training_tasks.jsonl
//!                     → CloudStudio GRPO/SFT 管线消费 (qwen_grpo_train.py 等)
//!                     → 新模型下发 → 端侧能力增长
//!
//! digest 三步:
//!   1. load:   读队列 jsonl (query/reason/ts)
//!   2. dedupe: 相同 query+reason 合并, 计数 (频次 = 优先级信号)
//!   3. tasks:  生成训练任务 — GRPO 奖励环境需要的 (prompt, expected_tool) 对
//!              reason 含 "NO_HIT" → lyv_knowledge 任务
//!              reason 含 "vnn"    → vnn_identify 任务 (VNN 训练队列)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct QueueEntry {
    pub query: String,
    pub reason: String,
    pub ts: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TrainingTask {
    pub query: String,
    pub expected_tool: &'static str,
    pub frequency: usize,
    pub first_seen: u64,
    pub last_seen: u64,
    /// 建议的奖励信号 (GRPO reward environment 可直接用)
    pub reward_hint: &'static str,
}

/// 从学习队列文件 digest 出训练任务 (按频次降序)
pub fn digest(queue_path: &Path) -> std::io::Result<Vec<TrainingTask>> {
    let raw = std::fs::read_to_string(queue_path)?;
    let mut merged: HashMap<(String, &'static str), TrainingTask> = HashMap::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(entry) = serde_json::from_str::<QueueEntry>(line) else {
            continue; // 坏行跳过, 不阻塞 digest
        };
        // 垃圾过滤: 无意义查询不进训练集 (bench 压测/乱敲键盘产生的)
        if is_noise(&entry.query) {
            continue;
        }
        let (tool, hint) = classify(&entry.reason);
        let key = (entry.query.clone(), tool);
        merged
            .entry(key)
            .and_modify(|t| {
                t.frequency += 1;
                t.last_seen = t.last_seen.max(entry.ts);
            })
            .or_insert(TrainingTask {
                query: entry.query.clone(),
                expected_tool: tool,
                frequency: 1,
                first_seen: entry.ts,
                last_seen: entry.ts,
                reward_hint: hint,
            });
    }
    let mut tasks: Vec<TrainingTask> = merged.into_values().collect();
    tasks.sort_by(|a, b| b.frequency.cmp(&a.frequency).then(b.last_seen.cmp(&a.first_seen)));
    Ok(tasks)
}

/// 噪声查询检测: 乱敲键盘/测试串/重复字符模式
fn is_noise(query: &str) -> bool {
    let q = query.trim();
    if q.chars().count() < 3 {
        return true;
    }
    // 重复字符比例过高 (如 "胡说八道xyz" 的 xyz 模式 — 无信息量的 ASCII 尾巴)
    let ascii_tail = q.chars().rev().take_while(|c| c.is_ascii_alphanumeric()).count();
    // 包含常见无意义测试词
    for noise in ["xyz", "abc", "test123", "asdf", "aaaa"] {
        if q.to_lowercase().contains(noise) {
            return true;
        }
    }
    let _ = ascii_tail;
    false
}

/// reason → (期望工具, 奖励提示)
fn classify(reason: &str) -> (&'static str, &'static str) {
    if reason.contains("NO_HIT") {
        ("lyv_knowledge", "调用 lyv_knowledge 且知识包应包含该操作 (+1)")
    } else if reason.contains("vnn") || reason.contains("VNN") {
        ("vnn_identify", "识图请求应调用 vnn_identify (+1)")
    } else {
        ("lyv_knowledge", "默认知识库查询 (+1)")
    }
}

/// 写出训练任务文件 (CloudStudio 消费格式: 每行一个任务 JSON)
pub fn write_tasks(tasks: &[TrainingTask], out_path: &Path) -> std::io::Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    use std::io::Write;
    let mut f = std::fs::File::create(out_path)?;
    for t in tasks {
        writeln!(f, "{}", serde_json::to_string(t)?)?;
    }
    Ok(())
}

/// 端到端便捷函数: digest + write 一步完成, 返回任务数
pub fn harvest(queue_path: &Path, out_path: &Path) -> std::io::Result<usize> {
    let tasks = digest(queue_path)?;
    write_tasks(&tasks, out_path)?;
    Ok(tasks.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_queue(dir: &Path, entries: &[&str]) -> PathBuf2 {
        let p = dir.join("learning_queue.jsonl");
        std::fs::write(&p, entries.join("\n")).unwrap();
        PathBuf2(p)
    }

    // 包装以便测试隔离 (临时目录)
    struct PathBuf2(std::path::PathBuf);
    impl std::ops::Deref for PathBuf2 {
        type Target = std::path::Path;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    #[test]
    fn digest_dedupes_and_counts() {
        let dir = tempfile::tempdir().unwrap();
        let q = write_queue(
            dir.path(),
            &[
                r#"{"query":"怎么配置防火墙","reason":"NO_HIT: 知识库没有这个操作","ts":100}"#,
                r#"{"query":"怎么配置防火墙","reason":"NO_HIT: 知识库没有这个操作","ts":200}"#,
                r#"{"query":"帮我识图","reason":"vnn_identify: Rust VNN 未实现","ts":300}"#,
                "坏行不炸",
            ],
        );
        let tasks = digest(&q).unwrap();
        assert_eq!(tasks.len(), 2, "去重后 2 个任务");
        // 频次排序: 防火墙 x2 在前
        assert_eq!(tasks[0].query, "怎么配置防火墙");
        assert_eq!(tasks[0].frequency, 2);
        assert_eq!(tasks[0].expected_tool, "lyv_knowledge");
        assert_eq!(tasks[1].expected_tool, "vnn_identify");
        assert_eq!(tasks[0].first_seen, 100);
        assert_eq!(tasks[0].last_seen, 200);
    }

    #[test]
    fn harvest_writes_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let q = write_queue(dir.path(), &[r#"{"query":"如何配置测试环境","reason":"NO_HIT","ts":1}"#]);
        let out = dir.path().join("training_tasks.jsonl");
        let n = harvest(&q, &out).unwrap();
        assert_eq!(n, 1);
        let content = std::fs::read_to_string(&out).unwrap();
        assert!(content.contains("\"frequency\":1"));
        assert!(content.contains("\"expected_tool\":\"lyv_knowledge\""));
    }
}
