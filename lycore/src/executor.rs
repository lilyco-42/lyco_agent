//! tool executor + 学习队列 — lyco_agent_loop.py exec_tool() 的 Rust 移植
//!
//! 职责:
//!   1. 解析模型生成的 tool_call JSON (宽松: <tool_call> 包裹或裸 JSON)
//!   2. 分发执行: lyv_knowledge → pack::lookup; vnn_identify → 降级学习队列 (VNN Rust 版未实现)
//!   3. NO_HIT → 学习队列落盘 (learning_queue.jsonl, 诚实不编造)
//!
//! 学习队列语义 (DESIGN.md): 全落空 → 诚实「不会」→ 学习队列, 不兜底编造。

use crate::pack::Pack;
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 工具执行结果 — 对应 Python exec_tool 返回的 dict (tool result JSON)
#[derive(Debug, Clone, Serialize)]
pub struct ToolResult {
    pub ok: bool,
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyframe: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strong: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ToolResult {
    fn ok(tool: &str, evidence: &crate::Evidence) -> Self {
        Self {
            ok: true,
            tool: tool.to_string(),
            answer: Some(evidence.text.clone()),
            command: evidence.command.clone(),
            clip: Some(format!("{:.1}s-{:.1}s", evidence.t0, evidence.t1)), // Python f-string 15.0 → "15.0"
            keyframe: Some(evidence.frame.clone()),
            strong: Some(evidence.strong.clone()),
            error: None,
        }
    }
    fn err(tool: &str, msg: &str) -> Self {
        Self {
            ok: false,
            tool: tool.to_string(),
            answer: None,
            command: None,
            clip: None,
            keyframe: None,
            strong: None,
            error: Some(msg.to_string()),
        }
    }
}

/// 模型 tool_call 解析 (宽松两级: <tool_call> 包裹 → 裸 JSON name 字段)
pub fn parse_call(text: &str) -> Option<(String, serde_json::Value)> {
    let candidate = if let Some(m) =
        re_find(text, "<tool_call>", "</tool_call>")
    {
        m
    } else {
        // 裸 JSON: 找第一个 { 到最后一个 }
        let start = text.find('{')?;
        let end = text.rfind('}')? + 1;
        text[start..end].to_string()
    };
    let v: serde_json::Value = serde_json::from_str(&candidate).ok()?;
    let name = v.get("name")?.as_str()?.to_string();
    Some((name, v.get("arguments").cloned().unwrap_or_default()))
}

fn re_find<'a>(text: &'a str, open: &str, close: &str) -> Option<String> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(text[start..end].trim().to_string())
}

/// 学习队列: NO_HIT / 工具不可用时落盘, 供训练管线消费 (FEE: 环境反馈驱动学习)
pub struct LearningQueue {
    path: PathBuf,
}

impl LearningQueue {
    pub fn open(pack_dir: &Path) -> Self {
        Self {
            path: pack_dir.join("learning_queue.jsonl"),
        }
    }

    /// 追加一条学习任务 (query + 原因 + 时间戳)
    pub fn push(&self, query: &str, reason: &str) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let entry = serde_json::json!({
            "query": query,
            "reason": reason,
            "ts": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        });
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{entry}")?;
        Ok(())
    }

    pub fn len(&self) -> usize {
        std::fs::read_to_string(&self.path)
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Agent 工具执行器: 模型 tool_call → 真实执行 → ToolResult
pub struct Executor {
    pack: Pack,
    pack_dir: PathBuf,
    queue: LearningQueue,
}

impl Executor {
    pub fn open(pack_dir: &Path) -> rusqlite::Result<Self> {
        Ok(Self {
            pack: Pack::open(pack_dir)?,
            pack_dir: pack_dir.to_path_buf(),
            queue: LearningQueue::open(pack_dir),
        })
    }

    /// 执行一次 tool_call (VNN 未实现 → 诚实入学习队列)
    pub fn execute(&self, name: &str, arguments: &serde_json::Value) -> ToolResult {
        match name {
            "lyv_knowledge" => {
                let query = arguments
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                match self.pack.lookup(query) {
                    Ok(Some(ev)) => ToolResult::ok(name, &ev),
                    Ok(None) => {
                        // 诚实降级: 入学习队列, 不编造
                        let _ = self.queue.push(
                            query,
                            "NO_HIT: 知识库没有这个操作",
                        );
                        ToolResult::err(
                            name,
                            "NO_HIT: 我还没学会这个操作。请诚实告诉用户你还不会, 已加入学习队列。",
                        )
                    }
                    Err(e) => ToolResult::err(name, &format!("lookup error: {e}")),
                }
            }
            "vnn_identify" => {
                // VNN Rust 版未实现 (Python 原型 vnn_proto 需 opencv/CNN)
                // 诚实降级路径: 不伪装成功
                let _ = self.queue.push(
                    arguments
                        .get("image")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    "vnn_identify: Rust VNN 未实现",
                );
                ToolResult::err(name, "VNN 不可用 (Rust 版未实现), 入学习队列")
            }
            other => ToolResult::err(other, "未知工具"),
        }
    }

    pub fn learning_queue(&self) -> &LearningQueue {
        &self.queue
    }

    pub fn pack_dir(&self) -> &Path {
        &self.pack_dir
    }

    /// 只读检索 (serve 模式用): 直接拿 Evidence, 不走 tool_call 包装
    pub fn pack_lookup(&self, query: &str) -> Option<crate::Evidence> {
        self.pack.lookup(query).ok().flatten()
    }
}
