//! lycore — lyco_agent 的 Rust 核心库
//!
//! Python 原型 (tools/lyv.py) 已验证的检索逻辑移植。原子化构建:
//!   - `pack::Lookup`: LVK 知识包 sqlite 检索 (intent 词典优先, FTS 兜底) — 对应 lyv.lookup
//!   - 后续原子单元: tokenizer (seg_zh 字级/ASCII 词级)、verifier cascade、tool executor
//!
//! 设计对齐 DESIGN.md: 检索证据 (切片+关键帧+OCR) 是产品核心, Rust 化为端侧 agent
//! 提供零依赖快速检索。

pub mod agent;
pub mod executor;
pub mod lernen;
pub mod llamacpp;
pub mod learn;
pub mod learn_cli;
pub mod pack;
pub mod rewrite;
pub mod serve;
pub mod tokens;
pub mod tools_runtime;
pub mod verify;
pub mod vnn;
pub mod vnn_cnn;

/// 检索结果 — 对应 Python lyv.lookup 返回的 dict
#[derive(Debug, Clone, serde::Serialize)]
pub struct Evidence {
    pub retrieval: String,
    pub intent: String,
    pub command: Option<String>,
    pub t0: f64,
    pub t1: f64,
    pub frame: String,
    pub frame_t: f64,
    pub text: String,
    pub strong: Vec<String>,
    pub weak: Vec<String>,
    pub prereq: Vec<String>,
    pub ocr_conf: f64,
}
