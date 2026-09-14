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
        //
        // 创作白名单优先于噪声启发式: is_noise 的 "<3 字符 = 噪声" 是为挡乱敲的
        // 短 ASCII 串写的, 但中文 2 字可能是真请求 ("队名" 在回归队列里出现 7 次 ——
        // 注: 该队列源自 agent-loop 测试残留, 非有机用户流量, 阈值调优不可依赖其分布)。
        // 放宽阈值会同时放进 2 字中文参数名 ("路径"/"图片"), 故改为: 命中创作
        // 白名单 (显式信号) 时跳过噪声判据, 其余一律照旧过滤。
        if !is_creative_request(&entry.query) && is_noise(&entry.query) {
            continue;
        }
        let Some((tool, hint)) = classify(&entry.query, &entry.reason) else {
            continue; // 工具执行失败等非知识缺口, 不进 FC 训练集
        };
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

/// 噪声查询检测: 乱敲键盘/测试串/重复字符/非自然语言 (路径、schema 占位符)
fn is_noise(query: &str) -> bool {
    let q = query.trim();
    if q.chars().count() < 3 {
        return true;
    }
    // 自然语言判据 (高精度, 专挡反馈污染): 训练 query 必须含 CJK 字符或 ≥2 词。
    // e2e 测试/模型幻觉把文件路径 (../smoke/x.html) 和 schema 参数名 (image_url,
    // your_image.png) 灌进队列 —— 喂 GRPO 等于教模型「一个路径=一个知识查询」。
    let has_cjk = q.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    let words = q.split_whitespace().count();
    if !has_cjk && words < 2 {
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

/// reason → FC 训练任务; 仅 NO_HIT 是知识缺口, 工具执行失败返回 None。
/// 只有 lyv_knowledge 的 NO_HIT 在 query 字段存的是**用户问句**; 工具执行失败
/// (html_gen: LLM 调用失败 / vnn_identify: 执行失败 等) 路由已正确、是基础设施故障,
/// 且 query 存的是工具参数 (路径/prompt), 进 FC 训练集会教出错误路由 (惩罚本已正确
/// 的工具选择), 故排除。可靠性信号仍在原始队列 jsonl, 供人工/告警消费。
///
/// ## NO_HIT 二次分类 (2026-09-14)
/// 短创作请求 ("队名" / "起个标题") 落进 NO_HIT 后, 若一律标 lyv_knowledge,
/// 等于把 **FC V4 修掉的 overcorrection 重新喂回训练集** ——
/// DESIGN.md 明确: 「短创作请求误路由 lyv」正是 V4 定向修复的靶心, 且
/// 「继续重训边际收益极低, V4 定为最终 FC 模型」。学习闭环若自我对抗,
/// 每轮回流都会把已修好的错误教回去。
/// 权威依据: `CHAT_TOOLS` 中 llm_generate 描述 = 「写文案/起标题等纯文字创作」。
fn classify(query: &str, reason: &str) -> Option<(&'static str, &'static str)> {
    if !reason.contains("NO_HIT") {
        return None;
    }
    if is_creative_request(query) {
        return Some((
            "llm_generate",
            "直接 llm_generate 生成, 不调 lyv_knowledge (短创作请求) (+1)",
        ));
    }
    Some(("lyv_knowledge", "调用 lyv_knowledge 且知识包应包含该操作 (+1)"))
}

/// 创作型请求检测 (高精度, 宁漏勿错 —— 漏判只是少一条训练数据,
/// 误判会把真知识查询教成"别去查知识包")。
///
/// 已知边界 (诚实记录): 疑问词开头的创作请求 ("怎么起标题") 判为知识查询,
/// 与 DESIGN 记录的 "怎么做红烧肉"→lyv 属同一类语言模糊, 不靠规则硬分。
fn is_creative_request(query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return false;
    }
    // 疑问词开头 = 在问"怎么做", 归知识查询
    for w in [
        "怎么", "如何", "怎样", "什么", "为什么", "哪里", "哪个", "是否", "能不能", "可以",
    ] {
        if q.starts_with(w) {
            return false;
        }
    }
    // 创作动词
    for v in [
        "起个", "起一个", "想个", "想一个", "取个", "取一个", "编个", "来个", "帮我起", "帮我想",
        "帮我写", "给我起", "给我写", "写一首", "写一段", "写个", "生成一个", "创作一首",
        "作一首", "拟一个", "slogan",
    ] {
        if q.to_lowercase().contains(v) {
            return true;
        }
    }
    // 创作名词 (仅短请求算 —— 回归队列里 "队名" 单条出现 7 次, 系测试残留非有机流量)
    if q.chars().count() <= 8 {
        for n in [
            "队名", "笔名", "店名", "昵称", "网名", "标题", "文案", "情书", "段子", "歌词",
            "口号", "简介", "广告语", "藏头诗",
        ] {
            if q.contains(n) {
                return true;
            }
        }
    }
    false
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

    /// 回归: 只有 NO_HIT 是 FC 训练任务; 工具执行失败 (路由已对, 基础设施故障,
    /// query 存的是工具参数) 一律排除 —— 否则教出错误路由 (惩罚正确工具选择)
    #[test]
    fn digest_excludes_tool_execution_failures() {
        let dir = tempfile::tempdir().unwrap();
        let q = write_queue(
            dir.path(),
            &[
                r#"{"query":"../smoke/page.html","reason":"html_render_video: 执行失败","ts":1}"#,
                r#"{"query":"一个极简的个人主页","reason":"html_gen: LLM 调用失败","ts":2}"#,
                r#"{"query":"图片路径","reason":"vnn_identify: 执行失败","ts":3}"#,
                r#"{"query":"x.png","reason":"rembg_remove: 执行失败","ts":4}"#,
                r#"{"query":"怎么配置 nginx","reason":"NO_HIT: 知识库没有这个操作","ts":5}"#,
            ],
        );
        let tasks = digest(&q).unwrap();
        assert_eq!(tasks.len(), 1, "4 个工具执行失败应全排除, 仅剩 1 个知识缺口");
        assert_eq!(tasks[0].query, "怎么配置 nginx");
        assert_eq!(tasks[0].expected_tool, "lyv_knowledge");
    }

    #[test]
    fn digest_dedupes_and_counts() {
        let dir = tempfile::tempdir().unwrap();
        let q = write_queue(
            dir.path(),
            &[
                r#"{"query":"怎么配置防火墙","reason":"NO_HIT: 知识库没有这个操作","ts":100}"#,
                r#"{"query":"怎么配置防火墙","reason":"NO_HIT: 知识库没有这个操作","ts":200}"#,
                r#"{"query":"帮我识图","reason":"vnn_identify: 执行失败","ts":300}"#,
                "坏行不炸",
            ],
        );
        let tasks = digest(&q).unwrap();
        // 仅 NO_HIT 进 FC 训练集; vnn_identify 执行失败 (基础设施故障, 非知识缺口) 排除
        assert_eq!(tasks.len(), 1, "去重后仅 1 个知识任务");
        assert_eq!(tasks[0].query, "怎么配置防火墙");
        assert_eq!(tasks[0].frequency, 2);
        assert_eq!(tasks[0].expected_tool, "lyv_knowledge");
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

    /// 回归: 反馈污染过滤 —— 文件路径与 ASCII schema 占位符不得进训练集,
    /// 真实多词查询保留。诚实边界: 短中文 (<3 字) 与纯中文参数名 (图片路径)
    /// 无法靠"是否自然语言"区分, 前者由既有 <3 规则挡、后者是已知残留。
    #[test]
    fn digest_filters_path_and_placeholder_pollution() {
        assert!(is_noise("../smoke/e2e_page.html"), "相对路径应过滤");
        assert!(is_noise("/tmp/x.png"), "绝对路径应过滤");
        assert!(is_noise("image_url"), "snake_case 占位符应过滤");
        assert!(is_noise("your_image.png"), "带扩展名占位符应过滤");
        // 真实查询不误伤
        assert!(!is_noise("怎么配置 nginx"), "中英混合真查询保留");
        assert!(!is_noise("build the project"), "多词英文真查询保留");
        // 已知残留 (记录而非假装解决): 纯中文参数名无法与自然语言区分
        assert!(!is_noise("图片路径"), "CJK 占位符是已知残留 (需上下文才能判)");

        let dir = tempfile::tempdir().unwrap();
        let q = write_queue(
            dir.path(),
            &[
                r#"{"query":"../smoke/e2e_page.html","reason":"html_render_video: 失败","ts":1}"#,
                r#"{"query":"image_url","reason":"vnn_identify: 失败","ts":2}"#,
                r#"{"query":"怎么配置 nginx","reason":"NO_HIT","ts":3}"#,
            ],
        );
        let tasks = digest(&q).unwrap();
        assert_eq!(tasks.len(), 1, "2 个污染项应被过滤, 仅剩真实查询");
        assert_eq!(tasks[0].query, "怎么配置 nginx");
    }

    /// 回归: 短创作请求不得标成 lyv_knowledge —— 否则把 FC V4 修掉的
    /// overcorrection ("起标题/想队名" 误路由 lyv) 又喂回训练集, 闭环自我对抗。
    /// 样本取自真实队列 smoke/pack_merged/learning_queue.jsonl ("队名" ×7)。
    #[test]
    fn digest_routes_creative_nohit_to_llm_generate() {
        let dir = tempfile::tempdir().unwrap();
        let q = write_queue(
            dir.path(),
            &[
                r#"{"query":"队名","reason":"NO_HIT: 知识库没有这个操作","ts":1}"#,
                r#"{"query":"队名","reason":"NO_HIT: 知识库没有这个操作","ts":2}"#,
                r#"{"query":"起个标题","reason":"NO_HIT: 知识库没有这个操作","ts":3}"#,
                r#"{"query":"怎么配置 nginx","reason":"NO_HIT: 知识库没有这个操作","ts":4}"#,
            ],
        );
        let tasks = digest(&q).unwrap();
        let creative = tasks.iter().find(|t| t.query == "队名").unwrap();
        assert_eq!(creative.expected_tool, "llm_generate", "短创作应标 llm_generate");
        assert_eq!(creative.frequency, 2, "重复出现的创作请求 = 强信号, 保留计数");
        let title = tasks.iter().find(|t| t.query == "起个标题").unwrap();
        assert_eq!(title.expected_tool, "llm_generate");
        let know = tasks.iter().find(|t| t.query == "怎么配置 nginx").unwrap();
        assert_eq!(know.expected_tool, "lyv_knowledge", "真知识查询不误伤");
    }

    #[test]
    fn creative_detection_precision() {
        // 命中
        assert!(is_creative_request("队名"));
        assert!(is_creative_request("起个标题"));
        assert!(is_creative_request("帮我起个队名"));
        assert!(is_creative_request("想个笔名"));
        assert!(is_creative_request("写一首诗"));
        // 不误伤: 疑问词开头的知识查询 (含 DESIGN 记录的 "怎么做X" 语言模糊)
        assert!(!is_creative_request("怎么配置 nginx"));
        assert!(!is_creative_request("如何起标题"), "疑问词优先归知识查询 (已知边界)");
        assert!(!is_creative_request("怎么新建 rust 项目"));
        assert!(!is_creative_request("帮我看看这张截图"));
    }
}
