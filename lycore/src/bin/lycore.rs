//! lycore CLI — 一键起 lyco agent
//!
//! 用法:
//!   lycore ask    --pack <dir> [--llama <url>] "<问题>"   单次问答 (无 llama-server 时走仅检索模式)
//!   lycore harvest --pack <dir> [--out <file>]            学习队列 → 训练任务回流
//!   lycore doctor  --pack <dir>                           检查知识包/服务/队列状态
//!
//! 无 llama-server 时的 ask: 纯检索模式 — 直接 lyv lookup + grounding, 证明
//! 知识包可用性 (也是学习队列 NO_HIT 的来源之一)。

use lycore::agent::Agent;
use lycore::executor::Executor;
use lycore::lernen::harvest;
use lycore::llamacpp::LlamaCppBackend;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let exit = match cmd {
        "ask" => cmd_ask(&args[1..]),
        "learn" => cmd_learn(&args[1..]),
        "harvest" => cmd_harvest(&args[1..]),
        "doctor" => cmd_doctor(&args[1..]),
        _ => {
            eprintln!(
                "lycore — lyco agent runtime\n\n\
                 用法:\n  \
                 lycore ask    --pack <dir> [--llama <url>] \"<问题>\"\n  \
                 lycore learn  --video <mp4> --srt <srt> --pack <out_dir>\n  \
                 lycore harvest --pack <dir> [--out <file>]\n  \
                 lycore doctor --pack <dir>"
            );
            2
        }
    };
    std::process::exit(exit);
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn cmd_ask(args: &[String]) -> i32 {
    let Some(pack) = flag(args, "--pack") else {
        eprintln!("缺少 --pack <dir>");
        return 2;
    };
    let question = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .last()
        .unwrap_or_default();
    if question.is_empty() {
        eprintln!("缺少问题");
        return 2;
    }

    let Ok(executor) = Executor::open(std::path::Path::new(&pack)) else {
        eprintln!("知识包打开失败: {pack}");
        return 1;
    };

    // 有 llama-server → 完整 agent loop; 否则纯检索模式
    if let Some(url) = flag(args, "--llama") {
        let model = flag(args, "--model").unwrap_or_else(|| "qwen3-0.6b".into());
        let mut backend = LlamaCppBackend::new(&url, &model);
        if let Err(e) = backend.health() {
            eprintln!("llama-server 不可用: {e}");
            return 1;
        }
        let agent = Agent::new(&executor, 4);
        match agent.run(&mut backend, &question) {
            Ok(turn) => {
                println!("{}", serde_json::to_string_pretty(&turn).unwrap());
                0
            }
            Err(e) => {
                eprintln!("agent 失败: {e}");
                1
            }
        }
    } else {
        // 仅检索: 快速验证知识包 + 记录学习队列
        let tools = lycore::pack::RULES;
        let _ = tools;
        match lycore::pack::Pack::open(std::path::Path::new(&pack))
            .and_then(|p| p.lookup(&question))
        {
            Ok(Some(ev)) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "route": "retrieval",
                        "intent": ev.intent,
                        "clip": format!("{:.1}s-{:.1}s", ev.t0, ev.t1),
                        "keyframe": ev.frame,
                        "text": ev.text,
                        "strong": ev.strong,
                    }))
                    .unwrap()
                );
                0
            }
            Ok(None) => {
                let q = lycore::executor::LearningQueue::open(std::path::Path::new(&pack));
                let _ = q.push(&question, "NO_HIT: 知识库没有这个操作");
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "route": "learning_queue",
                        "answer": "抱歉，我还没学会这个操作，已加入学习队列。"
                    }))
                    .unwrap()
                );
                0
            }
            Err(e) => {
                eprintln!("检索失败: {e}");
                1
            }
        }
    }
}

fn cmd_learn(args: &[String]) -> i32 {
    let Some(video) = flag(args, "--video") else {
        eprintln!("缺少 --video <mp4>");
        return 2;
    };
    let Some(srt) = flag(args, "--srt") else {
        eprintln!("缺少 --srt <srt>");
        return 2;
    };
    let Some(pack) = flag(args, "--pack") else {
        eprintln!("缺少 --pack <out_dir>");
        return 2;
    };
    let ffmpeg = flag(args, "--ffmpeg").unwrap_or_else(|| "ffmpeg".into());
    let srt_content = match std::fs::read_to_string(&srt) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("读 SRT 失败: {e}");
            return 1;
        }
    };
    let ocr = lycore::verify::Ocr::new();
    match lycore::learn::build(
        std::path::Path::new(&video),
        &srt_content,
        std::path::Path::new(&pack),
        &ffmpeg,
        &ocr,
        "eng",
    ) {
        Ok(n) => {
            println!("学习完成: {n} 个知识单元 → {pack}");
            0
        }
        Err(e) => {
            eprintln!("学习失败: {e}");
            1
        }
    }
}

fn cmd_harvest(args: &[String]) -> i32 {
    let Some(pack) = flag(args, "--pack") else {
        eprintln!("缺少 --pack <dir>");
        return 2;
    };
    let out = flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("training_tasks.jsonl"));
    match harvest(
        &std::path::Path::new(&pack).join("learning_queue.jsonl"),
        &out,
    ) {
        Ok(n) => {
            println!("已回流 {n} 个训练任务 → {}", out.display());
            0
        }
        Err(e) => {
            eprintln!("回流失败: {e}");
            1
        }
    }
}

fn cmd_doctor(args: &[String]) -> i32 {
    let Some(pack) = flag(args, "--pack") else {
        eprintln!("缺少 --pack <dir>");
        return 2;
    };
    let p = std::path::Path::new(&pack);
    let db = p.join("index/knowledge.sqlite");
    println!("[doctor] 知识包: {}", p.display());
    println!("  sqlite: {}", if db.exists() { "✓" } else { "✗" });
    let q = lycore::executor::LearningQueue::open(p);
    println!("  学习队列: {} 条待学习", q.len());
    if let Ok(url) = std::env::var("LYCORE_LLAMA") {
        println!("  llama-server: {url}");
        let backend = LlamaCppBackend::new(&url, "x");
        match backend.health() {
            Ok(()) => println!("  服务健康: ✓"),
            Err(e) => println!("  服务健康: ✗ ({e})"),
        }
    }
    0
}
