//! serve — lycore HTTP 常驻服务 (lyco API)
//!
//! 端点:
//!   POST /ask        {"question": "..."}            → 检索/agent 结果 + 学习队列自动落盘
//!   GET  /health                                    → 服务状态
//!   GET  /queue                                     → 学习队列条数
//!   POST /harvest   {"out": "path"}                 → 手动回流 (通常由定时任务调)
//!
//! 设计: 纯检索模式 (无模型) 是缺省 — llama-server 传入 --llama 时升级为完整 agent loop。
//! 单线程 tiny_http 足够端侧 (笔记本/手机热点场景), 并发需求留 Issue。

use crate::agent::Agent;
use crate::executor::{Executor, LearningQueue};
use crate::lernen::harvest;
use crate::llamacpp::LlamaCppBackend;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// /ask 的处理: llama 有 → agent loop; 无 → 纯检索 + 学习队列
fn handle_ask(
    question: &str,
    executor: &Executor,
    queue: &LearningQueue,
    cfg: &ServeConfig,
) -> (u16, serde_json::Value) {
    if let Some(url) = &cfg.llama_url {
        // 完整 agent loop
        let mut backend = LlamaCppBackend::new(url, &cfg.llama_model);
        let agent = Agent::new(executor, 4);
        match agent.run(&mut backend, question) {
            Ok(turn) => (
                200,
                json!({"route": "agent", "rounds": turn.rounds,
                       "answer": turn.answer,
                       "tool_calls": turn.tool_calls,
                       "learning_queue_used": turn.learning_queue_used}),
            ),
            Err(e) => (500, json!({"error": e.to_string()})),
        }
    } else {
        let hit = executor.pack_lookup(question);
        // 纯检索
        if let Some(ev) = hit {
            (200, json!({"route": "retrieval",
                         "intent": ev.intent, "text": ev.text,
                         "t0": ev.t0, "t1": ev.t1,
                         "keyframe": ev.frame}))
        } else {
            let _ = queue.push(question, "NO_HIT: 知识库没有这个操作");
            (200, json!({"route": "learning_queue",
                         "answer": "抱歉，我还没学会这个操作，已加入学习队列。"}))
        }
    }
}

pub struct ServeConfig {
    pub pack_dir: PathBuf,
    pub port: u16,
    pub llama_url: Option<String>,
    pub llama_model: String,
}

pub fn serve(cfg: ServeConfig) -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{}", cfg.port);
    let server = tiny_http::Server::http(&addr).map_err(anyhow::Error::msg)?;
    let executor = Executor::open(&cfg.pack_dir)?;
    let queue = LearningQueue::open(&cfg.pack_dir);
    let served = Arc::new(AtomicUsize::new(0));
    println!("[lycore] serving on http://{addr}  pack={}", cfg.pack_dir.display());
    println!("[lycore] mode: {}",
             if cfg.llama_url.is_some() { "agent+llama" } else { "retrieval-only" });

    loop {
        let mut request = match server.recv() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[lycore] recv error: {e}");
                continue;
            }
        };
        let url = request.url().to_string();
        let method = request.method().to_string();
        let mut body = String::new();
        let _ = request.as_reader().read_to_string(&mut body);

        let (status, payload) = match (method.as_str(), url.as_str()) {
            ("GET", "/health") => (
                200,
                json!({"ok": true, "served": served.load(Ordering::Relaxed)}),
            ),
            ("GET", "/queue") => (
                200,
                json!({"pending": queue.len()}),
            ),
            ("POST", "/harvest") => {
                let out = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v.get("out").and_then(|o| o.as_str()).map(String::from))
                    .unwrap_or_else(|| "training_tasks.jsonl".to_string());
                match harvest(&cfg.pack_dir.join("learning_queue.jsonl"), Path::new(&out)) {
                    Ok(n) => (200, json!({"ok": true, "tasks": n, "out": out})),
                    Err(e) => (500, json!({"ok": false, "error": e.to_string()})),
                }
            }
            ("POST", "/ask") => {
                served.fetch_add(1, Ordering::Relaxed);
                let question = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v.get("question").and_then(|q| q.as_str()).map(String::from));
                match question {
                    None => (400, json!({"error": "需要 question 字段"})),
                    Some(q) => handle_ask(&q, &executor, &queue, &cfg),
                }
            }
            _ => (404, json!({"error": "not found"})),
        };

        let resp = tiny_http::Response::from_string(payload.to_string())
            .with_status_code(status)
            .with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                    .unwrap(),
            );
        let _ = request.respond(resp);
    }
}
