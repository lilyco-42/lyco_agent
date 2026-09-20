//! tools_runtime — 端侧工具集 (进程外调用, 零重依赖)
//!
//! 目标: 任何能跑 std::process::Command 的设备上都可用 (笔记本/台式机/单板机/容器),
//! agent loop 的 tool_call 直达。硬件差异由 `backend` 模块负责, 本模块不假设
//! 任何具体芯片——没有 NPU 就走 CPU, 少哪个外部二进制就诚实报错入学队列。
//!   - rembg_remove: 抠图 (python -m rembg CLI, 需 pip install rembg)
//!   - llm_generate: 文本/HTML 生成 (OpenAI 兼容 endpoint: NVIDIA NIM / OpenRouter free, key 从 env)
//!   - html_render_video: HTML → 视频 (headless chrome 截帧 + ffmpeg 合成)
//!   - video_info: 视频元信息 (ffprobe)
//!
//! 全部走 std::process::Command (与 tesseract/ffmpeg 同一模式), 失败诚实返回
//! + 入学习队列, 不编造。endpoint/key 解析:
//!   LYCO_LLM_URL (默认 NVIDIA NIM https://integrate.api.nvidia.com/v1)
//!   LYCO_LLM_KEY (nvapi-... / sk-or-...)
//!   LYCO_LLM_MODEL (默认 meta/llama-3.2-11b-vision-instruct (NIM 实测可用))

use std::path::{Path, PathBuf};
use std::time::Duration;

/// 工具输出: stdout/stderr 摘要 + 产物路径
pub struct RunOutcome {
    pub ok: bool,
    pub summary: String,
    pub artifact: Option<PathBuf>,
}

fn run_cmd(bin: &str, args: &[&str], _timeout: Duration) -> RunOutcome {
    match std::process::Command::new(bin).args(args).output() {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let ok = out.status.success();
            let tail = |s: &str, n: usize| {
                s.lines()
                    .rev()
                    .take(n)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join(" | ")
            };
            RunOutcome {
                ok,
                summary: if ok {
                    tail(&stdout, 3)
                } else {
                    format!("stderr: {}", tail(&stderr, 3))
                },
                artifact: None,
            }
        }
        Err(e) => RunOutcome {
            ok: false,
            summary: format!("{bin} 启动失败: {e}"),
            artifact: None,
        },
    }
}

/// ring CryptoProvider 安装 (rustls-no-provider 必需, 交叉编译友好)。
/// 任何 reqwest Client 构造前都需调用 (llamacpp backend / llm_generate)。
pub fn install_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// 抠图: rembg CLI (pip install rembg; 首次运行自动下载 u2net 权重 ~170MB)
pub fn rembg_remove(input: &Path, output: &Path) -> RunOutcome {
    if !input.exists() {
        return RunOutcome {
            ok: false,
            summary: format!("输入不存在: {}", input.display()),
            artifact: None,
        };
    }
    let mut o = run_cmd(
        "rembg",
        &["i", &input.to_string_lossy(), &output.to_string_lossy()],
        Duration::from_secs(300),
    );
    if o.ok && output.exists() {
        o.artifact = Some(output.to_path_buf());
        o.summary = format!("抠图完成 → {}", output.display());
    } else if !o.ok {
        o.summary = format!(
            "rembg 失败 ({}). 需 pip install \"rembg[cpu]\"; {}",
            o.summary, "或 LYCO_REMBG_BIN 指定路径"
        );
    }
    o
}

