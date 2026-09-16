//! trace — agent 执行过程的事件流 (ndjson)，供**回放 / 可视化 / 打包**三用
//!
//! 设计对齐两处既有约定：
//!   - `mpkg` spec §6.5 `trace as commits`：**一步 = 一条 trace = 一个 jj commit**；
//!   - `mpkg` spec §8 `provenance`：**prompt 谱系与每步溯源**（"编曲注记"）。
//!
//! 一行一条 ndjson，5 种事件：
//! ```json
//! {"kind":"prompt","i":1,"t_ms":0,"text":"帮我写贪吃蛇"}
//! {"kind":"tool","i":2,"t_ms":800,"name":"shell_exec","arg":"cargo new snake","ok":true}
//! {"kind":"code","i":3,"t_ms":1500,"file":"src/main.rs","diff":"+fn main()"}
//! {"kind":"revert","i":4,"t_ms":3000,"why":"编译失败: 缺 use 语句"}
//! {"kind":"final","i":5,"t_ms":4200,"answer":"完成"}
//! ```
//! `revert` 是**编曲的灵魂**：只记成功步骤 = 成品；记下试错 = 编曲。

use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// 一条 trace 事件 (ndjson 行)
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraceEvent {
    /// 用户意图 (provenance 起点)
    Prompt { i: u64, t_ms: u64, text: String },
    /// 一次工具调用 (对应 mpkg 的一个 step / 一个 jj commit)
    Tool {
        i: u64,
        t_ms: u64,
        name: String,
        arg: String,
        ok: bool,
    },
    /// 一次代码写入 (可视化时按行显示的那行代码)
    Code {
        i: u64,
        t_ms: u64,
        file: String,
        diff: String,
    },
    /// 一次回退/试错 (编曲注记: 为什么推翻上一步)
    Revert { i: u64, t_ms: u64, why: String },
    /// 收束
    Final { i: u64, t_ms: u64, answer: String },
}

/// trace 写入器: 边执行边追加 (pigma 可 `tail -f` 直播)
pub struct Tracer {
    path: PathBuf,
    i: u64,
    t0: Instant,
}

impl Tracer {
    pub fn new(path: &Path) -> Self {
        if let Some(p) = path.parent() {
            if !p.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(p);
            }
        }
        // 截断重开: 一次会话一条 trace
        let _ = std::fs::write(path, b"");
        Self {
            path: path.to_path_buf(),
            i: 0,
            t0: Instant::now(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 当前事件序号 (已累计条数)
    pub fn count(&self) -> u64 {
        self.i
    }

    fn emit(&mut self, ev: TraceEvent) {
        self.i += 1;
        // 每次 flush: pigma `tail -f` 才能实时看到 (不做缓冲)
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            if let Ok(line) = serde_json::to_string(&ev) {
                let _ = writeln!(f, "{line}");
                let _ = f.flush();
            }
        }
    }

    /// t_ms 用相对上一步的毫秒数 (回放的"节拍")
    fn now_ms(&self) -> u64 {
        self.t0.elapsed().as_millis() as u64
    }

    pub fn prompt(&mut self, text: &str) {
        let (i, t) = (self.i + 1, self.now_ms());
        self.emit(TraceEvent::Prompt {
            i,
            t_ms: t,
            text: text.to_string(),
        });
    }

    pub fn tool(&mut self, name: &str, arg: &str, ok: bool) {
        let (i, t) = (self.i + 1, self.now_ms());
        self.emit(TraceEvent::Tool {
            i,
            t_ms: t,
            name: name.to_string(),
            arg: arg.to_string(),
            ok,
        });
    }

    pub fn code(&mut self, file: &str, diff: &str) {
        let (i, t) = (self.i + 1, self.now_ms());
        self.emit(TraceEvent::Code {
            i,
            t_ms: t,
            file: file.to_string(),
            diff: diff.to_string(),
        });
    }

    pub fn revert(&mut self, why: &str) {
        let (i, t) = (self.i + 1, self.now_ms());
        self.emit(TraceEvent::Revert {
            i,
            t_ms: t,
            why: why.to_string(),
        });
    }

    pub fn final_answer(&mut self, answer: &str) {
        let (i, t) = (self.i + 1, self.now_ms());
        self.emit(TraceEvent::Final {
            i,
            t_ms: t,
            answer: answer.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracer_writes_ndjson_events() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("sub/x.trace.ndjson");
        let mut tr = Tracer::new(&p);
        tr.prompt("写贪吃蛇");
        tr.tool("shell_exec", "cargo new snake", true);
        tr.revert("编译失败: 缺 use");
        tr.final_answer("完成");
        assert_eq!(tr.count(), 4);

        let raw = std::fs::read_to_string(&p).unwrap();
        let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 4, "每事件一行");
        // 每行合法 JSON 且带 kind/i/t_ms
        for (n, l) in lines.iter().enumerate() {
            let v: serde_json::Value = serde_json::from_str(l).expect("ndjson 行须合法");
            assert_eq!(v["i"].as_u64().unwrap(), n as u64 + 1, "序号递增");
            assert!(v["t_ms"].is_u64());
            assert!(v["kind"].is_string());
        }
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(lines[1]).unwrap()["kind"],
            "tool"
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(lines[2]).unwrap()["kind"],
            "revert"
        );
        // 工具事件的字段齐全 (可视化需要 name/arg/ok)
        let tool = serde_json::from_str::<serde_json::Value>(lines[1]).unwrap();
        assert_eq!(tool["name"], "shell_exec");
        assert_eq!(tool["arg"], "cargo new snake");
        assert_eq!(tool["ok"], true);
    }

    #[test]
    fn tracer_truncates_on_reopen() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("x.ndjson");
        let mut a = Tracer::new(&p);
        a.prompt("first");
        let mut b = Tracer::new(&p); // 重开 = 新会话
        b.prompt("second");
        let raw = std::fs::read_to_string(&p).unwrap();
        assert_eq!(raw.lines().count(), 1, "重开应截断旧 trace");
        assert!(raw.contains("second"));
        assert_eq!(b.count(), 1, "序号从 1 重新开始");
    }
}
