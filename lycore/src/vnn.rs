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
/// CNN 通道 (v2 训练版) 就位时: 64x64 灰度直接分类, conf≥0.6 用 CNN 判定,
/// 否则回退规则激活通道 (诚实降级)。
/// 返回 (verdict, conf, experts, learning_queue)
pub fn identify(ffmpeg: &str, image: &Path) -> anyhow::Result<(String, f64, Vec<ExpertScore>, Vec<String>)> {
    let feat = extract_features(ffmpeg, image)?;

    // ---- CNN 通道 (v2/v3 训练版, 4 类) ----
    if let Some(wpath) = crate::vnn_cnn::CnnClassifier::resolve() {
        match crate::vnn_cnn::CnnClassifier::load(&wpath).and_then(|clf| {
            let mut gray = [0f32; 64 * 64];
            fill_gray(ffmpeg, image, &mut gray)?;
            // 一次前向同时拿 fc2 softmax 与 fc1 嵌入 (神经元库投票用)
            let probs = clf.classify(&gray)?;
            let votes = if clf.has_neurons() {
                clf.neuron_vote(&clf.embed(&gray), 5)
            } else {
                Vec::new()
            };
            Ok((probs, votes))
        }) {
            Ok((probs, votes)) => {
                let (best_cls, best_p) = probs
                    .iter()
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                    .map(|(c, p)| (c.clone(), *p))
                    .unwrap_or_else(|| ("unknown".into(), 0.0));
                // conf≥0.6 → CNN 判定 (低置信 → 规则回退 + 学习队列, 由 executor 处理)
                if best_p >= 0.6 {
                    // 分歧升级 (n=9 实测: 投票 9/9, fc2 8/9, 仅不确定带内分歧):
                    // fc2 ∈ [0.6,0.8) 且投票冠军 ≠ fc2 冠军且投票票数明显更高 → 采纳投票。
                    // 8 张 fc2 正确图 conf≥0.87 永不进带 → 零回归风险; 只救 f0 型不确定例。
                    let (final_cls, final_conf, escalated) = match votes.first() {
                        Some((vcls, vscore))
                            if best_p < 0.8
                                && vcls != &best_cls
                                && *vscore
                                    > votes.iter().find(|(c, _)| c == &best_cls).map_or(0.0, |(_, s)| *s)
                                    + 0.1 =>
                        {
                            (vcls.clone(), vscore.max(best_p * 0.9), true)
                        }
                        _ => (best_cls.clone(), best_p, false),
                    };
                    // experts: fc2 softmax 给判定, 神经元投票 (V9 库) 给可解释证据
                    let mut experts: Vec<ExpertScore> = probs
                        .iter()
                        .map(|(c, p)| ExpertScore {
                            neuron: c.clone(),
                            activation: *p,
                            verdict: c.clone(),
                            conf: *p,
                            needs_training: false,
                        })
                        .collect();
                    for (cls, votes) in votes.iter().take(3) {
                        experts.push(ExpertScore {
                            neuron: format!("neuron_vote:{cls}"),
                            activation: *votes,
                            verdict: if escalated && cls == &final_cls {
                                format!("激活投票 {votes:.2} (已采纳)")
                            } else {
                                format!("激活投票 {votes:.2}")
                            },
                            conf: 0.0,
                            needs_training: false,
                        });
                    }
                    let learning_queue: Vec<String> = probs
                        .iter()
                        .filter(|(_, p)| *p < 0.05)
                        .map(|(c, _)| format!("{c}:needs_data"))
                        .collect();
                    return Ok((final_cls, final_conf, experts, learning_queue));
                }
                // 低置信: 落到规则回退, 但带上 CNN 的低置信信息
                let low = ExpertScore {
                    neuron: best_cls.clone(),
                    activation: best_p,
                    verdict: format!("cnn_low_conf_{best_cls}"),
                    conf: best_p,
                    needs_training: true,
                };
                let (v, c, experts, q) = rule_identify(&feat)?;
                let mut experts = experts;
                experts.insert(0, low);
                let mut q = q;
                if best_p < 0.4 {
                    q.push(format!("cnn_low_conf:{best_cls}:{best_p:.2}"));
                }
                return Ok((v, c, experts, q));
            }
            Err(_) => {} // CNN 不可用 → 规则回退 (诚实降级)
        }
    }

    rule_identify(&feat)
}

/// 规则通道 (v0 激活式, 原 identify 主体)
fn rule_identify(feat: &Features) -> anyhow::Result<(String, f64, Vec<ExpertScore>, Vec<String>)> {
    let activated = activate(feat, 3);
    let experts: Vec<ExpertScore> = activated
        .iter()
        .map(|(n, w)| expert_score(n, *w, feat))
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

/// ffmpeg 抽 64x64 灰度填充 buffer (与 extract_features 同一滤镜链)
fn fill_gray(ffmpeg: &str, image: &Path, out: &mut [f32; 64 * 64]) -> anyhow::Result<()> {
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
    anyhow::ensure!(output.status.success(), "ffmpeg 灰度抽取失败");
    anyhow::ensure!(output.stdout.len() >= out.len(), "raw 帧过小: {}", output.stdout.len());
    for (i, &b) in output.stdout.iter().take(out.len()).enumerate() {
        out[i] = b as f32;
    }
    Ok(())
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