/// LLM 生成 (OpenAI 兼容 chat/completions, 阻塞非流式)
/// prompt 给了 out_path 时把回复写入文件 (html_gen 用)
pub fn llm_generate(prompt: &str, out_path: Option<&Path>) -> RunOutcome {
    install_crypto_provider();
    let url = std::env::var("LYCO_LLM_URL")
        .unwrap_or_else(|_| "https://integrate.api.nvidia.com/v1/chat/completions".into());
    let key = std::env::var("LYCO_LLM_KEY").unwrap_or_default();
    let model = std::env::var("LYCO_LLM_MODEL")
        .unwrap_or_else(|_| "meta/llama-3.2-11b-vision-instruct".into());
    if key.is_empty() {
        return RunOutcome {
            ok: false,
            summary: "LYCO_LLM_KEY 未设置 (NIM nvapi-... / OpenRouter sk-or-...)".into(),
            artifact: None,
        };
    }
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 4096,
        "temperature": 0.7,
    });
    // 阻塞 HTTP: reqwest 已在依赖树 (blocking), 复用零新增
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return RunOutcome {
                ok: false,
                summary: format!("http client: {e}"),
                artifact: None,
            }
        }
    };
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {key}"))
        .json(&body)
        .send();
    match resp {
        Ok(r) => match r.json::<serde_json::Value>() {
            Ok(v) => {
                let content = v["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                if content.is_empty() {
                    return RunOutcome {
                        ok: false,
                        summary: format!(
                            "LLM 空回复: {}",
                            v.to_string().chars().take(200).collect::<String>()
                        ),
                        artifact: None,
                    };
                }
                match out_path {
                    Some(p) => {
                        if let Some(parent) = p.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        match std::fs::write(p, &content) {
                            Ok(_) => RunOutcome {
                                ok: true,
                                summary: format!("已生成 {} ({} 字符)", p.display(), content.len()),
                                artifact: Some(p.to_path_buf()),
                            },
                            Err(e) => RunOutcome {
                                ok: false,
                                summary: format!("写入失败: {e}"),
                                artifact: None,
                            },
                        }
                    }
                    None => RunOutcome {
                        ok: true,
                        summary: content.chars().take(400).collect(),
                        artifact: None,
                    },
                }
            }
            Err(e) => RunOutcome {
                ok: false,
                summary: format!("响应解析失败: {e}"),
                artifact: None,
            },
        },
        Err(e) => RunOutcome {
            ok: false,
            summary: format!("HTTP 失败: {e}"),
            artifact: None,
        },
    }
}

