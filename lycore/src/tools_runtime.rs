//! tools_runtime — Radxa 端侧工具集 (进程外调用, 零重依赖)
//!
//! 目标: Radxa A7A 上可跑的简单编排工具集, agent loop 的 tool_call 直达:
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

use anyhow::Context;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 工具输出: stdout/stderr 摘要 + 产物路径
pub struct RunOutcome {
    pub ok: bool,
    pub summary: String,
    pub artifact: Option<PathBuf>,
}

fn run_cmd(bin: &str, args: &[&str], _timeout: Duration) -> RunOutcome {
    match std::process::Command::new(bin)
        .args(args)
        .output()
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let ok = out.status.success();
            let tail = |s: &str, n: usize| {
                s.lines().rev().take(n).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ")
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
            o.summary,
            "或 LYCO_REMBG_BIN 指定路径"
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
    let model =
        std::env::var("LYCO_LLM_MODEL").unwrap_or_else(|_| "meta/llama-3.2-11b-vision-instruct".into());
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
            return RunOutcome { ok: false, summary: format!("http client: {e}"), artifact: None }
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
                        summary: format!("LLM 空回复: {}", v.to_string().chars().take(200).collect::<String>()),
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
    let abs = if html.is_absolute() { html.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(html) };
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
        RunOutcome { ok: false, summary: format!("ffmpeg 合成失败: {}", o.summary), artifact: None }
    }
}

/// 视频元信息: ffprobe 时长/分辨率
pub fn video_info(video: &Path) -> RunOutcome {
    if !video.exists() {
        return RunOutcome { ok: false, summary: format!("文件不存在: {}", video.display()), artifact: None };
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
        RunOutcome { ok: true, summary: format!("{} → {}", video.display(), o.summary), artifact: None }
    } else {
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
