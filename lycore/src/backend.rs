//! backend — 硬件无关的执行后端抽象
//!
//! ## 立场
//! lyco 的目标是**普惠设备**, 不是某一块板子。NPU 型号会一直变
//! (Vivante / Rockchip / Qualcomm / Intel / Apple / Android NNAPI / 浏览器 WebNN),
//! 写死任何一个都是把路走窄。
//!
//! ## 两条原则
//! 1. **能力保证** — CPU 后端恒可用。用户拿到 lyco **一定能跑**, 只是快慢之别。
//!    加速是"锦上添花", 绝不是"能不能用"的前提。
//! 2. **加速可选** — NPU/GPU 后端运行时 probe, 探不到就**静默降级**, 不报错、不阻塞。
//!    能力层永远只说"我要一次推理", 不关心跑在哪块硅片上。
//!
//! ## 探测方式
//! 设备节点 (`/dev/galcore` 等) + 平台特征 (`cfg!(target_os)`) + 环境变量覆盖
//! (`LYCO_BACKEND`)。探测只读不写, 失败即 false, **永不 panic**。

use serde::Serialize;
use std::path::Path;

/// 设备大类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DeviceClass {
    /// 通用 CPU — 永远可用, 兜底担当
    Cpu,
    /// GPU / 通用加速器
    Gpu,
    /// 专用 NPU
    Npu,
}

/// 一个可用后端
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Backend {
    /// 稳定标识 (可写进配置/日志/遥测)
    pub id: &'static str,
    pub device: DeviceClass,
    /// 面向用户的名字
    pub display: &'static str,
    /// 优先级: 越大越优先 (同设备类内比较)
    pub priority: u8,
}

/// 全后端目录 — 新增硬件支持只需往这里加一行 (能力层零改动)
pub fn catalog() -> &'static [Backend] {
    &[
        // ---- CPU: 兜底, 恒可用 ----
        Backend {
            id: "cpu",
            device: DeviceClass::Cpu,
            display: "通用 CPU",
            priority: 0,
        },
        // ---- NPU: 各家加速器 ----
        Backend {
            id: "npu-nnapi",
            device: DeviceClass::Npu,
            display: "Android NNAPI (覆盖高通/MTK/三星)",
            priority: 50,
        },
        Backend {
            id: "npu-coreml",
            device: DeviceClass::Npu,
            display: "Apple Neural Engine",
            priority: 50,
        },
        Backend {
            id: "npu-openvino",
            device: DeviceClass::Npu,
            display: "Intel NPU / OpenVINO",
            priority: 50,
        },
        Backend {
            id: "npu-rknn",
            device: DeviceClass::Npu,
            display: "Rockchip RKNN",
            priority: 40,
        },
        Backend {
            id: "npu-qnn",
            device: DeviceClass::Npu,
            display: "Qualcomm Hexagon QNN",
            priority: 40,
        },
        Backend {
            id: "npu-vip9000",
            device: DeviceClass::Npu,
            display: "Vivante VIP9000 (全志/Radxa)",
            priority: 40,
        },
        Backend {
            id: "npu-webnn",
            device: DeviceClass::Npu,
            display: "浏览器 WebNN",
            priority: 30,
        },
        // ---- GPU: 通用加速 ----
        Backend {
            id: "gpu-vulkan",
            device: DeviceClass::Gpu,
            display: "Vulkan GPU",
            priority: 20,
        },
        Backend {
            id: "gpu-metal",
            device: DeviceClass::Gpu,
            display: "Apple Metal",
            priority: 20,
        },
        Backend {
            id: "gpu-cuda",
            device: DeviceClass::Gpu,
            display: "NVIDIA CUDA",
            priority: 20,
        },
    ]
}

fn exists(p: &str) -> bool {
    Path::new(p).exists()
}

/// 该后端在这台机器上是否可用
///
/// 只读探测, 任何异常都判不可用。**CPU 恒 true** — 这是"一定能跑"的保证。
pub fn probe(b: &Backend) -> bool {
    match b.id {
        "cpu" => true,
        // 设备节点
        "npu-vip9000" => exists("/dev/galcore") || exists("/dev/npu"),
        "npu-rknn" => exists("/dev/rknpu") || exists("/dev/rknpu_service"),
        // 平台级运行时, 由 OS 决定可用性
        "npu-nnapi" | "npu-webnn" => cfg!(target_os = "android"),
        "npu-coreml" | "gpu-metal" => cfg!(target_os = "macos") || cfg!(target_os = "ios"),
        "npu-openvino" => {
            cfg!(target_os = "linux") && (exists("/dev/accel/accel0") || exists("/dev/dri"))
        }
        "npu-qnn" => cfg!(target_os = "android") || exists("/dev/npu"),
        "gpu-vulkan" => exists("/dev/dri") || cfg!(target_vendor = "pc-windows-msvc"),
        "gpu-cuda" => exists("/dev/nvidiactl") || exists("/dev/nvidia0"),
        _ => false,
    }
}

