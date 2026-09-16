//! datagen — 训练语料生成 (answer-first / ToolGrad 式)
//!
//! 数据闭环的第二段 (第一段是 `lernen::harvest`):
//!
//! ```text
//! learning_queue.jsonl
//!   → lernen::harvest  → TrainingTask { query, expected_tool(启发式 classify), reward_hint }
//!   → datagen::build_samples  (ToolRAG 语义复核 + 合成规范 tool_call = "answer-first")
//!   → datagen::to_chat_jsonl  (messages + tools 裁剪 schema，喂 GRPO/SFT)
//!   → CloudStudio A10 训练 → 新 FC 模型下发 → 端侧能力增长
//! ```
//!
//! **设计取舍 (诚实记录)**：`lernen::classify` 编码了 FC-V4 的硬仗经验
//! (短创作请求→`llm_generate` 以防 overcorrection)，是**权威**；本模块**不推翻**它，
//! 只把 ToolRAG 作为**语义复核信号** (`rag_confirms`) 挂在样本上，供数据质量监控与
//! 后续用真实语义 embedder 替换时的对照基准。`reverse_gen_seeds` 则补 ToolGrad 式
//! 「少量高质量」程序化种子，覆盖 7 工具全部，避免真实队列偏科 (NO_HIT 只产 lyv/llm)。

use crate::lernen::TrainingTask;
use crate::toolrag::{Embedder, RecallHit, ToolRag};
use serde::Serialize;
use std::path::Path;

/// 一个 answer-first SFT 样本 —— "回答"即规范 tool_call, query 为待逆向学习的问句
#[derive(Debug, Clone, Serialize)]
pub struct SftSample {
    pub query: String,
    /// 权威路由工具 (来自 lernen::classify 或 ToolGrad 种子)
    pub tool: &'static str,
    /// 合成的工具参数 (answer-first 的 "answer" 本体)
    pub arguments: serde_json::Value,
    /// 来源: "learning_queue" | "seed"
    pub source: &'static str,
    /// ToolRAG top-1 是否与权威工具一致 (语义复核, 不参与路由决策)
    pub rag_confirms: bool,
    /// ToolRAG top-1 (name, score), 供监控
    pub rag_top: Option<(String, f64)>,
    pub frequency: usize,
    pub reward_hint: String,
}

/// 由文本 query 合成工具参数。识图/抠图/渲染类需具体文件参数, 文本 query 无法合成 → {}。
fn synth_arguments(tool: &str, query: &str) -> serde_json::Value {
    match tool {
        "lyv_knowledge" => serde_json::json!({ "query": query }),
        "llm_generate" | "html_gen" => serde_json::json!({ "prompt": query }),
        _ => serde_json::json!({}),
    }
}

/// TrainingTask 列表 → answer-first SFT 样本 (ToolRAG 挂复核信号)
pub fn build_samples<E: Embedder>(tasks: &[TrainingTask], rag: &ToolRag<E>) -> Vec<SftSample> {
    let mut out = Vec::with_capacity(tasks.len());
    for t in tasks {
        let top = rag.recall(&t.query, 1).into_iter().next();
        let rag_confirms = top.as_ref().map(|h| h.name == t.expected_tool).unwrap_or(false);
        out.push(SftSample {
            query: t.query.clone(),
            tool: t.expected_tool,
            arguments: synth_arguments(t.expected_tool, &t.query),
            source: "learning_queue",
            rag_confirms,
            rag_top: top.map(|h| (h.name.to_string(), h.score)),
            frequency: t.frequency,
            reward_hint: t.reward_hint.to_string(),
        });
    }
    out
}

