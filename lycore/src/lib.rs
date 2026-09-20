//! lycore — lyco_agent 的 Rust 核心库
//!
//! Python 原型 (tools/runtime/lyv.py) 已验证的检索逻辑移植。原子化构建:
//!   - `pack::Lookup`: LVK 知识包 sqlite 检索 (intent 词典优先, FTS 兜底) — 对应 lyv.lookup
//!   - 后续原子单元: tokenizer (seg_zh 字级/ASCII 词级)、verifier cascade、tool executor
//!
//! 设计对齐 DESIGN.md: 检索证据 (切片+关键帧+OCR) 是产品核心, Rust 化为端侧 agent
//! 提供零依赖快速检索。

pub mod agent;
pub mod capability; // P0: 能力层 (Capability Layer) 一等公民组件
pub mod datagen; // P1: 训练语料生成 (answer-first / ToolGrad 式, 学习队列→SFT 语料)
pub mod executor;
pub mod learn;
pub mod learn_cli;
pub mod lernen;
pub mod llamacpp;
pub mod npu_runtime; // P2: A733 VIP9000 NPU 串行调度器 (租约+优先级队列+超时; 纯设计+mock 单测)
pub mod pack;
pub mod project; // 项目目录扫描 + 启动脚本生成 (识别 paper.jar → 关联知识 → 时间窗启动脚本)
pub mod rewrite;
pub mod search; // agent 自主检索 (SearXNG/离线占位) + 检索结果学习进知识包
pub mod router; // v13 路由器接线 (MVP 能力①): NL -> 命令 -> T1 分诊 -> brush
pub mod serve;
pub mod t1gate; // T1 执行门: 模型层不拒绝(reject 0% 实证), 执行层必须兜底
pub mod skill; // P0: 技能描述符 { capabilities, risk, verifier, executor }
pub mod tokens;
pub mod toolrag; // P1: ToolRAG 语义召回层 (embedding Top-K + 裁剪 tools_openai.json)
pub mod tools_runtime;
pub mod trace; // agent 执行过程事件流 (ndjson) — 回放/可视化/打包三用 (对齐 mpkg trace-as-commits)
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
