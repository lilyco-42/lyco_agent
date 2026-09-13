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
        "learn-cli" => cmd_learn_cli(&args[1..]),
        "harvest" => cmd_harvest(&args[1..]),
        "doctor" => cmd_doctor(&args[1..]),
        "serve" => cmd_serve(&args[1..]),
        "tools" => cmd_tools(&args[1..]),
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
    let Some(pack) = flag(args, "--pack") else {
        eprintln!("缺少 --pack <out_dir>");
        return 2;
    };
    let ffmpeg = flag(args, "--ffmpeg").unwrap_or_else(|| "ffmpeg".into());
    let lang = flag(args, "--lang").unwrap_or_else(|| "zh".into());
    let ocr = lycore::verify::Ocr::new();

    // 字幕来源: --srt 指定 → 用现成字幕; 否则端侧 whisper.cpp ASR
    let cues = if let Some(srt) = flag(args, "--srt") {
        match std::fs::read_to_string(&srt) {
            Ok(s) => lycore::learn::parse_srt(&s),
            Err(e) => {
                eprintln!("读 SRT 失败: {e}");
                return 1;
            }
        }
    } else {
        match lycore::learn::asr_local(
            std::path::Path::new(&video), &ffmpeg, &lang,
        ) {
            Ok(segs) => segs
                .into_iter()
                .map(|(t0, t1, text)| lycore::learn::Cue { t0, t1, text })
                .collect(),
            Err(e) => {
                eprintln!("端侧 ASR 失败: {e}");
                return 1;
            }
        }
    };
    if cues.is_empty() {
        eprintln!("字幕/ASR 无有效内容");
        return 1;
    }

    match lycore::learn::build_cues(
        std::path::Path::new(&video),
        &cues,
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

fn cmd_serve(args: &[String]) -> i32 {
    let Some(pack) = flag(args, "--pack") else {
        eprintln!("缺少 --pack <dir>");
        return 2;
    };
    let port = flag(args, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(8666);
    let llama = flag(args, "--llama");
    let model = flag(args, "--model").unwrap_or_else(|| "qwen3-0.6b".into());
    let cfg = lycore::serve::ServeConfig {
        pack_dir: PathBuf::from(pack),
        port,
        llama_url: llama,
        llama_model: model,
        rewrite_url: flag(args, "--rewrite").or_else(|| flag(args, "--llama").map(|_| format!("http://127.0.0.1:8082"))),
        rewrite_model: flag(args, "--rewrite-model").unwrap_or_else(|| "qwen3-rewrite".into()),
    };
    if let Err(e) = lycore::serve::serve(cfg) {
        eprintln!("serve 失败: {e}");
        return 1;
    }
    0
}

fn cmd_learn_cli(args: &[String]) -> i32 {
    let Some(tool) = flag(args, "--tool") else {
        eprintln!("缺少 --tool <name>");
        return 2;
    };
    let Some(pack) = flag(args, "--pack") else {
        eprintln!("缺少 --pack <dir>");
        return 2;
    };
    let py = flag(args, "--python").unwrap_or_else(|| "python3".into());
    let indexer = flag(args, "--indexer").unwrap_or_else(|| "tools/cli_indexer.py".into());
    // indexer 路径相对于当前目录, 直接在 cwd 执行 (不做 current_dir 切换)
    let indexer_path = std::path::Path::new(&indexer);
    let script_dir = if indexer_path.is_absolute() {
        indexer_path.parent().unwrap().to_path_buf()
    } else {
        std::env::current_dir().unwrap()
    };
    let indexer_abs = if indexer_path.is_absolute() {
        indexer.clone()
    } else {
        script_dir.join(&indexer).to_string_lossy().to_string()
    };
    let r = std::process::Command::new(&py)
        .args([&indexer_abs, &tool, &pack])
        .current_dir(&script_dir)
        .status();
    match r {
        Ok(s) if s.success() => {
            println!("CLI 索引完成: {tool} → {pack}");
            0
        }
        Ok(s) => {
            eprintln!("CLI 索引失败 (exit {})", s);
            1
        }
        Err(e) => {
            eprintln!("启动失败: {e}");
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

/// 导出权威工具 schema (OpenAI tools 数组) — 单一真源, 下游 (lyco_chat
/// tools_openai.json) 从此生成, 杜绝手工维护导致的漂移。
fn cmd_tools(args: &[String]) -> i32 {
    let tools = lycore::llamacpp::chat_tools();
    let n = tools.as_array().map_or(0, |a| a.len());
    match flag(args, "--out") {
        Some(out) => {
            let pretty = serde_json::to_string_pretty(&tools).expect("序列化");
            if let Err(e) = std::fs::write(&out, pretty) {
                eprintln!("写入 {out} 失败: {e}");
                return 1;
            }
            println!("{n} 工具已导出 → {out}");
        }
        None => println!("{}", serde_json::to_string_pretty(&tools).expect("序列化")),
    }
    0
}
