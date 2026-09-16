//! npu_runtime — A733 Vivante VIP9000 NPU 串行调度器 (设计 + mock, 板子 halt 不阻塞)
//!
//! 硬约束 (长期记忆): **VIP9000 同一时刻只能跑一个网络**, 多消费者必须自己串行排队。
//! 本模块把"串行"抽象成三方约束:
//!
//!   1. **租约 (Lease)** —— 持租约者独占 NPU; 未持租约调用 `infer`/`load` 一律拒绝。
//!   2. **优先级等待队列** —— 忙时 `enqueue(owner, prio)`, 释放时晋升最高优先级 (同级 FIFO)。
//!   3. **租约超时回收** —— 防止某技能霸占 NPU (sweep/acquire 时惰性回收过期租约)。
//!
//! 板子工作已 halt: 此处**只做纯设计 + `MockNpu` 单测**, 不依赖 `/dev/galcore`。
//! 真实联调走 TIM-VX 用户态 (避开 6.6 galcore hard hang), 届时:
//!   - 实现 `NpuBackend` 的 `FfiNpu` (unsafe FFI) → 可把本模块提为独立 crate `lyco-npu-runtime`
//!     以隔离 unsafe 依赖 (当前置于 lycore 内只为免搭第二套构建)
//!   - `vnn_identify` executor 在推理前 `acquire("vnn_identify", prio)`, 用完 `release`
//!
//! 这样 CPU 侧 (Qwen 类 LLM) 与 NPU 侧 (视觉/语音 encoder) 按 petayyyy 定论「hybrid」
//! 分工: NPU 吃视觉, 空出 CPU 核给 LLM。

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 调度错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NpuError {
    /// NPU 被占用 (当前租约未释放且未过期)
    Busy,
    /// 提供的租约不是当前持有者 (已释放/被回收/伪造)
    NoSuchLease,
    /// 底层后端错误 (加载失败/未加载/驱动异常)
    Backend(String),
}

impl std::fmt::Display for NpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NpuError::Busy => write!(f, "NPU 忙 (已被占用)"),
            NpuError::NoSuchLease => write!(f, "租约无效 (非当前持有者)"),
            NpuError::Backend(e) => write!(f, "NPU 后端错误: {e}"),
        }
    }
}

impl std::error::Error for NpuError {}

/// 租约 —— 对 NPU 的独占使用权凭证
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub id: u64,
    pub owner: String,
    pub priority: u8,
}

/// NPU 后端抽象 —— mock 单测与真实 FFI 实现同一接口
pub trait NpuBackend: Send {
    /// 加载模型 (.nbg / 量化权重)
    fn load(&mut self, model: &[u8]) -> Result<(), NpuError>;
    /// 单次推理 (串行语义由 NpuRuntime 保证)
    fn infer(&mut self, input: &[u8]) -> Result<Vec<u8>, NpuError>;
    /// 卸载模型 (释放 NPU 侧资源)
    fn unload(&mut self);
}

/// 等待者 (优先级队列元素)
#[derive(Debug, Clone)]
struct Waiter {
    id: u64,
    owner: String,
    priority: u8,
}

impl PartialEq for Waiter {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Waiter {}
impl PartialOrd for Waiter {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Waiter {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap 是最大堆: 高优先级先弹出; 同级用较小 id 先弹出 (= FIFO)
        self.priority
            .cmp(&other.priority)
            .then(other.id.cmp(&self.id))
    }
}

struct State {
    holder: Option<Lease>,
    acquired_at: Option<Instant>,
    waiters: BinaryHeap<Waiter>,
    next_id: u64,
    ttl: Duration,
}

/// NPU 串行调度器 (泛型于后端, 便于 mock 单测)
pub struct NpuRuntime<B: NpuBackend> {
    backend: Mutex<B>,
    state: Mutex<State>,
}

impl<B: NpuBackend> NpuRuntime<B> {
    /// 新建调度器; `lease_ttl` 为租约最长占用时长 (超时惰性回收)
    pub fn new(backend: B, lease_ttl: Duration) -> Self {
        Self {
            backend: Mutex::new(backend),
            state: Mutex::new(State {
                holder: None,
                acquired_at: None,
                waiters: BinaryHeap::new(),
                next_id: 1,
                ttl: lease_ttl,
            }),
        }
    }

    /// 惰性回收: 若当前租约已过期则清空 holder, 返回是否发生回收
    fn reclaim(st: &mut State, now: Instant) -> bool {
        if let (Some(_), Some(at)) = (&st.holder, st.acquired_at) {
            if now.saturating_duration_since(at) >= st.ttl {
                st.holder = None;
                st.acquired_at = None;
                return true;
            }
        }
        false
    }

