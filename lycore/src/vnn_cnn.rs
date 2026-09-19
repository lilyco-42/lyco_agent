//! vnn_cnn — 训练版 CNN 推理 (纯 Rust, 零依赖)
//!
//! 消费 CloudStudio 训练导出的 vnn_cnn_v{2,3}_weights.json ({shape, data} 格式):
//!   2×Conv(3x3,relu)+MaxPool → FC(relu) → FC → softmax
//! 与 PyTorch TinyCNN 前向数学语义一致 (padding=1, pool=2, 64x64 输入)。
//!
//! 模型路径解析: LYCO_VNN_CNN env → 各位置 v3 优先(含神经元库)后 v2。
//! 找不到 → None, 调用方走规则回退
//! (诚实降级, 不伪装 CNN 就位)。

use anyhow::Context;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const CLASSES: &[&str] = &["terminal", "gui_window", "nature", "document"];

#[derive(Deserialize)]
struct WeightsFile {
    classes: Vec<String>,
    /// 权重文件格式版本: 反序列化后当前不读, 保留供未来版本分派
    #[serde(default)]
    #[allow(dead_code)]
    version: u32,
    weights: std::collections::BTreeMap<String, TensorData>,
    /// V9+ 权重带数据驱动神经元库 (fc1 嵌入空间 K-means 原型); v2 无此字段 → 空
    #[serde(default)]
    neurons: Vec<NeuronData>,
}

#[derive(Deserialize, Clone)]
struct NeuronData {
    neuron: String,
    class: String,
    prototype: Vec<f32>,
    radius: f32,
}

/// 单个特征神经元 (嵌入空间原型 + 归属类 + 该类样本到它的最大距离)
#[derive(Debug, Clone)]
pub struct Neuron {
    pub name: String,
    pub class: String,
    pub prototype: [f32; 32],
    pub radius: f32,
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
    /// 神经元库 (V9 聚类生成); 空 = 权重文件未含 (v2), 投票不可用
    neurons: Vec<Neuron>,
}

