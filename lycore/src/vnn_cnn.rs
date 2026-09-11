//! vnn_cnn — 训练版 CNN 推理 (纯 Rust, 零依赖)
//!
//! 消费 CloudStudio 训练导出的 vnn_cnn_v2_weights.json ({shape, data} 格式):
//!   2×Conv(3x3,relu)+MaxPool → FC(relu) → FC → softmax
//! 与 PyTorch TinyCNN 前向数学语义一致 (padding=1, pool=2, 64x64 输入)。
//!
//! 模型路径解析: LYCO_VNN_CNN env → exe 同目录 models/vnn_cnn_v2_weights.json
//! → cwd models/vnn_cnn_v2_weights.json。找不到 → Ok(None), 调用方走规则回退
//! (诚实降级, 不伪装 CNN 就位)。

use anyhow::Context;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const CLASSES: &[&str] = &["terminal", "gui_window", "nature", "document"];

#[derive(Deserialize)]
struct WeightsFile {
    classes: Vec<String>,
    #[serde(default)]
    version: u32,
    weights: std::collections::BTreeMap<String, TensorData>,
}

#[derive(Deserialize)]
struct TensorData {
    shape: Vec<usize>,
    data: Vec<f32>,
}

/// 4 类 TinyCNN 推理器
pub struct CnnClassifier {
    conv1_w: Vec<f32>, // [8,1,3,3]
    conv1_b: Vec<f32>, // [8]
    conv2_w: Vec<f32>, // [16,8,3,3]
    conv2_b: Vec<f32>, // [16]
    fc1_w: Vec<f32>,   // [32,4096] (row-major, out×in)
    fc1_b: Vec<f32>,   // [32]
    fc2_w: Vec<f32>,   // [4,32]
    fc2_b: Vec<f32>,   // [4]
    classes: Vec<String>,
}

impl CnnClassifier {
    /// 从权重 JSON 加载。找不到/解析失败返回 Err (调用方决定是否回退规则)。
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("VNN CNN 权重读取失败: {}", path.display()))?;
        let f: WeightsFile =
            serde_json::from_str(&raw).context("VNN CNN 权重 JSON 解析失败")?;
        let get = |name: &str| -> anyhow::Result<Vec<f32>> {
            let t = f.weights.get(name).with_context(|| format!("缺少张量 {name}"))?;
            let expect: usize = t.shape.iter().product();
            anyhow::ensure!(t.data.len() == expect, "张量 {name} 数据量 {} ≠ shape 积 {expect}", t.data.len());
            Ok(t.data.clone())
        };
        Ok(Self {
            conv1_w: get("conv1.weight")?,
            conv1_b: get("conv1.bias")?,
            conv2_w: get("conv2.weight")?,
            conv2_b: get("conv2.bias")?,
            fc1_w: get("fc1.weight")?,
            fc1_b: get("fc1.bias")?,
            fc2_w: get("fc2.weight")?,
            fc2_b: get("fc2.bias")?,
            classes: f.classes,
        })
    }

    /// 按 env/默认位置解析权重文件; 不存在 → Ok(None)
    pub fn resolve() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("LYCO_VNN_CNN") {
            let p = PathBuf::from(p);
            if p.exists() {
                return Some(p);
            }
        }
        // cwd 变体: 仓库根 (lycore/assets)、crate 根 (assets)、models (历史位置)
        let exe_relative = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join("models/vnn_cnn_v2_weights.json")));
        let candidates: [Option<PathBuf>; 4] = [
            Some(PathBuf::from("lycore/assets/vnn_cnn_v2_weights.json")),
            Some(PathBuf::from("assets/vnn_cnn_v2_weights.json")),
            Some(PathBuf::from("models/vnn_cnn_v2_weights.json")),
            exe_relative,
        ];
        candidates
            .into_iter()
            .flatten()
            .find(|p| p.exists())
    }

    /// 单张 64x64 灰度 (0..255) → softmax 概率 (与 CLASSES/classes 对齐)
    pub fn classify(&self, img: &[f32; 64 * 64]) -> anyhow::Result<Vec<(String, f64)>> {
        // conv1: [8,1,3,3] pad=1 → 64x64 → relu → pool2 → 32x32
        let c1 = conv2d_relu_pool(img, &self.conv1_w, &self.conv1_b, 1, 8, 64, 2);
        // conv2: [16,8,3,3] pad=1, in=32x32 → relu → pool2 → 16x16
        let c2 = conv2d_relu_pool(&c1, &self.conv2_w, &self.conv2_b, 8, 16, 32, 2);
        // fc1: 4096 → 32 relu (PyTorch Linear 权重是 out×in row-major)
        let mut h1 = [0f32; 32];
        for (o, ob) in self.fc1_b.iter().enumerate() {
            let row = &self.fc1_w[o * 4096..(o + 1) * 4096];
            let s: f32 = c2.iter().zip(row).map(|(a, b)| a * b).sum::<f32>() + ob;
            h1[o] = s.max(0.0);
        }
        // fc2: 32 → 4 + softmax
        let mut logits = [0f64; 4];
        for (o, ob) in self.fc2_b.iter().enumerate() {
            let row = &self.fc2_w[o * 32..(o + 1) * 32];
            logits[o] = h1.iter().zip(row).map(|(a, b)| (*a * *b) as f64).sum::<f64>()
                + *ob as f64;
        }
        let max = logits.iter().cloned().fold(f64::MIN, f64::max);
        let exps: Vec<f64> = logits.iter().map(|&l| (l - max).exp()).collect();
        let sum: f64 = exps.iter().sum();
        Ok(self
            .classes
            .iter()
            .zip(exps.iter().map(|e| e / sum))
            .map(|(c, p)| (c.clone(), p))
            .collect())
    }
}

