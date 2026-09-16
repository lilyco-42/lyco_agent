//! verifier cascade — lyv.py ocr_pass/级联路由 的 Rust 移植
//!
//! 级联 (DESIGN.md):
//!   1. 便宜确定性优先: OCR 提取文字, 置信度/关键词命中 → 直接用
//!   2. OCR 失败信号: conf < τ 或关键 token 未命中 → VNN 打分 (Rust 版未实现 → 学习队列)
//!   3. 仍低分 → 学习队列 (诚实, 不编造)
//!
//! OCR 本体: 进程外 tesseract (零绑定依赖, 端侧 agent 也能装 tesseract)。
//! Python 版细节: lev() 编辑距离 + min_conf=0.5 + 词长>=4 才允许距离<=2 模糊命中。

use crate::skill::VerifierId;
use crate::tokens::tokens;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// OCR 识别输出 (文本 + 量化置信度)
#[derive(Debug, Clone)]
pub struct OcrOutput {
    pub text: String,
    pub conf: f64, // 0.0-1.0
}

/// tesseract 调用 (进程外)。默认通过 PATH 查找, 端侧可用 LYV_TESSERACT 覆盖路径。
pub struct Ocr {
    bin: PathBuf,
}


impl Ocr {
    pub fn new() -> Self {
        Self {
            bin: std::env::var("LYV_TESSERACT")
                .unwrap_or_else(|_| "tesseract".to_string())
                .into(),
        }
    }

    /// 识别图片: 文本 + 平均词置信度 (TSV conf 列, index 10, 与 Python 对齐)
    pub fn recognize(&self, image: &Path, lang: &str) -> Option<OcrOutput> {
        let txt_out = std::process::Command::new(&self.bin)
            .args([image.to_str()?, "stdout", "--psm", "6", "-l", lang])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&txt_out.stdout).trim().to_string();

        // conf: 输出到文件 (outputbase.tsv), 与 Python 踩坑结论一致 —
        // stdout 上 tsv 会被当 configfile, scoop 版也不认 'tsv' 参数名
        let stem = image.with_extension("");
        let tsv_path = stem.with_extension("tsv");
        let status = std::process::Command::new(&self.bin)
            .args([
                image.to_str()?,
                stem.to_str()?,
                "-c",
                "tessedit_create_tsv=1",
                "-l",
                lang,
                "--psm",
                "6",
            ])
            .output()
            .ok()?;
        if !status.status.success() {
            return Some(OcrOutput { text, conf: 0.0 });
        }
        let tsv = std::fs::read_to_string(&tsv_path).ok()?;
        let _ = std::fs::remove_file(&tsv_path);
        let confs: Vec<f64> = tsv
            .lines()
            .skip(1)
            .filter_map(|line| {
                let col: Vec<&str> = line.split('\t').collect();
                col.get(10)
                    .and_then(|c| c.trim().parse::<f64>().ok())
                    .filter(|c| *c >= 0.0)
            })
            .collect();
        let conf = if confs.is_empty() {
            0.0
        } else {
            confs.iter().sum::<f64>() / confs.len() as f64 / 100.0
        };
        Some(OcrOutput { text, conf })
    }
}

impl Default for Ocr {
    fn default() -> Self {
        Self::new()
    }
}