impl CnnClassifier {
    /// 从权重 JSON 加载。找不到/解析失败返回 Err (调用方决定是否回退规则)。
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("VNN CNN 权重读取失败: {}", path.display()))?;
        let f: WeightsFile = serde_json::from_str(&raw).context("VNN CNN 权重 JSON 解析失败")?;
        let get = |name: &str| -> anyhow::Result<Vec<f32>> {
            let t = f
                .weights
                .get(name)
                .with_context(|| format!("缺少张量 {name}"))?;
            let expect: usize = t.shape.iter().product();
            anyhow::ensure!(
                t.data.len() == expect,
                "张量 {name} 数据量 {} ≠ shape 积 {expect}",
                t.data.len()
            );
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
            // 原型维度非 32 的条目直接丢弃 (宁缺毋滥, 不污染投票)
            neurons: f
                .neurons
                .into_iter()
                .filter(|n| n.prototype.len() == 32)
                .map(|n| Neuron {
                    name: n.neuron,
                    class: n.class,
                    prototype: <[f32; 32]>::try_from(n.prototype).unwrap(),
                    radius: n.radius,
                })
                .collect(),
        })
    }

    /// 神经元库是否可用 (v2 权重无此字段 → false, 调用方跳过投票)
    pub fn has_neurons(&self) -> bool {
        !self.neurons.is_empty()
    }

    /// fc1 32 维嵌入 (神经元库所在的嵌入空间)
    pub fn embed(&self, img: &[f32; 64 * 64]) -> [f32; 32] {
        let c1 = conv2d_relu_pool(img, &self.conv1_w, &self.conv1_b, 1, 8, 64, 2);
        let c2 = conv2d_relu_pool(&c1, &self.conv2_w, &self.conv2_b, 8, 16, 32, 2);
        let mut h1 = [0f32; 32];
        for (o, ob) in self.fc1_b.iter().enumerate() {
            let row = &self.fc1_w[o * 4096..(o + 1) * 4096];
            let s: f32 = c2.iter().zip(row).map(|(a, b)| a * b).sum::<f32>() + ob;
            h1[o] = s.max(0.0);
        }
        h1
    }

    /// 特征神经元投票: 取嵌入空间 top_k 最近原型, 按「距离越近票越高」累加到各自归属类。
    /// 返回按票数降序的 (类, 票数)。无可解释原型命中 → 空 Vec。
    /// 与 fc2 softmax 互补: fc2 给判定, 本函数给「哪些神经元被激活」的可解释证据。
    pub fn neuron_vote(&self, emb: &[f32; 32], top_k: usize) -> Vec<(String, f64)> {
        if self.neurons.is_empty() {
            return Vec::new();
        }
        let mut ds: Vec<(usize, f64)> = self
            .neurons
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let d: f64 = emb
                    .iter()
                    .zip(n.prototype.iter())
                    .map(|(a, b)| ((*a - *b) as f64).powi(2))
                    .sum::<f64>()
                    .sqrt();
                (i, d)
            })
            .collect();
        ds.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        let mut tally: std::collections::BTreeMap<String, f64> = Default::default();
        for (i, d) in ds.into_iter().take(top_k) {
            let n = &self.neurons[i];
            // 半径内给满票并按接近程度衰减; 半径外仍计薄票 (保持类间可比)
            let vote = if d <= n.radius as f64 {
                1.0 - 0.5 * (d / n.radius.max(1e-6) as f64)
            } else {
                (n.radius as f64 / d).min(0.5)
            };
            *tally.entry(n.class.clone()).or_insert(0.0) += vote;
        }
        let mut out: Vec<(String, f64)> = tally.into_iter().collect();
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        out
    }

    /// 按 env/默认位置解析权重文件; 不存在 → Ok(None)
    pub fn resolve() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("LYCO_VNN_CNN") {
            let p = PathBuf::from(p);
            if p.exists() {
                return Some(p);
            }
        }
        // 优先 v3 (含 V9 神经元库 → 分歧升级生效), 回退 v2 (fc2-only)。
        // 每个位置先 v3 后 v2; 两版权重 fc2 在 terminal 类同为 9/9, v3 额外给可解释投票。
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.to_path_buf()));
        let bases: [Option<PathBuf>; 4] = [
            Some(PathBuf::from("lycore/assets")),
            Some(PathBuf::from("assets")),
            Some(PathBuf::from("models")),
            exe_dir,
        ];
        let candidates: Vec<PathBuf> = bases
            .into_iter()
            .flatten()
            .flat_map(|b| {
                [
                    b.join("vnn_cnn_v3_weights.json"),
                    b.join("vnn_cnn_v2_weights.json"),
                ]
            })
            .collect();
        candidates.into_iter().find(|p| p.exists())
    }

    /// 单张 64x64 灰度 (0..255) → softmax 概率 (与 CLASSES/classes 对齐)
    pub fn classify(&self, img: &[f32; 64 * 64]) -> anyhow::Result<Vec<(String, f64)>> {
        // fc2: 32 → 4 + softmax (嵌入由 embed() 复用, 与神经元库同一空间)
        let h1 = self.embed(img);
        let mut logits = [0f64; 4];
        for (o, ob) in self.fc2_b.iter().enumerate() {
            let row = &self.fc2_w[o * 32..(o + 1) * 32];
            logits[o] = h1
                .iter()
                .zip(row)
                .map(|(a, b)| (*a * *b) as f64)
                .sum::<f64>()
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
                            let plane =
                                &input[ic * in_size * in_size..(ic + 1) * in_size * in_size];
                            let wb = &wbase[ic * 9..(ic + 1) * 9];
                            for ky in 0..3isize {
                                for kx in 0..3isize {
                                    let sy = iy + ky - 1; // pad=1
                                    let sx = ix + kx - 1;
                                    if sy >= 0
                                        && sy < in_size as isize
                                        && sx >= 0
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

    /// 锁死"默认路径优先 v3": 上一轮修过 resolve() 默认返回 v2 → 神经元投票/分歧升级
    /// 全部休眠 (特性发布即失效)。v2 与 v3 对该合成图都给 terminal, 所以 classify
    /// 断言抓不到这个回归 —— 必须直接断言 resolve 命中的文件名与库可用性。
    #[test]
    fn resolve_prefers_v3_so_voting_is_live() {
        std::env::remove_var("LYCO_VNN_CNN");
        let path = CnnClassifier::resolve().expect("应能解析权重");
        assert!(
            path.to_string_lossy().contains("v3"),
            "resolve 应优先 v3 (含神经元库), 否则投票/升级休眠: {}",
            path.display()
        );
        let clf = CnnClassifier::load(&path).expect("加载失败");
        assert!(clf.has_neurons(), "v3 必带神经元库");
    }

    /// v2 权重无 neurons 字段 → has_neurons false, 投票返回空 (向后兼容, 不 panic)
    #[test]
    fn v2_weights_have_no_neurons() {
        use serde_json::json;
        let arr = |v: f32, n: usize| -> Vec<f32> { vec![v; n] };
        let doc = json!({
            "classes": ["terminal", "gui_window", "nature", "document"],
            "version": 2,
            "weights": {
                "conv1.weight": {"shape": [8,1,3,3], "data": arr(0.1f32, 72)},
                "conv1.bias":   {"shape": [8], "data": arr(0.0f32, 8)},
                "conv2.weight": {"shape": [16,8,3,3], "data": arr(0.1f32, 1152)},
                "conv2.bias":   {"shape": [16], "data": arr(0.0f32, 16)},
                "fc1.weight":   {"shape": [32,4096], "data": arr(0.01f32, 131072)},
                "fc1.bias":     {"shape": [32], "data": arr(0.0f32, 32)},
                "fc2.weight":   {"shape": [4,32], "data": arr(0.1f32, 128)},
                "fc2.bias":     {"shape": [4], "data": arr(0.0f32, 4)}
            }
        });
        let dir = std::env::temp_dir().join("lyco_v2_test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("v2.json");
        std::fs::write(&p, doc.to_string()).unwrap();
        let clf = CnnClassifier::load(&p).expect("v2 应能加载");
        assert!(!clf.has_neurons(), "v2 无神经元库");
        assert!(
            clf.neuron_vote(&[0.0f32; 32], 5).is_empty(),
            "无库投票应为空"
        );
    }

    /// V9 神经元库接入: v3 权重 (12 原型) → 暗底亮行嵌入投票, terminal 应得票
    #[test]
    fn v3_neuron_vote_prefers_terminal() {
        let p =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/vnn_cnn_v3_weights.json");
        let clf = match CnnClassifier::load(&p) {
            Ok(c) => c,
            Err(_) => return, // v3 权重不在则跳过 (非必需资产)
        };
        assert!(clf.has_neurons(), "v3 应含神经元库");
        let mut img = [10f32; 64 * 64];
        for (y, x0, x1) in [(20usize, 4usize, 40usize), (30, 4, 30), (40, 4, 36)] {
            for x in x0..x1 {
                img[y * 64 + x] = 180.0;
            }
        }
        let votes = clf.neuron_vote(&clf.embed(&img), 5);
        assert!(!votes.is_empty(), "有库应产出投票");
        assert_eq!(
            votes[0].0, "terminal",
            "暗底亮行神经元投票应 terminal 领先, got {votes:?}"
        );
    }

    #[test]
    fn conv_pool_shapes_consistent() {
        // 单通道全 0 输入: 只验证形状与无 panic
        let input = vec![0f32; 64 * 64];
        let w = vec![0.1f32; 8 * 9];
        let b = vec![0f32; 8];
        let out = conv2d_relu_pool(&input, &w, &b, 1, 8, 64, 2);
        assert_eq!(out.len(), 8 * 32 * 32);
        let out2 = conv2d_relu_pool(&out, &vec![0.1f32; 16 * 8 * 9], &[0f32; 16], 8, 16, 32, 2);
        assert_eq!(out2.len(), 16 * 16 * 16);
    }
}