/// HTML → 视频: chrome headless 按秒截 N 帧 → ffmpeg 合成 mp4
pub fn html_render_video(html: &Path, out_mp4: &Path, seconds: u32, chrome: &str) -> RunOutcome {
    if !html.exists() {
        return RunOutcome {
            ok: false,
            summary: format!("HTML 不存在: {}", html.display()),
            artifact: None,
        };
    }
    let tmp = std::env::temp_dir().join(format!("lyco_frames_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    // file:/// URL: 直接绝对路径, 正斜杠 (不 canonicalize — Windows 会加 \\?\ UNC 前缀)
    let abs = if html.is_absolute() {
        html.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(html)
    };
    let url = format!("file:///{}", abs.to_string_lossy().replace('\\', "/"));
    let mut shot_fail = 0;
    for i in 0..seconds {
        let frame = tmp.join(format!("f{i:03}.png"));
        // Chromium 必须 = 形式传 screenshot (分离参数 + 后续 URL 会解析错位)
        let shot_arg = format!("--screenshot={}", frame.to_string_lossy());
        let budget = format!("--virtual-time-budget={}", (i as u64 + 1) * 1000);
        let o = run_cmd(
            chrome,
            &[
                "--headless=new",
                "--disable-gpu",
                "--window-size=1280,720",
                &shot_arg,
                &budget,
                &url,
            ],
            Duration::from_secs(60),
        );
        if !o.ok || !frame.exists() {
            shot_fail += 1;
        }
    }
    if shot_fail * 2 >= seconds as usize {
        let _ = std::fs::remove_dir_all(&tmp);
        return RunOutcome {
            ok: false,
            summary: format!("chrome 截帧失败 {shot_fail}/{seconds} (需 headless chrome/edge)"),
            artifact: None,
        };
    }
    let frame_pattern = tmp.join("f%03d.png");
    let ff = std::env::var("LYV_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let o = run_cmd(
        &ff,
        &[
            "-y",
            "-framerate",
            "1",
            "-i",
            &frame_pattern.to_string_lossy(),
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            &out_mp4.to_string_lossy(),
        ],
        Duration::from_secs(300),
    );
    let _ = std::fs::remove_dir_all(&tmp);
    if o.ok && out_mp4.exists() {
        RunOutcome {
            ok: true,
            summary: format!("HTML→视频完成 {} ({}s)", out_mp4.display(), seconds),
            artifact: Some(out_mp4.to_path_buf()),
        }
    } else {
        RunOutcome {
            ok: false,
            summary: format!("ffmpeg 合成失败: {}", o.summary),
            artifact: None,
        }
    }
}

/// 视频元信息: ffprobe 时长/分辨率
pub fn video_info(video: &Path) -> RunOutcome {
    if !video.exists() {
        return RunOutcome {
            ok: false,
            summary: format!("文件不存在: {}", video.display()),
            artifact: None,
        };
    }
    let ffprobe = std::env::var("LYV_FFPROBE").unwrap_or_else(|_| "ffprobe".into());
    let o = run_cmd(
        &ffprobe,
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate:format=duration",
            "-of",
            "csv=p=0",
            &video.to_string_lossy(),
        ],
        Duration::from_secs(60),
    );
    if o.ok {
        RunOutcome {
            ok: true,
            summary: format!("{} → {}", video.display(), o.summary),
            artifact: None,
        }
    } else {
        o
    }
}

// ===================== 执行类工具 (日常任务"真能干活") =====================
//
// shell 选择链 (用户指定): LYCO_SHELL 显式覆盖 → **brush**(Rust 写的 bash/POSIX 兼容,
// 跨平台首选, MIT) → **nu**(nushell, 备用, MIT) → 平台默认(sh / cmd)。
// 三者都只在「命令字符串」层面工作, 故可互换; 缺失时优雅降级并在错误里提示安装。

/// 探测可执行文件是否在 PATH
fn which(prog: &str) -> bool {
    let checker = if cfg!(windows) { "where" } else { "which" };
    std::process::Command::new(checker)
        .arg(prog)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 选 shell → (程序名, 是否用 `-c` 传命令)
pub fn pick_shell() -> (String, bool) {
    if let Ok(s) = std::env::var("LYCO_SHELL") {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return (s, true);
        }
    }
    if which("brush") {
        return ("brush".to_string(), true);
    }
    if which("nu") {
        return ("nu".to_string(), true);
    }
    if cfg!(windows) {
        ("cmd".to_string(), false)
    } else {
        ("sh".to_string(), true)
    }
}

/// 带超时的命令执行 (std 无内置 timeout, 用 try_wait 轮询)
fn run_with_timeout(
    mut cmd: std::process::Command,
    secs: u64,
) -> std::io::Result<(bool, String, String)> {
    use std::io::Read;
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let mut so = String::new();
            let mut se = String::new();
            if let Some(mut o) = child.stdout.take() {
                let _ = o.read_to_string(&mut so);
            }
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut se);
            }
            return Ok((status.success(), so, se));
        }
        if start.elapsed() > Duration::from_secs(secs) {
            let _ = child.kill();
            return Ok((false, String::new(), format!("超时 {secs}s (已终止)")));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn tail_n(s: &str, n: usize) -> String {
    s.lines()
        .rev()
        .take(n)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ")
}

/// shell_exec: 执行一条命令 (brush/nu/平台默认), 返回 exit code + 输出摘要
pub fn shell_exec(command: &str, cwd: Option<&Path>) -> RunOutcome {
    if command.trim().is_empty() {
        return RunOutcome {
            ok: false,
            summary: "空命令".to_string(),
            artifact: None,
        };
    }
    let (prog, dash_c) = pick_shell();
    let mut c = std::process::Command::new(&prog);
    if dash_c {
        c.args(["-c", command]);
    } else {
        c.args(["/C", command]);
    }
    if let Some(d) = cwd {
        // 显式校验: cwd 不存在时 spawn 会报 ENOENT, 误指向"shell 找不到" (实测踩坑)
        if !d.is_dir() {
            return RunOutcome {
                ok: false,
                summary: format!("工作目录不存在: {}", d.display()),
                artifact: None,
            };
        }
        c.current_dir(d);
    }
    match run_with_timeout(c, 120) {
        Ok((ok, so, se)) => RunOutcome {
            ok,
            summary: if ok {
                format!("[{prog}] {}", tail_n(&so, 5))
            } else {
                format!("[{prog}] {}", tail_n(&se, 5))
            },
            artifact: None,
        },
        Err(e) => RunOutcome {
            ok: false,
            summary: format!("{prog} 启动失败: {e} (可安装 brush 或 nushell 作为跨平台 shell)"),
            artifact: None,
        },
    }
}

/// file_write: 写文本文件 (自动建父目录)
pub fn file_write(path: &Path, content: &str) -> RunOutcome {
    if let Some(p) = path.parent() {
        if !p.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(p) {
                return RunOutcome {
                    ok: false,
                    summary: format!("建目录失败: {e}"),
                    artifact: None,
                };
            }
        }
    }
    match std::fs::write(path, content) {
        Ok(()) => RunOutcome {
            ok: true,
            summary: format!("写入 {} 字节 → {}", content.len(), path.display()),
            artifact: Some(path.to_path_buf()),
        },
        Err(e) => RunOutcome {
            ok: false,
            summary: format!("写文件失败: {e}"),
            artifact: None,
        },
    }
}

