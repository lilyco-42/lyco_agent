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
        "manual" => cmd_manual(&args[1..]),
        "do" => cmd_do(&args[1..]),
        "vision" => cmd_vision(&args[1..]),
        "npu" => cmd_npu(&args[1..]),
        "t1" => cmd_t1(&args[1..]),
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
                   ↑ 把 CLI 的 --help 确定性解析成只读动作表 (v17 正解: 提炼不用模型)\n  \
                 lycore vision identify <image> [--json] [--ffmpeg <path>]\n    \
                   ↑ 识图: 终端/GUI/文档/自然 (vnn 规则通道; CNN 就位后自动接管)\n  \
                 lycore npu status [--json]\n    \
                   ↑ NPU 设备探测: 报告 /dev/galcore 是否存在 (无板子时诚实返回 present=false)\n  \
                 lycore t1 check \"<cmd>\" [--json]\n    \
                   ↑ T1 执行门: 风险分级(read/write/danger) + 必需参数校验(缺参=静默失效)"
            );
            2
        }
    };
    std::process::exit(exit);
}

/// `lycore t1 check "<cmd>"` —— T1 执行门的用户可触达入口（2026-09-21 P1.2）。
///
/// 一次输出**两条正交结论**：
///   1. 风险分级（read/write/danger）—— 来自 `t1gate::classify`
///   2. 必需参数校验（ok/needs_param/unchecked）—— 来自 `paramcheck::check`
///
/// 为什么合并成一个子命令而不是两个：调用方（含 agent loop）拿到一条命令时，
/// 「危不危险」和「能不能干活」必须**一起**决定放不放行，分两次调用会漏判其中一个。
///
/// 退出码：0=可执行(read) / 1=需确认或阻断(write/danger/needs_param) / 2=用法错误。
fn cmd_t1(args: &[String]) -> i32 {
    let Some(sub) = args.first().map(|s| s.as_str()) else {
        eprintln!("用法: lycore t1 check \"<cmd>\" [--json]");
        return 2;
    };
    if sub != "check" {
        eprintln!("未知子命令 t1 {sub}；目前只支持 check");
        return 2;
    }
    let rest = &args[1..];
    let json = rest.iter().any(|a| a == "--json");
    let Some(cmd) = rest.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("缺少命令: lycore t1 check \"npm install\"");
        return 2;
    };

    let v = lycore::t1gate::classify(cmd);
    let p = lycore::paramcheck::check(cmd);
    // 组合命令逐段查缺参（与 router::plan_from_raw 口径一致）
    let parts: Vec<&str> = v
        .command
        .split(|c| c == ';' || c == '|')
        .flat_map(|x| x.split("&&"))
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .collect();
    let all_ok = parts
        .iter()
        .all(|x| !lycore::paramcheck::check(x).status.is_needs_param());

    // 决策：与 router 的分诊链同序（危险优先 → 缺参 → 写确认 → 只读）
    let decision = match v.risk {
        lycore::t1gate::Risk::Danger => "block",
        _ if !all_ok => "needs_param",
        lycore::t1gate::Risk::Write => "confirm",
        lycore::t1gate::Risk::Read => "run",
    };

    if json {
        let out = serde_json::json!({
            "command": v.command,
            "verdict": decision,
            "risk": v.risk.as_str(),
            "matched": v.matched,
            "reason": v.reason,
            "param": {
                "status": p.status.as_str(),
                "missing": p.missing,
                "reason": p.reason,
            },
            "may_execute_now": decision == "run",
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    } else {
        println!("命令   : {}", v.command);
        println!(
            "风险   : {} ({})",
            v.risk.as_str(),
            v.matched.as_deref().unwrap_or("-")
        );
        println!("分诊   : {decision}");
        println!(
            "参数   : {}{}",
            p.status.as_str(),
            if p.missing.is_empty() {
                String::new()
            } else {
                format!("  缺: {}", p.missing.join(" "))
            }
        );
        if !p.reason.is_empty() {
            println!("提示   : {}", p.reason);
        }
        if !v.reason.is_empty() && decision != "needs_param" {
            println!("说明   : {}", v.reason);
        }
        if decision == "needs_param" {
            println!("\n→ 空参运行会静默失效（退出码 0 却干不成活），请补全参数后重试。");
        }
    }

    match decision {
        "run" => 0,
        _ => 1,
    }
}

/// `lycore npu status` —— 把 409 行、零调用的 `npu_runtime` 接出到 CLI（2026-09-21 P0.2）。
///
/// **诚实探测**：板子已 halt、6.6 galcore hard hang，所以本命令在无板子时**正常返回
/// `present:false` 且 exit 0**，而不是报错。理由：调用方（含 agent loop）需要
/// 「知道没有 NPU 然后走 CPU 兜底」，而不是收到一个失败——这与 `backend::available()`
/// 恒非空、CPU 永远兜底的普惠不变量一致。
fn cmd_npu(args: &[String]) -> i32 {
    let Some(sub) = args.first().map(|s| s.as_str()) else {
        eprintln!("用法: lycore npu status [--json]");
        return 2;
    };
    if sub != "status" {
        eprintln!("未知子命令 npu {sub}；目前只支持 status");
        return 2;
    }
    let json = args.iter().any(|a| a == "--json");

    // 设备节点探测：A733/VIP9000 的 galcore 驱动暴露 /dev/galcore。
    // 其他厂商（RKNN/QNN/CANN）节点名不同，此处只报我们实际支持的那一个，
    // 不做「假装支持」。
    let present = std::path::Path::new("/dev/galcore").exists();
    let model = std::fs::read_to_string("/proc/device-tree/model")
        .map(|s| s.trim_end_matches('\0').trim().to_string())
        .ok()
        .filter(|s| !s.is_empty());

    if json {
        let v = serde_json::json!({
            "present": present,
            "device": "/dev/galcore",
            "board_model": model,
            "serial_constraint": "VIP9000 同一时刻只能跑一个网络；多消费者须自行排队",
            "lease_api": "lycore::npu_runtime::{acquire,enqueue,release,sweep}",
            "fallback": if present { "NPU 可用: 走 NpuBackend 真实实现 (TIM-VX 用户态)" }
                        else { "CPU 兜底: 视觉走 vnn 规则通道 / yolo 走 tract" },
        });
        println!("{}", serde_json::to_string_pretty(&v).expect("序列化"));
    } else if present {
        println!("NPU: 存在  (/dev/galcore)");
        if let Some(m) = &model {
            println!("  板型: {m}");
        }
        println!("  硬约束: VIP9000 同一时刻只能跑一个网络 → 必须串行排队");
        println!("  租约 API: lycore::npu_runtime::{{acquire,enqueue,release,sweep}}");
    } else {
        println!("NPU: 不存在  (/dev/galcore 未找到)");
        if let Some(m) = &model {
            println!("  板型: {m}");
        }
        println!("  这是正常状态（板子 halt / 非 A733 设备）→ 视觉能力走 CPU 兜底");
        println!("  串行调度器设计已就绪（409 行 + 单测），待 TIM-VX 用户态联调");
    }
    0
}

/// `lycore vision identify <image>` —— 把已在 executor 中接线、但用户无法直接触达的
/// `vnn::identify` 暴露成子命令。逻辑零新增，只补 CLI 入口（2026-09-21 P0.1）。
///
/// 退出码：0 = 出结果；1 = 识别失败（图不存在 / ffmpeg 缺失）；
/// 2 = 用法错误。**未训练专家会诚实带上 `needs_training: true`，不伪装。**
fn cmd_vision(args: &[String]) -> i32 {
    let Some(sub) = args.first().map(|s| s.as_str()) else {
        eprintln!("用法: lycore vision identify <image> [--json] [--ffmpeg <path>]");
        return 2;
    };
    if sub != "identify" {
        eprintln!("未知子命令 vision {sub}；目前只支持 identify");
        return 2;
    }

    let rest = &args[1..];
    let image = rest
        .iter()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .next_back();
    let Some(image) = image else {
        eprintln!("缺少图片路径: lycore vision identify <image>");
        return 2;
    };
    let json = rest.iter().any(|a| a == "--json");
    let ffmpeg = flag(rest, "--ffmpeg")
        .or_else(|| std::env::var("LYV_FFMPEG").ok())
        .unwrap_or_else(|| "ffmpeg".into());

    match lycore::vnn::identify(&ffmpeg, std::path::Path::new(&image)) {
        Ok((verdict, conf, experts, queue)) => {
            if json {
                let v = serde_json::json!({
                    "image": image,
                    "kind": verdict,
                    "conf": conf,
                    "needs_training": experts.iter().any(|e| e.needs_training),
                    "experts": experts,
                    "learning_queue": queue,
                });
                println!("{}", serde_json::to_string_pretty(&v).expect("序列化"));
            } else {
                println!("{image} → {verdict}  (conf {conf:.2})");
                for e in &experts {
                    let tag = if e.needs_training { " [未训练]" } else { "" };
                    println!(
                        "   {:<12} act={:.3} {} (conf {:.2}){tag}",
                        e.neuron, e.activation, e.verdict, e.conf
                    );
                }
                if !queue.is_empty() {
                    println!("   → 已加入学习队列 {} 条", queue.len());
                }
            }
            0
        }
        Err(e) => {
            eprintln!("识别失败: {e}");
            1
        }
    }
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
    println!("  {:<14} {:<4} {:<5} ORT EP", "ID", "类", "优先级",);
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

// ══════════════ manual —— CLI 中文说明书 + 冷启动训练数据 ══════════════
//
// 需求（用户 2026-09-23）：「写一个 CLI 说明书 和本地翻译（为了以后发展），
// 提供 cli --help 解析为中文，帮助我做初步训练数据。」
//
// 链路：`cli --help` → `parse_help`（确定性）→ `cli_zh` 本地词典翻译（零模型）
//       → ① 中文说明书（给人看）② 训练样本（nl 中文 → cmd）
//
// 为什么翻译必须本地：端侧要离线；且译文**会成为训练数据** ——
// 掺进模型的幻觉等于把噪声训进权重（v13 失败的一个来源）。

/// 取 help 文本：优先 `--help-text <file>`，否则实跑 `cli --help`。
fn grab_help_text(cli: &str, args: &[String]) -> Result<String, String> {
    if let Some(f) = flag(args, "--help-text") {
        return std::fs::read_to_string(&f).map_err(|e| format!("读 {f} 失败: {e}"));
    }
    lycore::router::grab_help(cli).ok_or_else(|| {
        format!(
            "无法取得 `{cli} --help`（命令不存在或输出过短）。可用 --help-text <file> 直接喂文本"
        )
    })
}

/// manual —— 把 `cli --help` 变成中文说明书，并可选产出训练数据。
///
/// 用法:
///   lycore manual --cli docker                          # 打印中文说明书
///   lycore manual --cli docker --out docker.md          # 落成 markdown
///   lycore manual --cli docker --data docker.jsonl      # 出冷启动训练数据
///   lycore manual --cli docker --help-text h.txt --json
fn cmd_manual(args: &[String]) -> i32 {
    use lycore::cli_zh;
    use lycore::help_parse::{parse_help, rank_by_frequency};

    let Some(cli) = flag(args, "--cli") else {
        eprintln!(
            "用法: lycore manual --cli <name> [--help-text <file>] [--out <md>] [--data <jsonl>] [--json]"
        );
        return 2;
    };
    let raw = match grab_help_text(&cli, args) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };

    // 说明书要**全覆盖**（不只只读）—— 用户要知道这个 CLI 到底能干什么，
    // 安全性交给 T1 门，不靠"藏起来"。
    let acts = rank_by_frequency(&parse_help(&cli, &raw));
    if acts.is_empty() {
        eprintln!("`{cli} --help` 解析出 0 条动作（help 版式未覆盖）。不产出垃圾说明书。");
        return 1;
    }
    let zh: Vec<cli_zh::ZhAction> = acts.iter().map(cli_zh::translate_action).collect();
    let st = cli_zh::stats(&zh);

    // ── 训练数据（可选）──
    // 🔴 质量门禁：翻译覆盖率低于阈值的动作**不进训练数据**。
    //    半吊子译文与幻觉只有一线之隔，宁可少几千条也不要几百条错的
    //    （这是 v13 失败的直接教训：合成语料装的是"我们的想象"）。
    let min_cov: f64 = flag(args, "--min-coverage")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.8);
    let mut n_data = 0usize;
    if let Some(dp) = flag(args, "--data") {
        let mut samples = cli_zh::to_samples(&cli, &zh, min_cov);
        let n_actions = samples.len();
        samples.extend(cli_zh::reject_samples(&cli));
        n_data = samples.len();
        let body: String = samples
            .iter()
            .filter_map(|s| serde_json::to_string(s).ok())
            .collect::<Vec<_>>()
            .join("\n");
        if let Err(e) = std::fs::write(&dp, format!("{body}\n")) {
            eprintln!("写 {dp} 失败: {e}");
            return 1;
        }
    }

    let md = cli_zh::render_manual(&cli, &zh, raw.len());

    if args.iter().any(|a| a == "--json") {
        let out = serde_json::json!({
            "cli": cli,
            "help_len": raw.len(),
            "stats": st,
            "actions": zh,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    } else if let Some(outp) = flag(args, "--out") {
        if let Err(e) = std::fs::write(&outp, &md) {
            eprintln!("写 {outp} 失败: {e}");
            return 1;
        }
        println!(
            "{} 条动作 → {outp}（只读 {} / 写 {}，平均覆盖率 {:.0}%，完全未译 {}）",
            st.total,
            st.readonly,
            st.write,
            st.avg_coverage * 100.0,
            st.untranslated
        );
    } else {
        println!("{md}");
    }

    if n_data > 0 {
        let kept = cli_zh::eligible(&zh, min_cov);
        println!(
            "→ 训练数据 {} 条（{} 条动作样本 + {} 条出域拒绝样本）\n\
             \x20  质量门禁：覆盖率 ≥{:.0}% 的 {} 条动作入选，{} 条因翻译不可靠被挡在数据之外",
            n_data,
            n_data - cli_zh::REJECT_NLS.len(),
            cli_zh::REJECT_NLS.len(),
            min_cov * 100.0,
            kept,
            st.total - kept
        );
    }
    0
}

// ══════════════ do —— 人机闭环（人先当模型）══════════════
//
// 这是判定式脊的**第一个真实调用者**。2026-09-23 的接线审计发现：
// `plan_choice` / `router::plan` / `t1gate` 全都没有产品调用者 —— 所有
// 41.7% / 85.0% 的分数都来自评测脚本，产品里从没有一条路径走过
// 「新 CLI → help → 判定 → 执行」。
//
// 这里把「模型」那一环换成**人**，其余全部复用已有确定性代码。
// 副产品：每次执行都落一条 `(nl, cmd, exit_code)` —— 我们全部训练数据里
// 唯一从来没有过的东西。

/// 读一行（提示写 stderr，结果留 stdout 给 `--json`）
fn prompt_line(msg: &str) -> String {
    use std::io::Write;
    eprint!("{msg}");
    let _ = std::io::stderr().flush();
    let mut s = String::new();
    if std::io::stdin().read_line(&mut s).is_err() {
        return String::new();
    }
    s.trim().to_string()
}

/// do —— 一句话 → 候选 → 人选 → 填槽 → T1 门 → 执行 → 落盘。
///
/// 用法:
///   lycore do "看看有哪些容器在跑"                 # 交互
///   lycore do "看看有哪些容器在跑" --dry-run        # 只出计划不执行
///   lycore do "..." --cli docker --pick 1          # 非交互（脚本/评测）
fn cmd_do(args: &[String]) -> i32 {
    use lycore::choices::{
        build_choices, find_placeholders, parse_picked, render_choices, shell_quote_if_needed,
        Picked,
    };
    use lycore::help_parse::{parse_help, rank_by_frequency};
    use lycore::lbrush::{self, GateDecision, LbrushRecord};

    let dry_run = args.iter().any(|a| a == "--dry-run");
    let yes = args.iter().any(|a| a == "--yes") || args.iter().any(|a| a == "-y");
    let json = args.iter().any(|a| a == "--json");
    let top: usize = flag(args, "--top")
        .and_then(|s| s.parse().ok())
        .unwrap_or(40);
    let preset_pick: Option<usize> = flag(args, "--pick").and_then(|s| s.parse().ok());
    let record_path = flag(args, "--record")
        .map(PathBuf::from)
        .unwrap_or_else(lbrush::default_record_path);

    // ── 自然语言 = 所有非 flag 参数（跳过 flag 的值）──
    let val_flags = [
        "--cli",
        "--top",
        "--pick",
        "--record",
        "--help-text",
        "--cwd",
        "--slot",
    ];
    let mut nl_parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if val_flags.contains(&a.as_str()) {
            i += 2;
            continue;
        }
        if a.starts_with("--") {
            i += 1;
            continue;
        }
        nl_parts.push(a.clone());
        i += 1;
    }
    let nl = nl_parts.join(" ");
    if nl.trim().is_empty() {
        eprintln!(
            "用法: lycore do \"<一句话>\" [--cli <name>] [--dry-run] [--pick N] [--record <file>]"
        );
        return 2;
    }

    // ── 1. CLI 选择：显式 --cli 优先，否则从人话里猜 ──
    let Some(cli) = flag(args, "--cli").or_else(|| lycore::router::guess_cli(&nl)) else {
        eprintln!("猜不出要用哪个 CLI（可用 --cli <name> 指定）");
        return 1;
    };

    // ── 2. 取 help → 确定性动作表 ──
    let raw = match grab_help_text(&cli, args) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let acts = rank_by_frequency(&parse_help(&cli, &raw));
    if acts.is_empty() {
        eprintln!("`{cli} --help` 解析出 0 条动作。");
        return 1;
    }
    let choices = build_choices(&acts, top);

    if !json {
        println!(
            "[CLI] {cli}（help {} 字符 → {} 条动作，展示 {} 条）",
            raw.len(),
            acts.len(),
            choices.len()
        );
        println!("[人话] {nl}");
        println!();
        print!("{}", render_choices(&cli, &choices));
        println!();
    }

    // ── 3. 判定：人（或 --pick）──
    let picked = match preset_pick {
        Some(k) if k == 0 => Picked::None,
        Some(k) if k <= choices.len() => Picked::Idx(k),
        Some(k) => {
            eprintln!("--pick {k} 超出范围（1..={}）", choices.len());
            return 2;
        }
        None => {
            let line = prompt_line("选哪个? （编号，0 = 都不合适，直接回车取消）");
            if line.is_empty() {
                println!("已取消。");
                return 0;
            }
            parse_picked(&line, choices.len())
        }
    };

    // 拒绝态：一个候选都不合适 —— 这是我们 reject 0% 的**正样本**来源，必须落盘
    let idx = match picked {
        Picked::None => 0,
        Picked::Idx(k) => k,
        Picked::Unparsable => {
            eprintln!("没看懂编号，请输 1..={} 或 0", choices.len());
            return 2;
        }
    };

    let Some(choice) = choices.iter().find(|c| c.idx == idx).cloned() else {
        // 人明确说"都不合适" → 记录一条拒绝样本（plan 用一个占位动作）
        let placeholder = lycore::help_parse::HelpAction {
            full_cmd: String::new(),
            desc: "（人判定：候选里没有合适的动作）".into(),
            readonly: true,
            example: None,
        };
        let plan = lbrush::plan_human(&cli, &placeholder, &[]);
        let mut rec = LbrushRecord::new(&nl, &cli, 0, &plan, &[]);
        // 空命令会被 classify 判成 Danger("空命令, 不执行")，但语义是**拒绝**不是危险。
        // 不覆盖的话，拒绝样本在下游会被误读成"一条被拦下的危险命令"，污染归因。
        rec.risk = "read".to_string();
        rec.decision = "reject".to_string();
        if let Err(e) = lbrush::append(&rec, &record_path) {
            eprintln!("[警告] 记录失败: {e}");
        }
        if json {
            println!(
                "{}",
                serde_json::json!({"decision": "reject", "idx": 0, "cli": cli})
            );
        } else {
            println!("已记录：候选里没有合适动作（不执行）。这条就是「拒绝」的正样本。");
        }
        return 0;
    };

    // ── 4. 填槽位（不猜：缺就回问）──
    let tpl = choice
        .example
        .clone()
        .unwrap_or_else(|| choice.full_cmd.clone());
    let phs = find_placeholders(&tpl);
    let mut slots: Vec<String> = Vec::new();
    for p in &phs {
        let v = if let Some(v) = flag(args, "--slot") {
            v
        } else {
            prompt_line(&format!("  填 {p}: "))
        };
        let v = v.trim();
        if v.is_empty() {
            eprintln!("[{p}] 没填 → 不执行（空跑会静默失效，绝不猜值）");
            return 1;
        }
        slots.push(shell_quote_if_needed(v));
    }

    // ── 5. 组装 + T1 门 + 缺参检查（全部确定性）──
    let action = lycore::help_parse::HelpAction {
        full_cmd: choice.full_cmd.clone(),
        desc: String::new(),
        readonly: choice.readonly,
        example: choice.example.clone(),
    };
    let plan = lbrush::plan_human(&cli, &action, &slots);
    let decision = match plan.decision.as_str() {
        "run" => GateDecision::Run,
        "confirm" => GateDecision::Confirm,
        "block" => GateDecision::Block,
        _ => GateDecision::NeedsParam,
    };

    if !json {
        println!("[命令] {}", plan.cmd);
        println!("[风险] {}（{}）", plan.risk, plan.reason);
        println!("[决策] {}", plan.decision);
        if !plan.missing.is_empty() {
            println!("[缺槽] {}", plan.missing.join(" "));
        }
        if !plan.param_missing.is_empty() {
            println!("[缺参] {}", plan.param_missing.join(" "));
        }
    }

    if decision == GateDecision::NeedsParam {
        if json {
            println!(
                "{}",
                serde_json::json!({"decision": "needs_param", "plan": plan})
            );
        } else {
            println!("\n→ 不执行：参数不全，空跑等于静默失效。请补全后重试。");
        }
        let rec = LbrushRecord::new(&nl, &cli, idx, &plan, &slots);
        let _ = lbrush::append(&rec, &record_path);
        return 0;
    }

    // ── 6. 放行判定（人确认 / 危险 override）──
    let mut confirmed = yes;
    let mut override_danger = args.iter().any(|a| a == "--i-know");
    if !dry_run && !lbrush::may_execute(decision, confirmed, override_danger) {
        if decision == GateDecision::Confirm {
            confirmed = prompt_line("这是写操作，确认执行? [y/N] ").eq_ignore_ascii_case("y");
        } else if decision == GateDecision::Block {
            let l = prompt_line("⚠️ 这是破坏性/不可逆操作。要执行请输入 I-KNOW: ");
            override_danger = l == "I-KNOW";
            confirmed = override_danger;
        }
    }

    let allowed = dry_run || lbrush::may_execute(decision, confirmed, override_danger);
    if !allowed {
        if !json {
            println!("\n→ 已拦截，未执行。");
        }
        let mut rec = LbrushRecord::new(&nl, &cli, idx, &plan, &slots);
        rec.executed = false;
        let _ = lbrush::append(&rec, &record_path);
        if json {
            println!(
                "{}",
                serde_json::json!({"decision": plan.decision, "executed": false, "plan": plan})
            );
        }
        return 0;
    }

    if dry_run {
        if !json {
            println!("\n[dry-run] 通过全部门禁，未执行。");
        }
        let mut rec = LbrushRecord::new(&nl, &cli, idx, &plan, &slots);
        rec.executed = false;
        let _ = lbrush::append(&rec, &record_path);
        if json {
            println!(
                "{}",
                serde_json::json!({"decision": plan.decision, "executed": false, "dry_run": true, "plan": plan})
            );
        }
        return 0;
    }

    // ── 7. 执行 + **真实退出码**（我们所有旧数据都没有这个信号）──
    let cwd = flag(args, "--cwd").map(PathBuf::from);
    let (code, so, se) = match lycore::tools_runtime::shell_exec_capture(&plan.cmd, cwd.as_deref())
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("执行失败: {e}");
            return 1;
        }
    };

    let mut rec = LbrushRecord::new(&nl, &cli, idx, &plan, &slots);
    rec.executed = true;
    rec.exit_code = code;
    if let Err(e) = lbrush::append(&rec, &record_path) {
        eprintln!("[警告] 记录失败: {e}");
    }

    if json {
        println!(
            "{}",
            serde_json::json!({
                "decision": plan.decision, "executed": true,
                "exit_code": code, "stdout": so, "stderr": se, "plan": plan,
                "record": record_path.to_string_lossy(),
            })
        );
    } else {
        let body = if so.trim().is_empty() { &se } else { &so };
        println!(
            "[退出码] {}",
            code.map(|c| c.to_string())
                .unwrap_or_else(|| "无（超时/信号终止）".into())
        );
        if !body.trim().is_empty() {
            println!("{}", body.trim());
        }
        println!("[已记录] {}", record_path.display());
    }
    // T1 门的 exit_code 语义：命令成功 = 0；拦截仍然算"平台成功"
    if code == Some(0) {
        0
    } else {
        1
    }
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
    use lycore::help_parse::{parse_help, rank_by_frequency, readonly_only};

    let Some(cli) = flag(args, "--cli") else {
        eprintln!("用法: lycore help-parse --cli <name> [--help-text <file>] [--top N] [--json]");
        return 2;
    };
    // **v18c 实测：截断注入是净损失。** top-12 → top-5（对齐人工 schema 条数）
    // 后保真度反而 62.7% → 54.7%，因为排在前 5 之外的命令（docker images、
    // cargo tree、npm outdated）根本没被注入，模型只能在已注入项里做最近邻替代。
    // 6 个 CLI 的只读动作最坏不过 49 条，对 4k+ 上下文无压力 → **默认全量**，
    // 只有显式传 --top 才截断。
    let top: usize = flag(args, "--top")
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);
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
            if t.trim().len() > 20 {
                Some(t)
            } else {
                None
            }
        };
        lycore::help_parse::deepen_with_subcommand_usage(&cli, &mut head, probe_top, &helper);
        // 用补好的 head 覆盖回 acts 的对应位置
        for (i, a) in head.into_iter().enumerate() {
            if i < acts.len() {
                acts[i] = a;
            }
        }
    }

    // 「全量」在打印时不该显示成 usize::MAX（18446744073709551615）。
    let shown = if top == usize::MAX { acts.len() } else { top };

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
            "=== {cli}: 解析出 {} 条动作（只读 {} 条），展示前 {shown} ===",
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
        // ⚠️ v19b：与 router::help_aware_schema 用同一个渲染入口。
        // 薄 help（动作 <15 且原文 ≤4KB）会自动追加原文兜底 —— 若这里用裸
        // render_schema，本命令就**看不到生产实际注入的内容**，调试时会被误导。
        println!(
            "{}",
            lycore::help_parse::render_schema_with_raw_fallback(&cli, &acts, &raw, top)
        );
    }
    0
}
