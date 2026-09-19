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
        // 第一跳: 直查
        if let Some(ev) = executor.pack_lookup(question) {
            return (
                200,
                json!({"route": "retrieval",
                                "intent": ev.intent, "text": ev.text,
                                "t0": ev.t0, "t1": ev.t1,
                                "keyframe": ev.frame}),
            );
        }
        // 第二跳: rewrite 专训后端改写 → 二次检索 (分工模型: 改写走独立后端)
        if let Some(rurl) = &cfg.rewrite_url {
            let mut backend = LlamaCppBackend::new(rurl, &cfg.rewrite_model);
            match crate::rewrite::lookup_with_rewrite(executor, &mut backend, question) {
                Ok((Some(ev), rewritten)) => {
                    let route = if rewritten {
                        "retrieval-rewritten"
                    } else {
                        "retrieval"
                    };
                    return (
                        200,
                        json!({"route": route,
                                        "intent": ev.intent, "text": ev.text,
                                        "t0": ev.t0, "t1": ev.t1,
                                        "keyframe": ev.frame}),
                    );
                }
                Ok((None, _)) => { /* 落到学习队列 */ }
                Err(e) => {
                    eprintln!("[lycore] rewrite 跳失败: {e}"); // 改写后端挂了不阻塞主链
                }
            }
        }
        let _ = queue.push(question, "NO_HIT: 知识库没有这个操作");
        (
            200,
            json!({"route": "learning_queue",
                     "answer": "抱歉，我还没学会这个操作，已加入学习队列。"}),
        )
    }
}

pub struct ServeConfig {
    pub pack_dir: PathBuf,
    pub port: u16,
    /// tool_call 决策后端 (agent loop 用) — 分工模型: FC 专训
    pub llama_url: Option<String>,
    pub llama_model: String,
    /// 查询改写后端 (直查失败二跳用) — 分工模型: rewrite 专训
    pub rewrite_url: Option<String>,
    pub rewrite_model: String,
}

pub fn serve(cfg: ServeConfig) -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{}", cfg.port);
    let server = std::sync::Arc::new(tiny_http::Server::http(&addr).map_err(anyhow::Error::msg)?);
    let queue = std::sync::Arc::new(LearningQueue::open(&cfg.pack_dir));
    let served = Arc::new(AtomicUsize::new(0));
    let cfg = Arc::new(cfg);
    println!(
        "[lycore] serving on http://{addr}  pack={}",
        cfg.pack_dir.display()
    );
    println!(
        "[lycore] mode: {}  workers: {}",
        if cfg.llama_url.is_some() {
            "agent+llama"
        } else {
            "retrieval-only"
        },
        WORKERS
    );

    // 多并发: worker 线程池共享 Arc<Server> (tiny_http 官方多线程模式)
    let mut handles = Vec::new();
    for w in 0..WORKERS {
        let server = server.clone();
        let queue = queue.clone();
        let served = served.clone();
        let cfg = cfg.clone();
        // rusqlite Connection 非 Sync → 每 worker 在线程内独立开 Executor
        let pack_dir = cfg.pack_dir.clone();
        handles.push(std::thread::spawn(move || {
            worker_loop(w, server, queue, served, cfg, pack_dir)
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}

const WORKERS: usize = 4;

fn worker_loop(
    id: usize,
    server: Arc<tiny_http::Server>,
    queue: Arc<LearningQueue>,
    served: Arc<AtomicUsize>,
    cfg: Arc<ServeConfig>,
    pack_dir: PathBuf,
) {
    loop {
        let executor = match Executor::open(Path::new(pack_dir.as_os_str())) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[lycore:w{id}] executor open fail: {e}");
                return;
            }
        };
        let mut request = match server.recv() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[lycore:w{id}] recv error: {e}");
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
            ("GET", "/queue") => (200, json!({"pending": queue.len()})),
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