/// ToolGrad 式程序化种子 —— 覆盖 7 工具全部, 补真实队列偏科 (NO_HIT 只产 lyv/llm)
pub fn reverse_gen_seeds<E: Embedder>(rag: &ToolRag<E>) -> Vec<SftSample> {
    const SEEDS: &[(&str, &str)] = &[
        ("lyv_knowledge", "怎么用 cargo 新建一个项目"),
        ("lyv_knowledge", "git 如何回滚到上一个提交"),
        ("vnn_identify", "看看这张截图是什么界面"),
        ("rembg_remove", "帮我把这张图片抠图去掉背景"),
        ("html_gen", "生成一个产品落地页"),
        ("llm_generate", "写一段产品介绍文案"),
        ("html_render_video", "把这个网页录制成视频"),
        ("video_info", "这个视频多长"),
    ];
    SEEDS
        .iter()
        .map(|(tool, q)| {
            let top = rag.recall(q, 1).into_iter().next();
            let rag_confirms = top.as_ref().map(|h| h.name == *tool).unwrap_or(false);
            SftSample {
                query: (*q).to_string(),
                tool,
                arguments: synth_arguments(tool, q),
                source: "seed",
                rag_confirms,
                rag_top: top.map(|h| (h.name.to_string(), h.score)),
                frequency: 1,
                reward_hint: "ToolGrad 式 answer-first 种子 (少量高质量)".to_string(),
            }
        })
        .collect()
}

/// 样本 → chat JSONL。每行 = `{messages, tools, meta}`:
///   - `messages[1].content` = `<tool_call>{name,arguments}</tool_call>` (与
///     `executor::parse_call` 的解析格式闭环一致)
///   - `tools` = 该样本工具的**裁剪 schema** (ToolRAG 裁剪, 小模型只看相关工具)
pub fn to_chat_jsonl<E: Embedder>(samples: &[SftSample], rag: &ToolRag<E>) -> String {
    let mut out = String::new();
    for s in samples {
        let tools = rag.to_openai_tools_schema(&[RecallHit {
            name: s.tool,
            score: 1.0,
        }]);
        let call = serde_json::json!({ "name": s.tool, "arguments": s.arguments });
        let assistant = format!("<tool_call>{call}</tool_call>");
        let line = serde_json::json!({
            "messages": [
                { "role": "user", "content": s.query },
                { "role": "assistant", "content": assistant },
            ],
            "tools": tools,
            "meta": {
                "source": s.source,
                "rag_confirms": s.rag_confirms,
                "frequency": s.frequency,
                "reward_hint": s.reward_hint,
            }
        });
        out.push_str(&serde_json::to_string(&line).expect("样本序列化"));
        out.push('\n');
    }
    out
}

/// 写 chat 语料到文件
pub fn write_corpus<E: Embedder>(
    samples: &[SftSample],
    rag: &ToolRag<E>,
    out_path: &Path,
) -> std::io::Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out_path, to_chat_jsonl(samples, rag))
}

