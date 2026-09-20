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
        "run" => cmd_run(&args[1..]),
        "learn" => cmd_learn(&args[1..]),
        "learn-cli" => cmd_learn_cli(&args[1..]),
        "harvest" => cmd_harvest(&args[1..]),
        "datagen" => cmd_datagen(&args[1..]),
        "project" => cmd_project(&args[1..]),
        "search" => cmd_search(&args[1..]),
        "doctor" => cmd_doctor(&args[1..]),
        "serve" => cmd_serve(&args[1..]),
        "tools" => cmd_tools(&args[1..]),
        "backend" => cmd_backend(&args[1..]),
        "help-parse" => cmd_help_parse(&args[1..]),
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
                 lycore doctor --pack <dir>\n  \
                 lycore backend [--prefer <id>] [--json]  (列可用后端 / 解析偏好)\n  \
                 lycore help-parse --cli <name> [--help-text <file>] [--top 8] [--json]\n    \
                   ↑ 把 CLI 的 --help 确定性解析成只读动作表 (v17 正解: 提炼不用模型)"
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

/// run —— 执行一条本地 CLI（意图路由器的动作出口）
///
/// 用法:
///   lyco_agent run [--trace <file>] [--cwd <dir>] -- <command...>
///   lyco_agent run echo hello
///
/// 行为: 打印 `[run <cmd0>] <整条命令>` → 执行 → 摘要 → 以命令退出码退出。
/// 配合 --trace 可把这次执行写进 trace（回放/出版用）。
fn cmd_run(args: &[String]) -> i32 {
    let mut trace: Option<String> = None;
    let mut cwd: Option<PathBuf> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--trace" => {
                trace = args.get(i + 1).cloned();
                i += 2;
            }
            "--cwd" => {
                cwd = args.get(i + 1).map(PathBuf::from);
                i += 2;
            }
            "--" => {
                i += 1;
                break;
            }
            other => {
                if other.starts_with("--") {
                    i += 1;
                    continue;
                }
                break;
            }
        }
    }
    let command = args[i.min(args.len())..].join(" ");
    if command.trim().is_empty() {
        eprintln!("用法: lyco_agent run [--trace <file>] [--cwd <dir>] -- <command...>");
        return 2;
    }
    let head = command.split_whitespace().next().unwrap_or("").to_string();
    println!("[run {head}] {command}");

    let out = lycore::tools_runtime::shell_exec(&command, cwd.as_deref());

    if let Some(tp) = trace {
        let mut tr = lycore::trace::Tracer::new(std::path::Path::new(&tp));
        tr.prompt(&format!("run: {command}"));
        tr.tool(&head, &command, out.ok);
        if !out.ok {
            tr.revert(out.summary.trim());
        }
    }

    println!("{}", out.summary.trim());
    if out.ok {
        0
    } else {
        1
    }
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
        .next_back()
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
        let mut agent = Agent::new(&executor, 4);
        // --trace <file>: 把执行过程写成 ndjson 事件流 (回放/可视化/打包)
        if let Some(tp) = flag(args, "--trace") {
            agent = agent.with_trace(std::path::Path::new(&tp));
            eprintln!("[trace] → {tp}");
        }
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
        match lycore::learn::asr_local(std::path::Path::new(&video), &ffmpeg, &lang) {
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
        rewrite_url: flag(args, "--rewrite")
            .or_else(|| flag(args, "--llama").map(|_| "http://127.0.0.1:8082".to_string())),
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
    println!(
        "  服务端 jar: {}",
        scan.server_jar.as_deref().unwrap_or("(未发现)")
    );
    println!(
        "  eula.txt: {}   plugins/: {}",
        scan.eula_present, scan.plugins_dir
    );
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

/// backend — 本机会话可用的执行后端
///
/// 普惠承诺的对外窗口: 让用户/排障一眼看到**这台机器能跑什么**,
/// 以及某个偏好是否被静默降级 (`LYCO_BACKEND` / `--prefer`)。
/// 能力层日后统一从这里取后端, 而不是各自 probe 一遍。
fn cmd_backend(args: &[String]) -> i32 {
    use lycore::backend::{self, DeviceClass};
    let prefer = flag(args, "--prefer").or_else(backend::prefer_from_env);
    let (chosen, meta) = backend::resolve(prefer.as_deref());
    let avail = backend::available();

    if args.iter().any(|a| a == "--json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "available": avail,
                "preferred": prefer,
                "chosen": chosen,
                "degraded": meta.degraded,
            }))
            .unwrap()
        );
        return 0;
    }

    let class_name = |c: DeviceClass| match c {
        DeviceClass::Cpu => "CPU",
        DeviceClass::Gpu => "GPU",
        DeviceClass::Npu => "NPU",
    };
    println!("本机可用后端 ({} 个, 优先级降序):", avail.len());
    println!(
        "  {:<14} {:<4} {:<5} ORT EP",
        "ID", "类", "优先级",
    );
    for b in &avail {
        let ep = b.ep.unwrap_or("—");
        println!(
            "  {:<14} {:<4} P{:<3} {:<38} {}",
            b.id,
            class_name(b.device),
            b.priority,
            b.display,
            ep
        );
    }
    println!("\n本次解析 → {} ({})", chosen.id, chosen.display);
    if let Some(p) = &prefer {
        println!("  偏好来源: LYCO_BACKEND/--prefer = {p}");
    }
    if meta.degraded {
        println!(
            "  ⚠ 偏好项本机不可用, 已降级到 {} — 功能不受影响, 仅快慢之别",
            chosen.id
        );
    }
    println!("\n普惠不变量: 上面必定有一行叫 cpu。加速是锦上添花, 不是能不能用的前提。");
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

