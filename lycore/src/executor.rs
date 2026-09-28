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
    fn with_learning_queue(
        mut self,
        queue: Vec<String>,
        _experts: Vec<crate::vnn::ExpertScore>,
    ) -> Self {
        if !queue.is_empty() {
            self.strong = Some(queue);
        }
        self
    }
}

/// 模型 tool_call 解析 (宽松两级: <tool_call> 包裹 → 裸 JSON name 字段)
pub fn parse_call(text: &str) -> Option<(String, serde_json::Value)> {
    let candidate = if let Some(m) = re_find(text, "<tool_call>", "</tool_call>") {
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

fn re_find(text: &str, open: &str, close: &str) -> Option<String> {
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

/// mpkg 包目录解析：优先知识包下的 `packs/`，其次 exe 同级 `packs/`（分发 zip 布局）。
fn resolve_packs_dir(pack_dir: &Path) -> Option<PathBuf> {
    let mut cands = vec![pack_dir.join("packs")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            cands.push(parent.join("packs"));
        }
    }
    cands.into_iter().find(|c| c.is_dir())
}

/// 扫描包目录 → [{name, intent, version}]（坏包跳过 —— 宁少报不报垃圾）。
fn list_packs(dir: &Path) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let manifest_path = entry.path().join(crate::mpkg::MANIFEST);
        if !manifest_path.is_file() {
            continue;
        }
        let raw = std::fs::read_to_string(&manifest_path)?;
        let m: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue, // 坏清单跳过
        };
        out.push(serde_json::json!({
            "name": m.get("name").cloned().unwrap_or_default(),
            "intent": m.get("intent").cloned().unwrap_or_default(),
            "version": m.get("version").cloned().unwrap_or_default(),
        }));
    }
    Ok(out)
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
                        let _ = self.queue.push(query, "NO_HIT: 知识库没有这个操作");
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
                let input = arguments
                    .get("image")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let output = arguments
                    .get("output")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if input.is_empty() || output.is_empty() {
                    ToolResult::err(
                        name,
                        "参数需 {\"image\": \"输入图\", \"output\": \"输出png\"}",
                    )
                } else {
                    let o = crate::tools_runtime::rembg_remove(Path::new(input), Path::new(output));
                    if o.ok {
                        ToolResult {
                            ok: true,
                            tool: name.to_string(),
                            answer: Some(o.summary),
                            command: None,
                            clip: None,
                            keyframe: None,
                            strong: None,
                            error: None,
                        }
                    } else {
                        let _ = self.queue.push(input, "rembg_remove: 执行失败");
                        ToolResult::err(name, &o.summary)
                    }
                }
            }
            "mpkg_list" => {
                // 记忆包清单: 让模型「看见」自己有哪些已固化的技能包 (trace→mpkg 闭环的
                // 消费端)。没有包 = 诚实回答, 不编造。
                match resolve_packs_dir(&self.pack_dir) {
                    None => ToolResult::err(
                        name,
                        "NO_PACKS: 没有找到 packs 目录。请诚实告诉用户当前没有可用记忆包。",
                    ),
                    Some(dir) => match list_packs(&dir) {
                        Ok(list) if list.is_empty() => {
                            ToolResult::err(name, "NO_PACKS: packs 目录存在但没有可用记忆包。")
                        }
                        Ok(list) => {
                            let json = serde_json::to_string(&list).unwrap_or_else(|_| "[]".into());
                            ToolResult {
                                ok: true,
                                tool: name.to_string(),
                                answer: Some(json),
                                command: None,
                                clip: None,
                                keyframe: None,
                                strong: None,
                                error: None,
                            }
                        }
                        Err(e) => ToolResult::err(name, &format!("mpkg_list error: {e}")),
                    },
                }
            }
            "mpkg_run" => {
                // 记忆包回放: 模型按需求选中包 → 确定性执行 (verify_dir 语义:
                // 清单校验 + content-id 复算 + steps 逐步执行)。结果原样回喂模型转述。
                let pack = arguments
                    .get("pack")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if pack.is_empty() {
                    return ToolResult::err(name, "参数需 {\"pack\": \"包名(如 lab-python-env)\"}");
                }
                let dir = match resolve_packs_dir(&self.pack_dir) {
                    Some(d) => d.join(&pack),
                    None => {
                        return ToolResult::err(name, "NO_PACKS: 没有找到 packs 目录。");
                    }
                };
                if !dir.join("mpkg.json").is_file() {
                    let avail = resolve_packs_dir(&self.pack_dir)
                        .and_then(|d| list_packs(&d).ok())
                        .map(|l| {
                            l.iter()
                                .filter_map(|p| p.get("name").and_then(|v| v.as_str()))
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    let _ = self.queue.push(&pack, "mpkg_run: 包不存在");
                    return ToolResult::err(name, &format!("包不存在: {pack}。可用: [{avail}]"));
                }
                match crate::mpkg::verify_dir(&dir) {
                    Ok(att) => {
                        let ok = att["ok"] == true;
                        if !ok {
                            let _ = self.queue.push(&pack, "mpkg_run: 回放失败");
                        }
                        // 回放结论原样喂模型 (ok/steps/error) —— 模型负责转述, 不负责判定
                        ToolResult {
                            ok,
                            tool: name.to_string(),
                            answer: Some(att.to_string()),
                            command: None,
                            clip: None,
                            keyframe: None,
                            strong: None,
                            error: if ok {
                                None
                            } else {
                                att["error"].as_str().map(|e| e.to_string())
                            },
                        }
                    }
                    Err(e) => {
                        let _ = self.queue.push(&pack, "mpkg_run: 回放无法开始");
                        ToolResult::err(name, &format!("回放失败: {e:#}"))
                    }
                }
            }
            "html_gen" => {
                let prompt = arguments
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let out = arguments
                    .get("output")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if prompt.is_empty() || out.is_empty() {
                    ToolResult::err(
                        name,
                        "参数需 {\"prompt\": \"页面描述\", \"output\": \"输出.html 路径\"}",
                    )
                } else {
                    let full = format!("请生成完整单文件 HTML (内联 CSS/JS, 无外部依赖)。需求: {prompt}\n只输出 HTML 代码本身。");
                    let o = crate::tools_runtime::llm_generate(&full, Some(Path::new(out)));
                    if o.ok {
                        ToolResult {
                            ok: true,
                            tool: name.to_string(),
                            answer: Some(o.summary),
                            command: None,
                            clip: None,
                            keyframe: None,
                            strong: None,
                            error: None,
                        }
                    } else {
                        let _ = self.queue.push(prompt, "html_gen: LLM 调用失败");
                        ToolResult::err(name, &o.summary)
                    }
                }
            }
            "llm_generate" => {
                let prompt = arguments
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if prompt.is_empty() {
                    ToolResult::err(name, "参数需 {\"prompt\": \"...\"}")
                } else {
                    let o = crate::tools_runtime::llm_generate(prompt, None);
                    if o.ok {
                        ToolResult {
                            ok: true,
                            tool: name.to_string(),
                            answer: Some(o.summary),
                            command: None,
                            clip: None,
                            keyframe: None,
                            strong: None,
                            error: None,
                        }
                    } else {
                        ToolResult::err(name, &o.summary)
                    }
                }
            }
            "html_render_video" => {
                let html = arguments.get("html").and_then(|v| v.as_str()).unwrap_or("");
                let out = arguments
                    .get("output")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let secs = arguments
                    .get("seconds")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3)
                    .min(30) as u32;
                if html.is_empty() || out.is_empty() {
                    ToolResult::err(
                        name,
                        "参数需 {\"html\": \"页面路径\", \"output\": \"输出.mp4\", \"seconds\": 3}",
                    )
                } else {
                    let chrome = std::env::var("LYCO_CHROME").unwrap_or_else(|_| "chrome".into());
                    let o = crate::tools_runtime::html_render_video(
                        Path::new(html),
                        Path::new(out),
                        secs,
                        &chrome,
                    );
                    if o.ok {
                        ToolResult {
                            ok: true,
                            tool: name.to_string(),
                            answer: Some(o.summary),
                            command: None,
                            clip: None,
                            keyframe: None,
                            strong: None,
                            error: None,
                        }
                    } else {
                        let _ = self.queue.push(html, "html_render_video: 执行失败");
                        ToolResult::err(name, &o.summary)
                    }
                }
            }
            "video_info" => {
                let video = arguments
                    .get("video")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if video.is_empty() {
                    ToolResult::err(name, "参数需 {\"video\": \"路径\"}")
                } else {
                    let o = crate::tools_runtime::video_info(Path::new(video));
                    if o.ok {
                        ToolResult {
                            ok: true,
                            tool: name.to_string(),
                            answer: Some(o.summary),
                            command: None,
                            clip: None,
                            keyframe: None,
                            strong: None,
                            error: None,
                        }
                    } else {
                        ToolResult::err(name, &o.summary)
                    }
                }
            }
            "shell_exec" => {
                let command = arguments
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let cwd = arguments.get("cwd").and_then(|v| v.as_str());
                if command.is_empty() {
                    ToolResult::err(name, "参数需 {\"command\": \"...\"} (可选 cwd)")
                } else {
                    let o = crate::tools_runtime::shell_exec(command, cwd.map(Path::new));
                    if o.ok {
                        ToolResult {
                            ok: true,
                            tool: name.to_string(),
                            answer: Some(o.summary),
                            command: Some(command.to_string()),
                            clip: None,
                            keyframe: None,
                            strong: None,
                            error: None,
                        }
                    } else {
                        let _ = self.queue.push(command, "shell_exec: 执行失败");
                        ToolResult::err(name, &o.summary)
                    }
                }
            }
            "file_write" => {
                let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let content = arguments
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if path.is_empty() {
                    ToolResult::err(name, "参数需 {\"path\": \"...\", \"content\": \"...\"}")
                } else {
                    let o = crate::tools_runtime::file_write(Path::new(path), content);
                    if o.ok {
                        ToolResult {
                            ok: true,
                            tool: name.to_string(),
                            answer: Some(o.summary),
                            command: None,
                            clip: None,
                            keyframe: None,
                            strong: None,
                            error: None,
                        }
                    } else {
                        let _ = self.queue.push(path, "file_write: 执行失败");
                        ToolResult::err(name, &o.summary)
                    }
                }
            }
            "schedule" => {
                let spec = arguments.get("spec").and_then(|v| v.as_str()).unwrap_or("");
                let command = arguments
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let apply = arguments
                    .get("apply")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if spec.is_empty() || command.is_empty() {
                    ToolResult::err(
                        name,
                        "参数需 {\"spec\": \"0 8 * * *\", \"command\": \"...\", \"apply\": false}",
                    )
                } else {
                    let o = crate::tools_runtime::schedule(spec, command, apply);
                    if o.ok {
                        ToolResult {
                            ok: true,
                            tool: name.to_string(),
                            answer: Some(o.summary),
                            command: Some(format!("{spec} {command}")),
                            clip: None,
                            keyframe: None,
                            strong: None,
                            error: None,
                        }
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

#[cfg(test)]
mod mpkg_tools_tests {
    use super::*;

    /// 建临时布局: 复制 smoke/pack_final (只读 sqlite 必须随目录带过来),
    /// 再塞一个真 mpkg 包 packs/echo-demo (echo 步骤)。
    fn setup() -> PathBuf {
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../smoke/pack_final");
        let root = std::env::temp_dir().join(format!("lyco-exe-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let pack = root.join("pack");
        std::fs::create_dir_all(&pack).expect("建 pack 目录");
        copy_dir(&src, &pack);
        std::fs::create_dir_all(pack.join("packs/echo-demo")).expect("建 packs 目录");
        std::fs::write(
            pack.join("packs/echo-demo/mpkg.json"),
            r#"{
  "mpkg": "0.1",
  "name": "echo-demo",
  "version": "0.1.0",
  "intent": "executor mpkg 工具测试包",
  "steps": [ { "run": "echo lyco-mpkg-ok" } ],
  "verify": [ "true" ]
}"#,
        )
        .expect("写 manifest");
        root.join("pack")
    }

    fn copy_dir(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).expect("copy_dir mkdir");
        for entry in std::fs::read_dir(src).expect("copy_dir readdir") {
            let entry = entry.expect("copy_dir entry");
            let to = dst.join(entry.file_name());
            if entry.file_type().expect("ft").is_dir() {
                copy_dir(&entry.path(), &to);
            } else {
                std::fs::copy(entry.path(), &to).expect("copy_dir file");
            }
        }
    }

    #[test]
    fn executor_mpkg_list_and_run() {
        let pack = setup();
        let ex = Executor::open(&pack).expect("Executor::open");

        // list: 模型能「看见」包
        let r = ex.execute("mpkg_list", &serde_json::json!({}));
        assert!(r.ok, "mpkg_list 应成功: {:?}", r.error);
        let answer = r.answer.as_deref().unwrap_or("");
        assert!(answer.contains("echo-demo"), "清单应含包名: {answer}");
        assert!(
            answer.contains("executor mpkg 工具测试包"),
            "清单应含 intent"
        );

        // run: 真回放 (echo 步骤真执行)
        let r = ex.execute("mpkg_run", &serde_json::json!({ "pack": "echo-demo" }));
        assert!(r.ok, "mpkg_run 应成功: {:?}", r.error);
        let answer = r.answer.as_deref().unwrap_or("");
        assert!(answer.contains("\"ok\":true"), "回放结论 ok=true: {answer}");

        // run 不存在的包: 诚实报错并列出可用项 (不编造)
        let r = ex.execute("mpkg_run", &serde_json::json!({ "pack": "no-such-pack" }));
        assert!(!r.ok, "不存在的包必须失败");
        let err = r.error.as_deref().unwrap_or("");
        assert!(
            err.contains("包不存在") && err.contains("echo-demo"),
            "报错应点名可用包: {err}"
        );

        let _ = std::fs::remove_dir_all(&pack);
    }
}
