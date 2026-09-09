//! vnn — 内部识图神经网络 (特征激活式, 纯 Rust)
//!
//! Python 原型 vnn_proto.py 的规则签名移植 — 特征提取改用 ffmpeg rawvideo 管道
//! (灰度 64x64), 无 OpenCV 依赖。激活/专家打分与 Python 版标定一致:
//!   终端截图 edge 0.01-0.03 / dark 0.99 → terminal top-1
//!
//! 真实 CNN 专家 (CloudStudio 训练) 就位后替换 expert_score 内部实现,
//! trait 结构不变。未训练专家诚实输出 needs_training (不伪装)。

use serde::Serialize;
use std::path::{Path, PathBuf};

/// 图像特征 (64x64 灰度)
#[derive(Debug, Clone)]
pub struct Features {
    pub edge_density: f64,
    pub dark_ratio: f64,
    /// 16 档梯度方向直方图 (归一化) — 保留给未来 CNN 对齐, v0 规则不用
    pub angle_hist: [f32; 16],
}

/// ffmpeg 抽灰度 64x64 raw → 特征
pub fn extract_features(ffmpeg: &str, image: &Path) -> anyhow::Result<Features> {
    use std::io::Read;
    let output = std::process::Command::new(ffmpeg)
        .args([
            "-v", "error",
            "-i", &image.to_string_lossy(),
            "-vf", "format=gray,scale=64:64",
            "-f", "rawvideo",
            "-pix_fmt", "gray",
            "-",
        ])
        .output()?;
    anyhow::ensure!(output.status.success(), "ffmpeg 特征提取失败");
    let raw = output.stdout;
    let w = 64usize;
    let h = raw.len() / w;
    anyhow::ensure!(h > 2, "raw 帧过小: {}", raw.len());

    // dark_ratio: 像素 < 64
    let dark = raw.iter().filter(|&&p| p < 64).count();
    let dark_ratio = dark as f64 / raw.len() as f64;

    // 边缘密度 + 梯度方向直方图 (Sobel 简化: |gx|+|gy| > 阈值 即边缘)
    let at = |x: usize, y: usize| raw[y * w + x] as f32;
    let mut edges = 0usize;
    let mut hist = [0f32; 16];
    let mut hist_total = 0f32;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = at(x + 1, y) - at(x - 1, y);
            let gy = at(x, y + 1) - at(x, y - 1);
            let mag = (gx.abs() + gy.abs()) / 2.0;
            if mag > 24.0 {
                edges += 1;
                let ang = gy.atan2(gx); // -pi..pi
                let bin = (((ang + std::f32::consts::PI) / (2.0 * std::f32::consts::PI))
                    * 16.0)
                    .clamp(0.0, 15.99) as usize;
                hist[bin] += mag;
                hist_total += mag;
            }
        }
    }
    let inner = (h - 2) * (w - 2);
    let edge_density = edges as f64 / inner as f64;
    if hist_total > 0.0 {
        for v in hist.iter_mut() {
            *v /= hist_total;
        }
    }
    Ok(Features { edge_density, dark_ratio, angle_hist: hist })
}

/// 特征神经元库
pub const NEURONS: &[&str] = &[
    "terminal", "gui_window", "document", "animal", "human_face", "nature",
];

/// 特征激活: top-k 神经元 (标定与 vnn_proto Python 版一致)
pub fn activate(feat: &Features, top_k: usize) -> Vec<(&'static str, f64)> {
    let ed = feat.edge_density;
    let dr = feat.dark_ratio;
    let mut scores: Vec<(&'static str, f64)> = vec![
        ("terminal", dr * (ed * 30.0).min(1.0)),
        ("gui_window", (1.0 - dr) * (ed * 20.0).min(1.0)),
        ("document", (1.0 - dr) * (1.0 - dr) * (ed * 15.0).min(1.0)),
        ("animal", (1.0 - ed * 8.0) * (0.5 - 0.4 * dr) * 0.6),
        (
            "human_face",
            (0.2 - (ed - 0.10).abs()).max(0.0) * 2.0 * (0.3 + 0.4 * (1.0 - dr)),
        ),
        ("nature", (1.0 - ed * 10.0) * (1.0 - dr) * 0.8),
    ];
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scores.truncate(top_k);
    let total: f64 = scores.iter().map(|s| s.1).sum::<f64>() + 1e-7;
    scores.into_iter().map(|(k, s)| (k, s / total)).collect()
}

/// 专家识别结果
#[derive(Debug, Serialize)]
pub struct ExpertScore {
    pub neuron: String,
    pub activation: f64,
    pub verdict: String,
    pub conf: f64,
    pub needs_training: bool,
}

/// 已训练专家 (v0: 界面类规则可给结构化分数) / 未训练 → 诚实 unknown
fn expert_score(neuron: &str, activation: f64, feat: &Features) -> ExpertScore {
    let trained = matches!(neuron, "terminal" | "gui_window" | "document");
    let (verdict, conf, needs_training) = if trained {
        let conf = (0.5 + feat.edge_density * 2.0).min(0.95);
        let desc = match neuron {
            "terminal" => "终端/命令行界面",
            "gui_window" => "GUI 窗口界面",
            _ => "文本文档",
        };
        (desc.to_string(), conf, false)
    } else {
        (format!("unknown_{neuron}"), 0.0, true)
    };
    ExpertScore { neuron: neuron.to_string(), activation, verdict, conf, needs_training }
}

/// VNN 主入口: 特征 → 激活 → 专家打分 → 汇总
/// 返回 (verdict, conf, experts, learning_queue)
pub fn identify(ffmpeg: &str, image: &Path) -> anyhow::Result<(String, f64, Vec<ExpertScore>, Vec<String>)> {
    let feat = extract_features(ffmpeg, image)?;
    let activated = activate(&feat, 3);
    let experts: Vec<ExpertScore> = activated
        .iter()
        .map(|(n, w)| expert_score(n, *w, &feat))
        .collect();
    let trained_best = experts
        .iter()
        .filter(|e| !e.needs_training)
        .max_by(|a, b| {
            (a.conf * a.activation)
                .partial_cmp(&(b.conf * b.activation))
                .unwrap()
        });
    let learning_queue: Vec<String> = experts
        .iter()
        .filter(|e| e.needs_training)
        .map(|e| e.neuron.clone())
        .collect();
    match trained_best {
        Some(e) => Ok((e.verdict.clone(), e.conf, experts, learning_queue)),
        None => Ok(("unknown".to_string(), 0.0, experts, learning_queue)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_calibration_terminal() {
        // 与 Python 版标定对齐: 暗底少边缘 → terminal top-1
        let f = Features { edge_density: 0.02, dark_ratio: 0.99, angle_hist: [0.0; 16] };
        let act = activate(&f, 3);
        assert_eq!(act[0].0, "terminal");
    }

    #[test]
    fn activation_calibration_nature() {
        // 亮底低边缘 → nature top-1
        let f = Features { edge_density: 0.016, dark_ratio: 0.0, angle_hist: [0.0; 16] };
        let act = activate(&f, 3);
        assert_eq!(act[0].0, "nature");
    }
}
