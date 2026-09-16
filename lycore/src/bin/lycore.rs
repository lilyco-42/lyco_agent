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
use lycore::search::SearchBackend;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let exit = match cmd {
        "ask" => cmd_ask(&args[1..]),
        "learn" => cmd_learn(&args[1..]),
        "learn-cli" => cmd_learn_cli(&args[1..]),
        "harvest" => cmd_harvest(&args[1..]),
        "datagen" => cmd_datagen(&args[1..]),
        "project" => cmd_project(&args[1..]),
        "search" => cmd_search(&args[1..]),
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
                 lycore datagen --pack <dir> [--out <file>]  (学习队列→answer-first SFT 语料)\n  \
                 lycore project --dir <path> [--pack <dir>] [--out <script>] [--start 08:00] [--end 22:00]\n  \
                 lycore search --query <...> [--pack <dir>] [--url <searxng>] [--k 5]\n  \
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
    // Rust 原生索引 (零 Python): tool --help → parse Commands → 逐子命令 help
    // → tool-scoped 幂等写包。旧 --python/--indexer 参数忽略 (兼容不报错)。
    match lycore::learn_cli::index_and_build(&tool, std::path::Path::new(&pack)) {
        Ok(n) => {
            println!("CLI 索引完成: {tool} → {pack} ({n} 条目)");
            0
        }
        Err(e) => {
            eprintln!("CLI 索引失败: {e}");
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

/// 学习队列 → answer-first SFT 语料 (P1 数据闭环; ToolRAG 语义复核 + ToolGrad 种子)
fn cmd_datagen(args: &[String]) -> i32 {
    let Some(pack) = flag(args, "--pack") else {
        eprintln!("缺少 --pack <dir>");
        return 2;
    };
    let out = flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("sft_corpus.jsonl"));
    let rag = lycore::toolrag::ToolRag::build(lycore::toolrag::TokenEmbedder::new());
    let queue = std::path::Path::new(&pack).join("learning_queue.jsonl");
    match lycore::datagen::datagen(&queue, &out, &rag) {
        Ok(n) => {
            println!("answer-first SFT 语料: {n} 条 → {}", out.display());
            0
        }
        Err(e) => {
            eprintln!("datagen 失败: {e}");
            1
        }
    }
}

/// 项目目录 → 服务端识别 + 关联知识入库 + 时间窗启动脚本
/// (支撑指令: "写一个 8:00-22:00 自动启动 java 起 Minecraft 的程序")
fn cmd_project(args: &[String]) -> i32 {
    let Some(dir) = flag(args, "--dir") else {
        eprintln!("缺少 --dir <path>");
        return 2;
    };
    let start = flag(args, "--start").unwrap_or_else(|| "08:00".into());
    let end = flag(args, "--end").unwrap_or_else(|| "22:00".into());
    let scan = match lycore::project::scan(std::path::Path::new(&dir)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("扫描失败: {e}");
            return 1;
        }
    };
    println!("[project] {}", scan.dir);
    println!("  服务端类型: {}", scan.kind.as_str());
    println!("  服务端 jar: {}", scan.server_jar.as_deref().unwrap_or("(未发现)"));
    println!("  eula.txt: {}   plugins/: {}", scan.eula_present, scan.plugins_dir);
    for a in &scan.artifacts {
        println!("   - {} [{}]", a.name, a.kind);
    }

    // 关联知识入库 (Minecraft/paper 领域知识 + 本项目事实)
    if let Some(pack) = flag(args, "--pack") {
        let cues = lycore::project::knowledge_cues(&scan);
        match lycore::learn_cli::append_cues(std::path::Path::new(&pack), "project", &cues) {
            Ok(n) => println!("  关联知识入库: {n} 条 → {pack}"),
            Err(e) => {
                eprintln!("入库失败: {e}");
                return 1;
            }
        }
    }

    // 生成启动脚本
    let script = lycore::project::launch_script(&scan, &start, &end);
    match flag(args, "--out") {
        Some(out) => {
            if let Err(e) = std::fs::write(&out, &script) {
                eprintln!("写脚本失败: {e}");
                return 1;
            }
            println!("  启动脚本 → {out} (时间窗 {start}-{end})");
        }
        None => println!("\n{script}"),
    }
    0
}

/// 自主检索 + 学习进知识包 (补"懂得自己去搜"这一环)
/// 未给 --url 时用离线占位后端 (不触网); 给了则走自建 SearXNG。
fn cmd_search(args: &[String]) -> i32 {
    let Some(query) = flag(args, "--query") else {
        eprintln!("缺少 --query <...>");
        return 2;
    };
    let k = flag(args, "--k").and_then(|x| x.parse().ok()).unwrap_or(5);
    let results = match flag(args, "--url") {
        Some(base) => {
            let b = lycore::search::SearxngBackend::new(&base);
            println!("[search] SearXNG: {}", b.base());
            match b.search(&query, k) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("搜索失败: {e}");
                    return 1;
                }
            }
        }
        None => {
            eprintln!("[search] 未给 --url, 使用离线占位结果 (StubSearch, 不触网)");
            lycore::search::StubSearch::demo(&query)
                .search(&query, k)
                .unwrap_or_default()
        }
    };
    for (i, r) in results.iter().enumerate() {
        println!("{}. {} — {}\n   {}", i + 1, r.title, r.url, r.snippet);
    }
    if let Some(pack) = flag(args, "--pack") {
        let cues = lycore::search::to_cues(&query, &results);
        match lycore::learn_cli::append_cues(std::path::Path::new(&pack), "search", &cues) {
            Ok(n) => println!("  已学习 {n} 条 → {pack}"),
            Err(e) => {
                eprintln!("入库失败: {e}");
                return 1;
            }
        }
    }
    0
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