    fn grant(st: &mut State, owner: &str, priority: u8, now: Instant) -> Lease {
        let id = st.next_id;
        st.next_id += 1;
        let lease = Lease {
            id,
            owner: owner.to_string(),
            priority,
        };
        st.holder = Some(lease.clone());
        st.acquired_at = Some(now);
        lease
    }

    /// 申请租约 (非阻塞): 空闲(或旧租约已过期)则授予, 否则 `Busy`
    pub fn acquire(&self, owner: &str, priority: u8) -> Result<Lease, NpuError> {
        self.acquire_at(owner, priority, Instant::now())
    }

    /// 同 `acquire`, 但注入时钟 (确定性测试用)
    pub fn acquire_at(&self, owner: &str, priority: u8, now: Instant) -> Result<Lease, NpuError> {
        let mut st = self.state.lock().unwrap();
        Self::reclaim(&mut st, now);
        if st.holder.is_some() {
            return Err(NpuError::Busy);
        }
        Ok(Self::grant(&mut st, owner, priority, now))
    }

    /// 忙时登记等待者, 返回 ticket (释放时按优先级晋升)
    pub fn enqueue(&self, owner: &str, priority: u8) -> u64 {
        let mut st = self.state.lock().unwrap();
        let id = st.next_id;
        st.next_id += 1;
        st.waiters.push(Waiter {
            id,
            owner: owner.to_string(),
            priority,
        });
        id
    }

    /// 释放租约 (仅当前持有者有效), 并晋升最高优先级等待者 (若有)
    pub fn release(&self, lease: &Lease) -> Result<Option<Lease>, NpuError> {
        let now = Instant::now();
        let mut st = self.state.lock().unwrap();
        match &st.holder {
            Some(h) if h.id == lease.id => {
                st.holder = None;
                st.acquired_at = None;
                match st.waiters.pop() {
                    Some(w) => {
                        let promoted = Lease {
                            id: w.id,
                            owner: w.owner,
                            priority: w.priority,
                        };
                        st.holder = Some(promoted.clone());
                        st.acquired_at = Some(now);
                        Ok(Some(promoted))
                    }
                    None => Ok(None),
                }
            }
            _ => Err(NpuError::NoSuchLease),
        }
    }

    /// 超时扫描: 若持有者租约过期则回收并晋升等待者 (惰性回收的主动触发点)
    pub fn sweep(&self) -> Option<Lease> {
        self.sweep_at(Instant::now())
    }

    /// 同 `sweep`, 但注入时钟 (确定性测试用)
    pub fn sweep_at(&self, now: Instant) -> Option<Lease> {
        let mut st = self.state.lock().unwrap();
        if !Self::reclaim(&mut st, now) {
            return None;
        }
        match st.waiters.pop() {
            Some(w) => {
                let promoted = Lease {
                    id: w.id,
                    owner: w.owner,
                    priority: w.priority,
                };
                st.holder = Some(promoted.clone());
                st.acquired_at = Some(now);
                Some(promoted)
            }
            None => None,
        }
    }

    /// 当前持有者 (供监控/调试)
    pub fn holder(&self) -> Option<Lease> {
        self.state.lock().unwrap().holder.clone()
    }

    /// 等待队列长度
    pub fn pending(&self) -> usize {
        self.state.lock().unwrap().waiters.len()
    }

    fn assert_holder(st: &State, lease: &Lease) -> Result<(), NpuError> {
        match &st.holder {
            Some(h) if h.id == lease.id => Ok(()),
            _ => Err(NpuError::NoSuchLease),
        }
    }

    /// 加载模型 (须持租约)
    pub fn load(&self, lease: &Lease, model: &[u8]) -> Result<(), NpuError> {
        Self::assert_holder(&self.state.lock().unwrap(), lease)?;
        self.backend.lock().unwrap().load(model)
    }

    /// 推理 (须持租约 —— 这是"单网络串行"的强制执行点)
    pub fn infer(&self, lease: &Lease, input: &[u8]) -> Result<Vec<u8>, NpuError> {
        Self::assert_holder(&self.state.lock().unwrap(), lease)?;
        self.backend.lock().unwrap().infer(input)
    }

