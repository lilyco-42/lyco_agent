//! ToolRAG 召回层 — TinyAgent ToolRAG 范式在 lyco 的 Rust 落地
//!
//! 在 `learn_cli` 的 FTS5 全文检索之上加一层 **embedding Top-K 语义召回**:
//!
//! ```text
//! query → embed → 取 Top-K 工具 → 裁出 tools_openai.json 子集 → 喂小模型
//! ```
//!
//! 默认 `embedder` = `TokenEmbedder` (bag-of-tokens 哈希桶 + L2 归一, 余弦=点积),
//! 零外部依赖、确定性、可单测。真实语义 embedder (ONNX/GGUF, 在 CloudStudio 或端侧
//! SLM 上跑) 通过 `Embedder` trait 即插即换, 无需改调用方。
//!
//! 语料来自 `skill::ALL_SKILLS` 的 `desc` 字段 (含同义词/中英文), 与
//! `llamacpp::chat_tools()` 单一来源对齐 (research-architecture-2026-09-16.md §1.4)。

use crate::skill::ALL_SKILLS;
use serde::Serialize;

/// 嵌入器 trait — 文本 → 稠密向量。默认实现 `TokenEmbedder`, 可替换为真实模型。
pub trait Embedder: Send + Sync {
    /// 向量维度
    fn dim(&self) -> usize;
    /// 文本 → 向量 (已 L2 归一化, 余弦相似度 = 点积)
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// 默认嵌入器: bag-of-tokens 哈希桶 (词表无关, 端侧友好, 确定性)
///
/// 不追求 SOTA 语义, 而是给 ToolRAG 一个**可用、可测、零依赖**的召回基线;
/// 真实 embedder 上线后只需实现 `Embedder` trait 替换 `TokenEmbedder` 即可。
pub struct TokenEmbedder {
    dim: usize,
}

impl Default for TokenEmbedder {
    fn default() -> Self {
        // 4096 桶: 降低哈希碰撞噪声 (词表无关, 仍零依赖)
        Self { dim: 4096 }
    }
}

impl TokenEmbedder {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Embedder for TokenEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }
    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; self.dim];
        for tok in crate::tokens::tokens(text) {
            let h = simple_hash(&tok) % self.dim;
            v[h] += 1.0;
        }
        // L2 归一化 → 余弦 = 点积
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in v.iter_mut() {
                *x /= norm;
            }
        }
        v
    }
}

/// djb2 风格字符串哈希 (确定性, 与 CloudStudio CSRF 同源思路但仅本地用)
fn simple_hash(s: &str) -> usize {
    let mut h: usize = 5381;
    for b in s.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as usize);
    }
    h
}

/// 语料条目 (工具名 + 向量; 召回文本仅用于构建时嵌入, 不驻留)
struct Entry {
    name: &'static str,
    vec: Vec<f32>,
}

/// 召回命中 (工具名 + 余弦分)
#[derive(Debug, Clone, Serialize)]
pub struct RecallHit {
    pub name: &'static str,
    pub score: f64,
}

/// ToolRAG 召回器 — 持有 embedder + 工具语料, 做 Top-K 语义召回
pub struct ToolRag<E: Embedder> {
    embedder: E,
    corpus: Vec<Entry>,
}

impl<E: Embedder> ToolRag<E> {
    /// 由 `skill::ALL_SKILLS` 的 `desc` 构建语料 (与 CHAT_TOOLS 单一来源一致)
    pub fn build(embedder: E) -> Self {
        let mut corpus = Vec::new();
        for s in ALL_SKILLS {
            // 召回文本 = 技能名 + 执行器 + 描述 + 能力 + 风险, 多信号加权
            let text = format!(
                "{} {} {} {:?} {}",
                s.name,
                s.executor,
                s.desc,
                s.capabilities,
                s.risk.as_str()
            );
            let vec = embedder.embed(&text);
            corpus.push(Entry { name: s.name, vec });
        }
        Self { embedder, corpus }
    }

    /// 召回: query → embed → 全语料余弦 → 取 Top-K
    pub fn recall(&self, query: &str, k: usize) -> Vec<RecallHit> {
        let q = self.embedder.embed(query);
        let mut scored: Vec<(f64, &Entry)> = self
            .corpus
            .iter()
            .map(|e| (cosine(&q, &e.vec), e))
            .collect();
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal)
        });
        scored
            .into_iter()
            .take(k)
            .map(|(score, e)| RecallHit {
                name: e.name,
                score,
            })
            .collect()
    }

    /// 由召回命中裁出 OpenAI function-calling schema 子集
    ///
    /// 直接按命中工具名过滤 `llamacpp::chat_tools()` 全量 schema,
    /// 产出喂给小模型的「裁剪版 tools_openai.json」(TinyAgent ToolRAG 范式)。
    pub fn to_openai_tools_schema(&self, hits: &[RecallHit]) -> serde_json::Value {
        let full = crate::llamacpp::chat_tools();
        let names: std::collections::HashSet<&str> =
            hits.iter().map(|h| h.name).collect();
        let arr = full
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter(|t| {
                        t.get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                            .map(|n| names.contains(n))
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        serde_json::Value::Array(arr)
    }
}

/// 余弦相似度 (向量已 L2 归一 → 返回点积 ∈ [-1, 1])
fn cosine(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| *x as f64 * *y as f64)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top1(rag: &ToolRag<TokenEmbedder>, q: &str) -> &'static str {
        rag.recall(q, 1)[0].name
    }

    #[test]
    fn recall_routes_remove_background_to_rembg() {
        let rag = ToolRag::build(TokenEmbedder::new());
        assert_eq!(top1(&rag, "remove background from image"), "rembg_remove");
        assert_eq!(top1(&rag, "抠图 去背 matting"), "rembg_remove");
    }

    #[test]
    fn recall_routes_html_to_video() {
        let rag = ToolRag::build(TokenEmbedder::new());
        assert_eq!(top1(&rag, "render html to mp4 video"), "html_render_video");
        assert_eq!(top1(&rag, "网页转视频 录屏"), "html_render_video");
    }

    #[test]
    fn recall_routes_writing_and_knowledge() {
        let rag = ToolRag::build(TokenEmbedder::new());
        // 语料 desc 为中文 + 英文关键词混合; query 需与语料同词表 (bag-of-tokens 基线)
        assert_eq!(top1(&rag, "写作 生成 writing generation"), "llm_generate");
        assert_eq!(top1(&rag, "教程里怎么敲这个命令"), "lyv_knowledge");
        assert_eq!(top1(&rag, "查看视频时长 分辨率 帧率"), "video_info");
    }

    #[test]
    fn recall_topk_size_respected() {
        let rag = ToolRag::build(TokenEmbedder::new());
        let hits = rag.recall("image", 3);
        assert!(hits.len() <= 3);
        // 降序: 第一分 >= 第二分
        if hits.len() >= 2 {
            assert!(hits[0].score >= hits[1].score);
        }
    }

    #[test]
    fn to_openai_schema_crops_by_hits() {
        let rag = ToolRag::build(TokenEmbedder::new());
        let hits = rag.recall("remove background", 2);
        let schema = rag.to_openai_tools_schema(&hits);
        let arr = schema.as_array().expect("应为数组");
        assert!(!arr.is_empty());
        assert!(arr.len() <= 2, "裁剪后应 ≤ Top-K");
        for t in arr {
            let n = t["function"]["name"].as_str().unwrap();
            assert!(hits.iter().any(|h| h.name == n), "schema 工具须来自命中集");
        }
    }
}