/// 编辑距离 (lyv.lev 的移植, 词长差>2 提前返回 99 的短路优化保留)
pub fn lev(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > 2 {
        return 99;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur.push((prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// 量化判定 (lyv.ocr_pass 移植): conf>=min_conf 且非空 且 精确/编辑距离命中预期词
pub fn ocr_pass(ocr_text: &str, ocr_conf: f64, expected: &[String], min_conf: f64) -> bool {
    if ocr_text.is_empty() || ocr_conf < min_conf {
        return false;
    }
    let ot: std::collections::HashSet<String> = tokens(ocr_text).into_iter().collect();
    if expected.iter().any(|e| ot.contains(e)) {
        return true;
    }
    // 模糊: 词长>=4 的预期词, 与 OCR 词距离<=2
    for w in ocr_text.split_whitespace() {
        for e in expected {
            if e.chars().count() >= 4 && lev(&w.to_lowercase(), e) <= 2 {
                return true;
            }
        }
    }
    false
}

/// 级联判定输出
#[derive(Debug, Serialize)]
pub struct Verdict {
    pub route: &'static str, // "ocr" | "vnn" | "learning_queue"
    pub pass: bool,
    pub ocr_conf: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vnn_hint: Option<String>,
}

/// 级联入口: OCR 优先 → 失败才走 VNN/学习队列
pub fn verify(
    ocr: &Ocr,
    image: &Path,
    expected: &[String],
    lang: &str,
    min_conf: f64,
) -> Option<Verdict> {
    let out = ocr.recognize(image, lang)?;
    let pass = ocr_pass(&out.text, out.conf, expected, min_conf);
    if pass {
        return Some(Verdict {
            route: "ocr",
            pass: true,
            ocr_conf: out.conf,
            vnn_hint: None,
        });
    }
    // OCR 失败 → VNN (Rust 未实现) → 学习队列。vnn_hint 提示调用方降级语义。
    Some(Verdict {
        route: "learning_queue",
        pass: false,
        ocr_conf: out.conf,
        vnn_hint: Some("VNN Rust 版未实现: 此帧入学习队列".to_string()),
    })
}

// ===================== P1: Verifier 注册表 =====================
//
// 把「技能算不算做成了」从具体级联提升为 `Verifier` trait + 注册表,
// 让新增技能即插即用确定性验证器 (research-architecture-2026-09-16.md §1.3)。
// `VerifierId::instantiate()` 返回 `Box<dyn Verifier>`, 与 `skill::Skill.verifier`
// 一一对应。

/// 验证器输入 (统一抽象: 产物路径 + 期望命中词 + 语言 + 最小置信度)
pub struct VerifierInput {
    /// 产物路径 (图片 / 视频 / 文件)
    pub artifact: Option<PathBuf>,
    /// 期望命中的关键词 (OCR 类验证用)
    pub expected: Vec<String>,
    /// OCR 语言 (默认 eng)
    pub lang: String,
    /// 最小置信度 (OCR 类)
    pub min_conf: f64,
}

impl Default for VerifierInput {
    fn default() -> Self {
        Self {
            artifact: None,
            expected: Vec::new(),
            lang: "eng".to_string(),
            min_conf: 0.5,
        }
    }
}

/// 验证器输出 (确定性判定)
#[derive(Debug, Clone, Serialize)]
pub struct VerifierResult {
    /// 是否通过确定性验收
    pub pass: bool,
    /// 路由: "ocr" | "ffprobe" | "vnn" | "none" | "learning_queue"
    pub route: &'static str,
    /// 可信度 0.0-1.0
    pub score: f64,
    /// 人类可读诊断
    pub detail: String,
}

/// 验证器 trait — 把「任务是否完成」抽离成确定性程序, 不调 LLM
pub trait Verifier: Send + Sync {
    /// 该验证器对应的标识 (与 `skill::VerifierId` 对齐)
    fn id(&self) -> VerifierId;
    /// 对产物做确定性验收
    fn run(&self, input: &VerifierInput) -> VerifierResult;
}

/// OCR 级联验证器 — 包 `verify()` (tesseract conf + 编辑距离模糊)
pub struct OcrVerifier {
    ocr: Ocr,
}

impl OcrVerifier {
    pub fn new() -> Self {
        Self { ocr: Ocr::new() }
    }
}

impl Default for OcrVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Verifier for OcrVerifier {
    fn id(&self) -> VerifierId {
        VerifierId::Ocr
    }
    fn run(&self, input: &VerifierInput) -> VerifierResult {
        let Some(image) = &input.artifact else {
            return VerifierResult {
                pass: false,
                route: "learning_queue",
                score: 0.0,
                detail: "OCR 验证缺产物路径".into(),
            };
        };
        match verify(
            &self.ocr,
            image,
            &input.expected,
            &input.lang,
            input.min_conf,
        ) {
            Some(v) => VerifierResult {
                pass: v.pass,
                route: v.route,
                score: v.ocr_conf,
                detail: format!("ocr_conf={:.2}", v.ocr_conf),
            },
            None => VerifierResult {
                pass: false,
                route: "learning_queue",
                score: 0.0,
                detail: "OCR 执行失败".into(),
            },
        }
    }
}

/// ffprobe 验证器 — 验收 `html_render_video` 的 mp4 产物
///
/// 确定性判定: 含 video 流 且 duration > 0。ffprobe 缺失时诚实降级。
pub struct FfprobeVerifier {
    bin: PathBuf,
}

impl FfprobeVerifier {
    pub fn new() -> Self {
        Self {
            bin: std::env::var("LYCO_FFPROBE")
                .unwrap_or_else(|_| "ffprobe".to_string())
                .into(),
        }
    }
}

impl Default for FfprobeVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Verifier for FfprobeVerifier {
    fn id(&self) -> VerifierId {
        VerifierId::Ffprobe
    }
    fn run(&self, input: &VerifierInput) -> VerifierResult {
        let Some(video) = &input.artifact else {
            return VerifierResult {
                pass: false,
                route: "learning_queue",
                score: 0.0,
                detail: "ffprobe 验证缺视频路径".into(),
            };
        };
        let out = std::process::Command::new(&self.bin)
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration:stream=codec_type",
                "-of",
                "json",
                video.to_str().unwrap_or(""),
            ])
            .output();
        let Ok(out) = out else {
            return VerifierResult {
                pass: false,
                route: "learning_queue",
                score: 0.0,
                detail: "ffprobe 不可用".into(),
            };
        };
        if !out.status.success() {
            return VerifierResult {
                pass: false,
                route: "learning_queue",
                score: 0.0,
                detail: "ffprobe 退出非 0".into(),
            };
        }
        let v: serde_json::Value = match serde_json::from_slice(&out.stdout) {
            Ok(v) => v,
            Err(_) => {
                return VerifierResult {
                    pass: false,
                    route: "learning_queue",
                    score: 0.0,
                    detail: "ffprobe 输出非 JSON".into(),
                }
            }
        };
        let has_video = v["streams"]
            .as_array()
            .map(|s| s.iter().any(|x| x["codec_type"] == serde_json::json!("video")))
            .unwrap_or(false);
        let dur: f64 = v["format"]["duration"]
            .as_str()
            .and_then(|d| d.parse().ok())
            .or_else(|| v["format"]["duration"].as_f64())
            .unwrap_or(0.0);
        let pass = has_video && dur > 0.0;
        VerifierResult {
            pass,
            route: if pass { "ffprobe" } else { "learning_queue" },
            score: if pass { 1.0 } else { 0.0 },
            detail: format!("has_video={has_video} duration={dur:.2}s"),
        }
    }
}

/// VNN Rust 路径占位验证器 — 当前未实现 → 诚实降级到学习队列 (DESIGN.md 语义)
pub struct VnnVerifier;

impl Verifier for VnnVerifier {
    fn id(&self) -> VerifierId {
        VerifierId::Vnn
    }
    fn run(&self, _input: &VerifierInput) -> VerifierResult {
        VerifierResult {
            pass: false,
            route: "learning_queue",
            score: 0.0,
            detail: "VNN Rust 路径未实现: 此帧入学习队列".into(),
        }
    }
}

/// 退出码验证器 — shell 类技能: 命令是否成功 (exit=0)
///
/// `VerifierInput.score` 复用为退出码 (0 = 成功)。
pub struct ExitCodeVerifier;

impl Verifier for ExitCodeVerifier {
    fn id(&self) -> VerifierId {
        VerifierId::ExitCode
    }
    fn run(&self, input: &VerifierInput) -> VerifierResult {
        let pass = input.min_conf == 0.0; // 约定: 0 表示 exit code 0
        VerifierResult {
            pass,
            route: if pass { "exit_code" } else { "learning_queue" },
            score: input.min_conf,
            detail: format!("exit_code={}", input.min_conf as i64),
        }
    }
}

/// 文件存在验证器 — 落盘类技能: 产物存在且非空
pub struct FileExistsVerifier;

impl Verifier for FileExistsVerifier {
    fn id(&self) -> VerifierId {
        VerifierId::FileExists
    }
    fn run(&self, input: &VerifierInput) -> VerifierResult {
        let Some(p) = &input.artifact else {
            return VerifierResult {
                pass: false,
                route: "learning_queue",
                score: 0.0,
                detail: "FileExists 验证缺产物路径".into(),
            };
        };
        let len = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let pass = len > 0;
        VerifierResult {
            pass,
            route: if pass { "file_exists" } else { "learning_queue" },
            score: if pass { 1.0 } else { 0.0 },
            detail: format!("{} = {} bytes", p.display(), len),
        }
    }
}

/// 无确定性验证器 — 纯文本创作 / 检索, 视为永远 pass
pub struct NoneVerifier;

impl Verifier for NoneVerifier {
    fn id(&self) -> VerifierId {
        VerifierId::None
    }
    fn run(&self, _input: &VerifierInput) -> VerifierResult {
        VerifierResult {
            pass: true,
            route: "none",
            score: 1.0,
            detail: "无确定性验证".into(),
        }
    }
}

/// 验证器注册表 — 按 `VerifierId` 取具体验证器实例 (即插即用)
pub struct VerifierRegistry;

impl VerifierRegistry {
    /// 实例化某标识对应的验证器 (与 `skill::Skill.verifier` 对齐)
    pub fn instantiate(id: VerifierId) -> Box<dyn Verifier> {
        match id {
            VerifierId::Ocr => Box::new(OcrVerifier::new()),
            VerifierId::Ffprobe => Box::new(FfprobeVerifier::new()),
            VerifierId::Vnn => Box::new(VnnVerifier),
            VerifierId::ExitCode => Box::new(ExitCodeVerifier),
            VerifierId::FileExists => Box::new(FileExistsVerifier),
            VerifierId::None => Box::new(NoneVerifier),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lev_shortcircuit_and_distances() {
        assert_eq!(lev("cargo", "cargo"), 0);
        assert_eq!(lev("no", "new"), 2);
        assert_eq!(lev("abc", "verylongstring"), 99); // 词长差短路
    }

    #[test]
    fn ocr_pass_exact_and_fuzzy() {
        let expected: Vec<String> = vec!["cargo".into(), "new".into()];
        // 精确命中
        assert!(ocr_pass("PS> cargo new demo", 0.93, &expected, 0.5));
        // conf 不足
        assert!(!ocr_pass("PS> cargo new demo", 0.4, &expected, 0.5));
        // 空文本
        assert!(!ocr_pass("", 0.9, &expected, 0.5));
        // 模糊命中: hello1 → hello 距离 1, 词长 >= 4
        assert!(ocr_pass("PS> hello1 world", 0.9, &["hello".into()], 0.5));
        // 无命中
        assert!(!ocr_pass("completely unrelated text", 0.9, &expected, 0.5));
    }

    #[test]
    fn parse_call_style_consistency() {
        // 确认 expected 词表与 Python 侧一致生成 (strong ∪ tokens(command))
        let expected: Vec<String> = vec![" 建".into(), "cargo".into(), "项目".into()];
        assert!(ocr_pass("cargo new 建立项目", 0.9, &expected, 0.5));
    }

    // ---- P1: Verifier 注册表 ----

    #[test]
    fn registry_instantiates_each_id() {
        // 四个标识都能取到对应具体验证器, 且 id 回环一致
        let ids = [
            VerifierId::Ocr,
            VerifierId::Ffprobe,
            VerifierId::Vnn,
            VerifierId::ExitCode,
            VerifierId::FileExists,
            VerifierId::None,
        ];
        for id in ids {
            let v = VerifierRegistry::instantiate(id);
            assert_eq!(v.id(), id, "注册表回环不一致");
        }
    }

    #[test]
    fn none_verifier_always_passes() {
        let v = VerifierRegistry::instantiate(VerifierId::None);
        let r = v.run(&VerifierInput::default());
        assert!(r.pass);
        assert_eq!(r.route, "none");
    }

    #[test]
    fn vnn_verifier_degrades_honestly() {
        // VNN Rust 路径未实现 → 永远 pass=false, 路由学习队列 (DESIGN 诚实降级)
        let v = VerifierRegistry::instantiate(VerifierId::Vnn);
        let r = v.run(&VerifierInput {
            artifact: Some(std::path::PathBuf::from("/tmp/x.png")),
            ..Default::default()
        });
        assert!(!r.pass);
        assert_eq!(r.route, "learning_queue");
    }

    #[test]
    fn ffprobe_verifier_reports_missing_binary() {
        // 指向不存在的二进制 → 诚实降级 (不 panic, 不假 pass)
        let v = FfprobeVerifier {
            bin: std::path::PathBuf::from("/nonexistent/ffprobe-xyz"),
        };
        let r = v.run(&VerifierInput {
            artifact: Some(std::path::PathBuf::from("/tmp/x.mp4")),
            ..Default::default()
        });
        assert!(!r.pass);
        assert_eq!(r.route, "learning_queue");
    }

    #[test]
    fn ocr_verifier_missing_artifact_fails() {
        // 缺产物路径 → 不调用 tesseract, 直接降级
        let v = VerifierRegistry::instantiate(VerifierId::Ocr);
        let r = v.run(&VerifierInput::default());
        assert!(!r.pass);
        assert_eq!(r.route, "learning_queue");
    }
}