/// 本机会话可用的后端 (按优先级降序)。
///
/// **恒非空** — 至少含 CPU。这条不变量是整个"普惠"承诺的工程表达。
pub fn available() -> Vec<Backend> {
    let mut v: Vec<Backend> = catalog().iter().copied().filter(probe).collect();
    v.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(b.id)));
    v
}

/// 选后端: 显式偏好 > probe 通过的最高优先级 > CPU。
///
/// 偏好项的 id 不存在或 probe 不过时**静默降级**, 不报错 —
/// 用户写错配置不该让整个能力挂掉。
pub fn pick(prefer: Option<&str>) -> Backend {
    let avail = available();
    if let Some(want) = prefer {
        let want = want.trim();
        if let Some(b) = avail.iter().find(|b| b.id == want) {
            return *b;
        }
        // 偏好但本机没有 → 降级到最高优先级可用项 (兜底至少是 cpu)
        return avail.into_iter().next().unwrap_or(CPU);
    }
    avail.into_iter().next().unwrap_or(CPU)
}

/// CPU 后端常量 (兜底值, 不依赖 probe)
pub const CPU: Backend = Backend {
    id: "cpu",
    device: DeviceClass::Cpu,
    display: "通用 CPU",
    priority: 0,
};

/// 推理结果的元信息 — 输出里必带, 便于排障与跨设备公平比较
#[derive(Debug, Clone, Serialize)]
pub struct RunMeta {
    pub backend: &'static str,
    pub device: DeviceClass,
    pub elapsed_ms: u64,
    /// 是否因偏好项不可用而降级 (排障用: 用户以为在跑 NPU, 实际是 CPU)
    pub degraded: bool,
}

/// 按偏好选后端并给出 RunMeta 骨架
pub fn resolve(prefer: Option<&str>) -> (Backend, RunMeta) {
    let want = prefer.map(str::trim);
    let b = pick(prefer);
    let degraded = match want {
        Some(w) => !w.is_empty() && w != "cpu" && b.id != w,
        None => false,
    };
    (
        b,
        RunMeta {
            backend: b.id,
            device: b.device,
            elapsed_ms: 0,
            degraded,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_is_always_available() {
        assert!(probe(&CPU));
        let a = available();
        assert!(
            a.iter().any(|b| b.device == DeviceClass::Cpu),
            "普惠不变量: 任何机器都必须有 CPU 兜底"
        );
        assert!(!a.is_empty());
    }

    #[test]
    fn available_is_sorted_by_priority_desc() {
        let a = available();
        for w in a.windows(2) {
            assert!(w[0].priority >= w[1].priority, "{:?} vs {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn catalog_ids_are_unique() {
        let ids: Vec<&str> = catalog().iter().map(|b| b.id).collect();
        let mut uniq = ids.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(ids.len(), uniq.len(), "后端 id 不能重复: {ids:?}");
    }

    /// 普惠核心测试: 指定一个本机没有的后端, 必须降级而不是崩/返回空
    #[test]
    fn unavailable_preference_degrades_gracefully() {
        let (b, meta) = resolve(Some("npu-that-does-not-exist"));
        assert_eq!(b.id, pick(None).id, "应降级到最高优先级可用后端");
        assert!(meta.degraded, "偏好不可用必须标记 degraded, 便于排障");
    }

    #[test]
    fn explicit_cpu_never_marks_degraded() {
        let (b, meta) = resolve(Some("cpu"));
        assert_eq!(b.device, DeviceClass::Cpu);
        assert!(!meta.degraded);
    }

    #[test]
    fn empty_preference_is_not_degraded() {
        let (_b, meta) = resolve(Some(""));
        assert!(!meta.degraded);
    }

    #[test]
    fn pick_never_panics_for_any_catalog_id() {
        for b in catalog() {
            let selected = pick(Some(b.id));
            assert!(available().contains(&selected));
        }
    }

    #[test]
    fn covers_all_major_accelerator_families() {
        let ids: Vec<&str> = catalog().iter().map(|b| b.id).collect();
        for want in [
            "cpu",
            "npu-nnapi",
            "npu-coreml",
            "npu-openvino",
            "npu-rknn",
            "npu-qnn",
            "npu-vip9000",
            "npu-webnn",
            "gpu-vulkan",
            "gpu-metal",
            "gpu-cuda",
        ] {
            assert!(ids.contains(&want), "缺少 {want}");
        }
    }
}
