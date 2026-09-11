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
    fn with_conf(mut self, conf: f64) -> Self {
        self.clip = Some(format!("conf={conf:.2}"));
        self
    }
    fn with_learning_queue(mut self, queue: Vec<String>, _experts: Vec<crate::vnn::ExpertScore>) -> Self {
        if !queue.is_empty() {
            self.strong = Some(queue);
        }
        self
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
                // VNN Rust 版 (crate::vnn): 特征激活式识图, ffmpeg rawvideo 管道
                let image = arguments
                    .get("image")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let ffmpeg = std::env::var("LYV_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
                match crate::vnn::identify(&ffmpeg, Path::new(image)) {
                    Ok((verdict, conf, experts, learning_queue)) => ToolResult {
                        ok: true,
                        tool: name.to_string(),
                        answer: Some(verdict),
                        command: None,
                        clip: None,
                        keyframe: None,
                        strong: None,
                        error: None,
                    }
                    .with_conf(conf)
                    .with_learning_queue(learning_queue, experts),
                    Err(e) => {
                        let _ = self.queue.push(image, "vnn_identify: 执行失败");
                        ToolResult::err(name, &format!("VNN 失败 ({e}), 入学习队列"))
                    }
                }
            }
            "rembg_remove" => {
                let input = arguments.get("image").and_then(|v| v.as_str()).unwrap_or("");
                let output = arguments
                    .get("output")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if input.is_empty() || output.is_empty() {
                    ToolResult::err(name, "参数需 {\"image\": \"输入图\", \"output\": \"输出png\"}")
                } else {
                    let o = crate::tools_runtime::rembg_remove(Path::new(input), Path::new(output));
                    if o.ok {
                        ToolResult { ok: true, tool: name.to_string(), answer: Some(o.summary), command: None, clip: None, keyframe: None, strong: None, error: None }
                    } else {
                        let _ = self.queue.push(input, "rembg_remove: 执行失败");
                        ToolResult::err(name, &o.summary)
                    }
                }
            }
            "html_gen" => {
                let prompt = arguments.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
                let out = arguments.get("output").and_then(|v| v.as_str()).unwrap_or("");
                if prompt.is_empty() || out.is_empty() {
                    ToolResult::err(name, "参数需 {\"prompt\": \"页面描述\", \"output\": \"输出.html 路径\"}")
                } else {
                    let full = format!("请生成完整单文件 HTML (内联 CSS/JS, 无外部依赖)。需求: {prompt}\n只输出 HTML 代码本身。");
                    let o = crate::tools_runtime::llm_generate(&full, Some(Path::new(out)));
                    if o.ok {
                        ToolResult { ok: true, tool: name.to_string(), answer: Some(o.summary), command: None, clip: None, keyframe: None, strong: None, error: None }
                    } else {
                        let _ = self.queue.push(prompt, "html_gen: LLM 调用失败");
                        ToolResult::err(name, &o.summary)
                    }
                }
            }
            "llm_generate" => {
                let prompt = arguments.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
                if prompt.is_empty() {
                    ToolResult::err(name, "参数需 {\"prompt\": \"...\"}")
                } else {
                    let o = crate::tools_runtime::llm_generate(prompt, None);
                    if o.ok {
                        ToolResult { ok: true, tool: name.to_string(), answer: Some(o.summary), command: None, clip: None, keyframe: None, strong: None, error: None }
                    } else {
                        ToolResult::err(name, &o.summary)
                    }
                }
            }
            "html_render_video" => {
                let html = arguments.get("html").and_then(|v| v.as_str()).unwrap_or("");
                let out = arguments.get("output").and_then(|v| v.as_str()).unwrap_or("");
                let secs = arguments.get("seconds").and_then(|v| v.as_u64()).unwrap_or(3).min(30) as u32;
                if html.is_empty() || out.is_empty() {
                    ToolResult::err(name, "参数需 {\"html\": \"页面路径\", \"output\": \"输出.mp4\", \"seconds\": 3}")
                } else {
                    let chrome = std::env::var("LYCO_CHROME").unwrap_or_else(|_| "chrome".into());
                    let o = crate::tools_runtime::html_render_video(Path::new(html), Path::new(out), secs, &chrome);
                    if o.ok {
                        ToolResult { ok: true, tool: name.to_string(), answer: Some(o.summary), command: None, clip: None, keyframe: None, strong: None, error: None }
                    } else {
                        let _ = self.queue.push(html, "html_render_video: 执行失败");
                        ToolResult::err(name, &o.summary)
                    }
                }
            }
            "video_info" => {
                let video = arguments.get("video").and_then(|v| v.as_str()).unwrap_or("");
                if video.is_empty() {
                    ToolResult::err(name, "参数需 {\"video\": \"路径\"}")
                } else {
                    let o = crate::tools_runtime::video_info(Path::new(video));
                    if o.ok {
                        ToolResult { ok: true, tool: name.to_string(), answer: Some(o.summary), command: None, clip: None, keyframe: None, strong: None, error: None }
                    } else {
                        ToolResult::err(name, &o.summary)
                    }
                }
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