    /// 卸载模型 (须持租约)
    pub fn unload(&self, lease: &Lease) -> Result<(), NpuError> {
        Self::assert_holder(&self.state.lock().unwrap(), lease)?;
        self.backend.lock().unwrap().unload();
        Ok(())
    }
}

/// 测试/占位后端: 确定性、无硬件依赖
#[derive(Default)]
pub struct MockNpu {
    pub loaded: bool,
    pub infer_calls: usize,
    pub fail_load: bool,
}

impl NpuBackend for MockNpu {
    fn load(&mut self, _model: &[u8]) -> Result<(), NpuError> {
        if self.fail_load {
            return Err(NpuError::Backend("mock load 失败".into()));
        }
        self.loaded = true;
        Ok(())
    }
    fn infer(&mut self, input: &[u8]) -> Result<Vec<u8>, NpuError> {
        if !self.loaded {
            return Err(NpuError::Backend("模型未加载".into()));
        }
        self.infer_calls += 1;
        Ok(input.iter().map(|b| b.wrapping_add(1)).collect())
    }
    fn unload(&mut self) {
        self.loaded = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt() -> NpuRuntime<MockNpu> {
        NpuRuntime::new(MockNpu::default(), Duration::from_secs(60))
    }

    #[test]
    fn single_network_serialization() {
        let r = rt();
        let a = r.acquire("vnn_identify", 5).unwrap();
        // 第二个消费者必须被拒 (VIP9000 单网络)
        assert!(matches!(r.acquire("other", 9), Err(NpuError::Busy)));
        assert_eq!(r.pending(), 0, "Busy 不应吞掉 ticket");
        // 释放后可用
        assert!(r.release(&a).unwrap().is_none());
        assert!(r.acquire("other", 1).is_ok());
    }

    #[test]
    fn priority_promotion_on_release() {
        let r = rt();
        let a = r.acquire("A", 5).unwrap();
        r.enqueue("low", 1);
        r.enqueue("high", 9);
        r.enqueue("mid", 5);
        let p1 = r.release(&a).unwrap().unwrap();
        assert_eq!(p1.owner, "high", "最高优先级先晋升");
        let p2 = r.release(&p1).unwrap().unwrap();
        assert_eq!(p2.owner, "mid");
        let p3 = r.release(&p2).unwrap().unwrap();
        assert_eq!(p3.owner, "low");
        assert!(r.release(&p3).unwrap().is_none());
        assert_eq!(r.holder(), None);
    }

    #[test]
    fn fifo_tie_break_same_priority() {
        let r = rt();
        let a = r.acquire("A", 1).unwrap();
        r.enqueue("first", 3);
        r.enqueue("second", 3);
        let p = r.release(&a).unwrap().unwrap();
        assert_eq!(p.owner, "first", "同优先级 FIFO");
    }

    #[test]
    fn lease_timeout_reclaims() {
        let ttl = Duration::from_secs(10);
        let r = NpuRuntime::new(MockNpu::default(), ttl);
        let t0 = Instant::now();
        let _a = r.acquire_at("A", 1, t0).unwrap();
        // 未过期 → Busy
        assert!(matches!(
            r.acquire_at("B", 1, t0 + Duration::from_secs(5)),
            Err(NpuError::Busy)
        ));
        // 过期 → 惰性回收并授予
        let b = r.acquire_at("B", 1, t0 + Duration::from_secs(11)).unwrap();
        assert_eq!(b.owner, "B");
    }

    #[test]
    fn sweep_reclaims_expired_and_promotes() {
        let ttl = Duration::from_secs(10);
        let r = NpuRuntime::new(MockNpu::default(), ttl);
        let t0 = Instant::now();
        let _a = r.acquire_at("A", 1, t0).unwrap();
        r.enqueue("B", 1);
        // 未过期: 不回收
        assert!(r.sweep_at(t0 + Duration::from_secs(5)).is_none());
        // 过期: 回收 + 晋升 B
        let b = r.sweep_at(t0 + Duration::from_secs(11)).unwrap();
        assert_eq!(b.owner, "B");
        assert_eq!(r.pending(), 0);
    }

    #[test]
    fn infer_requires_current_lease() {
        let r = rt();
        let a = r.acquire("A", 1).unwrap();
        r.load(&a, b"model").unwrap();
        let out = r.infer(&a, b"abc").unwrap();
        assert_eq!(out, vec![b'b', b'c', b'd'], "mock: 每字节 +1");
        // 伪造租约 → 拒绝 (不触碰后端)
        let fake = Lease {
            id: 999,
            owner: "x".into(),
            priority: 1,
        };
        assert!(matches!(r.infer(&fake, b"z"), Err(NpuError::NoSuchLease)));
        // 释放后旧租约失效
        r.release(&a).unwrap();
        assert!(matches!(r.infer(&a, b"z"), Err(NpuError::NoSuchLease)));
    }

    #[test]
    fn release_stale_lease_rejected() {
        let r = rt();
        let a = r.acquire("A", 1).unwrap();
        r.release(&a).unwrap();
        assert!(matches!(r.release(&a), Err(NpuError::NoSuchLease)));
    }

    #[test]
    fn load_failure_surfaces_backend_error() {
        let r = NpuRuntime::new(
            MockNpu {
                fail_load: true,
                ..Default::default()
            },
            Duration::from_secs(60),
        );
        let a = r.acquire("A", 1).unwrap();
        assert!(matches!(r.load(&a, b"m"), Err(NpuError::Backend(_))));
    }
}