/// help-parse —— 把 CLI 的 `--help` **确定性**解析成只读动作表。
///
/// 依据 docs/research-v17-help-selflearning-2026-09-21.md：
/// v17（裸灌 help）与 v17b（让模型提炼 help）双双证伪 —— 「提炼」是信息抽取，
/// 0.6B 做不可靠（docker 掉前缀、cargo 输出 `cargo:build`）。
/// 本命令把这一步变成纯字符串处理，**零 GPU、确定性、可测**。
///
/// 用法:
///   lycore help-parse --cli docker            # 实跑 `docker --help`
///   lycore help-parse --cli jq --help-text h.txt
///   lycore help-parse --cli cargo --top 8 --json
fn cmd_help_parse(args: &[String]) -> i32 {
    use lycore::help_parse::{parse_help, rank_by_frequency, readonly_only, render_schema};

    let Some(cli) = flag(args, "--cli") else {
        eprintln!("用法: lycore help-parse --cli <name> [--help-text <file>] [--top N] [--json]");
        return 2;
    };
    let top: usize = flag(args, "--top")
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let json = args.iter().any(|a| a == "--json");
    let include_writes = args.iter().any(|a| a == "--include-writes");

    // ── 取 help 文本：优先文件，否则实跑 `cli --help` ──
    let raw = match flag(args, "--help-text") {
        Some(f) => match std::fs::read_to_string(&f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("读 {f} 失败: {e}");
                return 1;
            }
        },
        None => {
            let mut got: Option<String> = None;
            // Windows 上许多 CLI 是 `.cmd`/`.bat` shim，Rust 的 Command::new
            // 不会自动解析 PATHEXT → 依次尝试 `cli` / `cli.cmd` / `cli.exe`.
            // Linux/macOS 上只有第一个候选存在，行为不变。
            let names: Vec<String> = if cfg!(windows) {
                vec![
                    cli.clone(),
                    format!("{cli}.cmd"),
                    format!("{cli}.exe"),
                    format!("{cli}.bat"),
                ]
            } else {
                vec![cli.clone()]
            };
            'outer: for name in &names {
                for a in [vec!["--help"], vec!["-h"], vec!["help"]] {
                    let mut cmd = std::process::Command::new(name);
                    cmd.args(&a);
                    // 避免 CLI 因缺 TTY 而输出分页/色码（色码另有 strip_ansi 兜底）
                    cmd.env("NO_COLOR", "1");
                    cmd.env("PAGER", "cat");
                    if let Ok(o) = cmd.output() {
                        let mut t = String::from_utf8_lossy(&o.stdout).to_string();
                        t.push_str(&String::from_utf8_lossy(&o.stderr));
                        if t.trim().len() > 60 {
                            got = Some(t);
                            break 'outer;
                        }
                    }
                }
            }
            match got {
                Some(t) => t,
                None => {
                    eprintln!("无法取得 `{cli} --help`（命令不存在或输出过短）");
                    return 1;
                }
            }
        }
    };

    let all = parse_help(&cli, &raw);
    let mut acts = if include_writes {
        rank_by_frequency(&all)
    } else {
        rank_by_frequency(&readonly_only(&all))
    };

    // ── 可选：深度探测（--deepen）拿必需位置参数 ──
    // v18 实测：`docker --help` 看不到 `logs` 需要容器名 → 模型输出 `docker logs`（漏 web）。
    // 真正语法在 `docker logs --help` 的 usage 行里。深度探测逐个补上。
    if args.iter().any(|a| a == "--deepen") {
        let probe_top = flag(args, "--deepen-top")
            .and_then(|s| s.parse().ok())
            .unwrap_or(top);
        let mut head: Vec<_> = acts.iter().take(probe_top).cloned().collect();
        let helper = |c: &str, sub: &str| -> Option<String> {
            let mut cmd = std::process::Command::new(c);
            cmd.args([sub, "--help"]);
            cmd.env("NO_COLOR", "1");
            cmd.env("PAGER", "cat");
            let o = cmd.output().ok()?;
            let mut t = String::from_utf8_lossy(&o.stdout).to_string();
            t.push_str(&String::from_utf8_lossy(&o.stderr));
            if t.trim().len() > 20 { Some(t) } else { None }
        };
        lycore::help_parse::deepen_with_subcommand_usage(&cli, &mut head, probe_top, &helper);
        // 用补好的 head 覆盖回 acts 的对应位置
        for (i, a) in head.into_iter().enumerate() {
            if i < acts.len() {
                acts[i] = a;
            }
        }
    }

    if json {
        let v = serde_json::json!({
            "cli": cli,
            "total_parsed": all.len(),
            "readonly": acts.len(),
            "actions": acts.iter().take(top).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&v).expect("序列化"));
    } else {
        println!(
            "=== {cli}: 解析出 {} 条动作（只读 {} 条），展示前 {top} ===",
            all.len(),
            acts.len()
        );
        for a in acts.iter().take(top) {
            println!("  {}", a.full_cmd);
            println!("      {}", a.desc);
        }
        if !include_writes {
            let wr = all.len() - acts.len();
            if wr > 0 {
                println!("  （另有 {wr} 条写操作被过滤；--include-writes 可查看）");
            }
        }
        println!("\n--- 注入用 schema（形态与人工 schema 一致）---");
        println!("{}", render_schema(&cli, &acts, top));
    }
    0
}