/// 端到端: 学习队列 → 语料 (真实队列样本 ∪ ToolGrad 种子), 返回样本数
pub fn datagen<E: Embedder>(
    queue_path: &Path,
    out_path: &Path,
    rag: &ToolRag<E>,
) -> std::io::Result<usize> {
    // 队列不存在 = 无真实失败样本, 仍产出 ToolGrad 种子 (不因缺文件而失败)
    let tasks = if queue_path.exists() {
        crate::lernen::digest(queue_path)?
    } else {
        Vec::new()
    };
    let mut samples = build_samples(&tasks, rag);
    samples.extend(reverse_gen_seeds(rag));
    write_corpus(&samples, rag, out_path)?;
    Ok(samples.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolrag::TokenEmbedder;

    fn rag() -> ToolRag<TokenEmbedder> {
        ToolRag::build(TokenEmbedder::new())
    }

    fn seed_queue(dir: &Path, lines: &[&str]) -> std::path::PathBuf {
        let p = dir.join("learning_queue.jsonl");
        std::fs::write(&p, lines.join("\n")).unwrap();
        p
    }

    #[test]
    fn build_samples_keeps_classify_tool_and_flags_rag() {
        let dir = tempfile::tempdir().unwrap();
        let q = seed_queue(
            dir.path(),
            &[
                r#"{"query":"怎么配置 nginx","reason":"NO_HIT: 知识库没有这个操作","ts":1}"#,
                r#"{"query":"起个标题","reason":"NO_HIT: 知识库没有这个操作","ts":2}"#,
            ],
        );
        let tasks = crate::lernen::digest(&q).unwrap();
        let samples = build_samples(&tasks, &rag());
        assert_eq!(samples.len(), 2);
        let nginx = samples.iter().find(|s| s.query == "怎么配置 nginx").unwrap();
        assert_eq!(nginx.tool, "lyv_knowledge", "权威工具来自 classify");
        assert_eq!(nginx.arguments["query"], "怎么配置 nginx", "answer-first 合成参数");
        assert!(nginx.rag_top.is_some(), "应挂上 ToolRAG 复核信号");
        assert_eq!(nginx.source, "learning_queue");

        let title = samples.iter().find(|s| s.query == "起个标题").unwrap();
        assert_eq!(title.tool, "llm_generate", "创作请求不被 ToolRAG 推翻");
        assert_eq!(title.arguments["prompt"], "起个标题");
    }

    #[test]
    fn rag_confirms_authoritative_tool_for_representative_queries() {
        let r = rag();
        // 权威工具与语义召回一致 (样本质量信号)
        let nginx = r.recall("怎么配置 nginx", 1);
        assert_eq!(nginx[0].name, "lyv_knowledge");
        let title = r.recall("起个标题", 1);
        assert_eq!(title[0].name, "llm_generate");
    }

    #[test]
    fn reverse_gen_seeds_cover_all_seven_tools() {
        let seeds = reverse_gen_seeds(&rag());
        let mut names: Vec<&str> = seeds.iter().map(|s| s.tool).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 7, "种子须覆盖 7 工具, got {names:?}");
        assert!(seeds.iter().all(|s| s.source == "seed"));
    }

    #[test]
    fn chat_jsonl_is_parseable_and_roundtrips_tool_call() {
        let r = rag();
        let dir = tempfile::tempdir().unwrap();
        let q = seed_queue(
            dir.path(),
            &[r#"{"query":"怎么配置 nginx","reason":"NO_HIT","ts":1}"#],
        );
        let tasks = crate::lernen::digest(&q).unwrap();
        let mut samples = build_samples(&tasks, &r);
        samples.extend(reverse_gen_seeds(&r));
        let jsonl = to_chat_jsonl(&samples, &r);

        for line in jsonl.lines() {
            let v: serde_json::Value = serde_json::from_str(line).expect("每行须为合法 JSON");
            let msgs = v["messages"].as_array().unwrap();
            assert_eq!(msgs.len(), 2);
            assert_eq!(msgs[0]["role"], "user");
            let content = msgs[1]["content"].as_str().unwrap();
            assert!(content.contains("<tool_call>"), "assistant 须含 tool_call");
            // 闭环: executor::parse_call 必须能解析回 (name, arguments)
            let (name, args) = crate::executor::parse_call(content).expect("parse_call 应成功");
            assert!(
                crate::skill::ALL_SKILLS.iter().any(|s| s.name == name),
                "解析出的工具名须是已注册技能, got {name}"
            );
            assert!(args.is_object() || args.is_null(), "arguments 应为对象");
            // tools 裁剪后只含该样本工具
            let tools = v["tools"].as_array().unwrap();
            assert_eq!(tools.len(), 1, "裁剪 schema 只含 1 个工具");
            assert_eq!(tools[0]["function"]["name"].as_str().unwrap(), name);
        }
    }

    #[test]
    fn datagen_end_to_end_writes_file() {
        let r = rag();
        let dir = tempfile::tempdir().unwrap();
        let q = seed_queue(
            dir.path(),
            &[r#"{"query":"怎么配置 nginx","reason":"NO_HIT","ts":1}"#],
        );
        let out = dir.path().join("sft_corpus.jsonl");
        let n = datagen(&q, &out, &r).unwrap();
        assert_eq!(n, 1 + reverse_gen_seeds(&r).len(), "真实样本 + 种子");
        let content = std::fs::read_to_string(&out).unwrap();
        assert_eq!(content.lines().count(), n);
        assert!(content.contains("<tool_call>"));
    }
}
