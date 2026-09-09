//! verifier cascade — lyv.py ocr_pass/级联路由 的 Rust 移植
//!
//! 级联 (DESIGN.md):
//!   1. 便宜确定性优先: OCR 提取文字, 置信度/关键词命中 → 直接用
//!   2. OCR 失败信号: conf < τ 或关键 token 未命中 → VNN 打分 (Rust 版未实现 → 学习队列)
//!   3. 仍低分 → 学习队列 (诚实, 不编造)
//!
//! OCR 本体: 进程外 tesseract (零绑定依赖, 端侧 agent 也能装 tesseract)。
//! Python 版细节: lev() 编辑距离 + min_conf=0.5 + 词长>=4 才允许距离<=2 模糊命中。

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
}