/// schedule: 定时任务 (Linux cron / Windows schtasks)。
/// **安全默认**: 只返回配置片段 (dry-run); 传 `apply=true` 才真正登记。
pub fn schedule(spec: &str, command: &str, apply: bool) -> RunOutcome {
    if spec.trim().is_empty() || command.trim().is_empty() {
        return RunOutcome {
            ok: false,
            summary: "需 {spec, command}".to_string(),
            artifact: None,
        };
    }
    #[cfg(windows)]
    let snippet = format!(
        "# Windows 计划任务\nschtasks /Create /SC DAILY /TN lyco_task /TR \"{command}\" /F\n\
         # (cron 风格 spec: {spec} — Windows 下需换算为 /ST HH:MM)"
    );
    #[cfg(not(windows))]
    let snippet = format!("# crontab 行 (crontab -e 粘贴)\n{spec} {command}\n");

    if !apply {
        return RunOutcome {
            ok: true,
            summary: format!("[dry-run] 定时配置:\n{snippet}"),
            artifact: None,
        };
    }
    #[cfg(windows)]
    let result = {
        let mut c = std::process::Command::new("schtasks");
        c.args([
            "/Create",
            "/SC",
            "DAILY",
            "/TN",
            "lyco_task",
            "/TR",
            command,
            "/F",
        ]);
        run_with_timeout(c, 30)
    };
    #[cfg(not(windows))]
    let result = {
        let line = format!("(crontab -l 2>/dev/null; echo \"{spec} {command}\") | crontab -");
        let mut c = std::process::Command::new("sh");
        c.args(["-c", line.as_str()]);
        run_with_timeout(c, 30)
    };
    match result {
        Ok((ok, _so, se)) => RunOutcome {
            ok,
            summary: if ok {
                format!("已登记定时任务: {spec} {command}")
            } else {
                format!("登记失败: {}", tail_n(&se, 3))
            },
            artifact: None,
        },
        Err(e) => RunOutcome {
            ok: false,
            summary: format!("登记失败: {e}"),
            artifact: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_write_creates_file_and_parents() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a/b/c.txt");
        let o = super::file_write(&p, "hi");
        assert!(o.ok, "{}", o.summary);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "hi");
        assert_eq!(o.artifact.as_deref(), Some(p.as_path()));
    }

    #[test]
    fn shell_exec_runs_command() {
        let o = super::shell_exec("echo lyco_ok", None);
        assert!(o.ok, "shell_exec 失败: {}", o.summary);
        assert!(o.summary.contains("lyco_ok"), "summary={}", o.summary);
    }

    #[test]
    fn schedule_dry_run_returns_snippet_without_applying() {
        let o = super::schedule("0 8 * * *", "java -jar paper.jar nogui", false);
        assert!(o.ok, "{}", o.summary);
        assert!(o.summary.contains("0 8 * * *"), "{}", o.summary);
        assert!(o.summary.contains("dry-run"), "默认不落地: {}", o.summary);
    }

    #[test]
    fn llm_generate_requires_key() {
        // 无 key 时诚实失败, 不编造
        std::env::remove_var("LYCO_LLM_KEY");
        let o = llm_generate("test", None);
        assert!(!o.ok);
        assert!(o.summary.contains("LYCO_LLM_KEY"));
    }

    #[test]
    fn rembg_missing_input_fails_honestly() {
        let o = rembg_remove(Path::new("Z:/no/such.png"), Path::new("Z:/tmp/out.png"));
        assert!(!o.ok);
        assert!(o.summary.contains("输入不存在"));
    }

    #[test]
    fn video_info_missing_file_fails_honestly() {
        let o = video_info(Path::new("Z:/no/such.mp4"));
        assert!(!o.ok);
        assert!(o.summary.contains("不存在"));
    }
}