/// conv2d(pad=1) + relu + maxpool(stride=k): 输入 in_ch 通道平面, 输出 out_ch 平面
/// 尺寸: in_size → pool 后 in_size/2
fn conv2d_relu_pool(
    input: &[f32],
    w: &[f32],
    b: &[f32],
    in_ch: usize,
    out_ch: usize,
    in_size: usize,
    pool: usize,
) -> Vec<f32> {
    let out_size = in_size / 2;
    let mut out = vec![f32::MIN; out_ch * out_size * out_size];
    // 逐输出通道
    for oc in 0..out_ch {
        // 权重布局 [out_ch, in_ch, 3, 3]
        let wbase = &w[oc * in_ch * 9..(oc + 1) * in_ch * 9];
        let bias = b[oc];
        for oy in 0..out_size {
            for ox in 0..out_size {
                // 对应输入 2×2 池化窗 (pool=2, stride=2): y0=oy*2, x0=ox*2
                // 每个池化点先算 3x3 卷积 → relu, 再取 2x2 最大
                let mut pooled = f32::MIN;
                for py in 0..pool {
                    for px in 0..pool {
                        let iy = (oy * 2 + py) as isize;
                        let ix = (ox * 2 + px) as isize;
                        let mut acc = bias;
                        for ic in 0..in_ch {
                            let plane = &input[ic * in_size * in_size..(ic + 1) * in_size * in_size];
                            let wb = &wbase[ic * 9..(ic + 1) * 9];
                            for ky in 0..3isize {
                                for kx in 0..3isize {
                                    let sy = iy + ky - 1; // pad=1
                                    let sx = ix + kx - 1;
                                    if sy >= 0 && sy < in_size as isize && sx >= 0
                                        && sx < in_size as isize
                                    {
                                        acc += plane[(sy as usize) * in_size + sx as usize]
                                            * wb[(ky * 3 + kx) as usize];
                                    }
                                }
                            }
                        }
                        let r = acc.max(0.0);
                        if r > pooled {
                            pooled = r;
                        }
                    }
                }
                out[oc * out_size * out_size + oy * out_size + ox] = pooled;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_file_loads_and_classifies() {
        let path = CnnClassifier::resolve().expect("权重应能解析 (已提交 lycore/assets/)");
        let clf = CnnClassifier::load(&path).expect("加载失败");
        assert_eq!(clf.classes.len(), 4);
        // 暗底 + 稀疏亮行 (真实终端的分布, 训练域内) → terminal 概率最高
        let mut img = [10f32; 64 * 64];
        for (y, x0, x1) in [(20usize, 4usize, 40usize), (30, 4, 30), (40, 4, 36)] {
            for x in x0..x1 {
                img[y * 64 + x] = 180.0;
            }
        }
        let probs = clf.classify(&img).expect("classify 失败");
        let best = probs
            .iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .unwrap();
        assert_eq!(best.0, "terminal", "暗底亮行应判 terminal, got {probs:?}");
    }

    #[test]
    fn conv_pool_shapes_consistent() {
        // 单通道全 0 输入: 只验证形状与无 panic
        let input = vec![0f32; 64 * 64];
        let w = vec![0.1f32; 8 * 1 * 9];
        let b = vec![0f32; 8];
        let out = conv2d_relu_pool(&input, &w, &b, 1, 8, 64, 2);
        assert_eq!(out.len(), 8 * 32 * 32);
        let out2 = conv2d_relu_pool(&out, &vec![0.1f32; 16 * 8 * 9], &vec![0f32; 16], 8, 16, 32, 2);
        assert_eq!(out2.len(), 16 * 16 * 16);
    }
}
