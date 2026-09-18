# A733 NPU — 在当前 6.6 内核上移植（不换镜像）

调研日期 2026-09-14。目标：在 `6.6.98-4-aw2511`（Radxa trixie，Debian 13）上让 VIP9000 跑起来。
**约束：不换内核、不换镜像。**

---

## 一、先纠正根因（与之前的结论不同）

之前记的「6.6 上 NPU hang」是 **galcore/TIM-VX 路线**的现象，而且根因不止一个。逐条拆：

| # | 根因 | 证据 | 状态 |
|---|---|---|---|
| 1 | **当前镜像根本没有 `sunxi_npu` 驱动** | `modinfo sunxi_npu` → not found；`/lib/modules/*/kernel/drivers/` 无 npu 目录；`dmesg \| grep -icE "npu\|vip"` = **0** | VIPLite 官方路线**无源可移** |
| 2 | **内核驱动与用户态版本错配** | 内核用 bsp 的 `6.4.18.6.904649`，而 ai-sdk `unified-tina` 用户态只有 `6.4.15.3.690884`。MaverickLong 明确指出混版会 malfunction | **这就是 hang 的头号嫌疑** |
| 3 | **6.6 内核 API 漂移** | dma-buf 在 6.4+ 改了锁：`mutex_lock(&buf_obj->lock)` → `dma_resv_lock_interruptible(buf_obj->resv)`；头文件 `dma-resv.h` | 有现成补丁 |
| 4 | Clock / Power 域 | `clk_npu` 只 prepare 未 enable；`pd_npu` 保持 off | ✅ 已由 `radxa_utlra/npu_clk_fix.ko` 修好 |

**板端实测铁证**（`radxa_utlra/docs/a733-npu-usable-path.md`）：

```
VIPLite driver software version 2.0.3.2-AW-2024-08-30      ← 用户态库 OK ✓
viphal_os_init[81], fail to open device /dev/vipcore       ← 设备节点缺失 ✗
```

即：**用户态完全就绪，缺的是内核侧设备节点。**

---

## 二、两条路线：只有一条可移

| 路线 | 设备节点 | 源码 | 6.6 可行性 |
|---|---|---|---|
| VIPLite（官方/社区主流） | `/dev/vipcore` | **闭源，不在内核树** | ⛔ 无源可移 |
| galcore / TIM-VX | `/dev/galcore` | ✅ `aw_nna_galcore` 开源 + 6.6 补丁 | ✅ **可行** |

内核树里搜 `vipcore` / `sunxi_npu` 只命中 U-Boot 的寄存器宏（`SUNXI_NPU_BASE 0x03600000`），
没有任何 Linux 驱动源码 —— VIPLite 的内核侧确实是闭源的。

**所以：不换镜像的前提下，只能走 galcore / TIM-VX。**

好消息是 TIM-VX 直接吃 **TFLite / ONNX**，
**不需要 ACUITY 那 11 GB 的主机工具链和 NBG 转换** —— 对我们反而是简化。

---

## 三、移植资源（已核实存在）

- **驱动源码**：`MaverickLong/Radxa-A733-NPU-Unified-Driver-Support-Package`
  `aw_nna_galcore/`（Kconfig, Makefile, hal/, os/, ta/）
  上游同源：`radxa/allwinner-bsp` @ `cubie-aiot-v1.4.6` `drivers/npu/aw_nna_galcore`
- **6.6 补丁**：同仓库 `galcore_6.6_kernel_api_drift.patch`
  用 `LINUX_VERSION_CODE >= KERNEL_VERSION(6,4,0)` 条件编译，多版本兼容
- **一键脚本**：`apply_galcore_6.6_drift.bash` → `build_galcore.bash` → `load_galcore.bash`
- **成功标志**（dmesg）：
  ```
  galcore: enter gckPLATFORM_Init from allwinenertech
  galcore: irq line = 477
  ```
  且 `/dev/vipcore` 被 `/dev/galcore` 取代

---

## 四、必须守住的三条硬约束（踩了就白干）

1. **内核驱动版本必须 = `6.4.15.3.690884`**
   不能用 bsp 里的 `6.4.18.6.904649` —— 用户态只认 6.4.15.3。
   → **先确认 `aw_nna_galcore` 里的版本宏，不是 6.4.15.3 就换源。**
2. **不要 `rmmod` 现有 galcore** —— 会 panic 板子。
   用 `radxa_utlra` 的 `scripts/install-npu-clk-fix.sh`（幂等，且会修 vipcore 抢设备绑定的问题）。
3. **Clock/Power 必须在加载后仍然成立**
   `npu_clk_fix.ko` 是运行时补丁，重启会失效。加载顺序：`clk_fix` → `galcore`。

---

## 五、执行步骤

### 5.0 前置（5 分钟，先确认再动手）

```bash
uname -r                                        # 期望 6.6.98-4-aw2511
ls -l /dev/galcore /dev/vipcore 2>&1            # 记下现状
ls /lib/modules/$(uname -r)/build >/dev/null 2>&1 && echo "headers OK" || echo "缺 headers"
cat /sys/kernel/debug/clk/clk_summary 2>/dev/null | grep -i npu
```

### 5.1 取源码与补丁

```bash
git clone https://github.com/MaverickLong/Radxa-A733-NPU-Unified-Driver-Support-Package ~/npu-galcore
cd ~/npu-galcore
grep -rn "6\.4\.1[58]" aw_nna_galcore/ | head     # ★ 确认版本是 6.4.15.3
```

### 5.2 应用补丁并编译

```bash
bash apply_galcore_6.6_drift.bash
bash build_galcore.bash
```

编译前确保内核 headers 就位（否则先装 `linux-headers-$(uname -r)`）。

### 5.3 加载

```bash
sudo bash load_galcore.bash
dmesg | tail -40
ls -l /dev/galcore
```

### 5.4 Gate

| Gate | 判据 |
|---|---|
| G1 驱动加载 | `dmesg` 出现 `gckPLATFORM_Init ... allwinenertech` + `irq line = 477` |
| G2 用户态握手 | `ai-sdk/unified-tina` 的示例能 init 成功（不是 `-1`） |
| G3 真实计算 | TIM-VX 跑一个卷积网络出结果 —— **必须 G3 才算成**，G2 过而 G3 挂 = 回到老问题 |

⚠️ G3 的判定标准要严格：之前栽在拿 `/proc/interrupts` 计数当证据（`galcore` 的 IRQ 来自
`_SetPower` 电源管理配对，纯 CPU 跑也会涨，**该判据对 galcore 无效**）。

---

## 六、两份补丁的逐条 diff（已做完，2026-09-15）

先确认前提：两份补丁**基准是同一份源码**。blob 前缀逐一对上——

| 文件 | utlra (`patches/galcore-6.6-port.patch`) | MaverickLong (`galcore_6.6_kernel_api_drift.patch`) |
|---|---|---|
| `gc_hal_kernel_os.c` | `448b40a` | `448b40a8` |
| `gc_hal_kernel_driver.c` | `264c91c` | `264c91ce` |
| `…_gfp.c` | `09902c0` | `09902c03` |
| `…_dmabuf.c` | `44fbfac` | `44fbfac0` |
| `…_reserved_mem.c` | `1c92af1` | `1c92af17` |
| `…_user_memory.c` | `4d87117` | `4d871178` |

所以不是"两份不同的驱动"，而是**同一份源码的两种改法**。逐项对比：

| 漂移点 | utlra 9-hunk | MaverickLong | 采用 |
|---|---|---|---|
| Makefile `-Werror` | ✅ 移除 | ❌ 未处理 | **utlra**（6.6 BSP 头文件必然产生新 warning，`-Werror` 会直接打断构建） |
| dma-buf 锁（6.4+） | `#if < 6.6.0` **直接跳过加锁** | `dma_resv_lock_interruptible(buf_obj->resv, NULL)` | **MaverickLong**（语义正确；注意两处都在 `dma_buf_info_show` 这个 debugfs 路径，**不是计算热路径，与 hang 无关**） |
| `class_create()`（6.4+ 少一个参数） | ✅ | ✅ | 相同 |
| `vma->vm_flags \|=` → `vm_flags_set()`（6.3+） | ✅ | ✅ | 相同 |
| `__GFP_ATOMIC`（Allwinner 6.6 BSP 头里没有） | `#ifndef → #define 0` | `>= 5.18` 直接不用 | MaverickLong（更简洁，等价） |
| `pin_user_pages()` 6.5 起去掉 vmas 参数 | ✅ | ✅ | 相同 |
| `virt_addr_valid()` 需强转 | ✅ | ✅ | 相同 |
| `_QuerySignal` 声明/定义类型冲突 | ❌ 未处理 | ✅ `gctBOOL` → `gceSTATUS` | **MaverickLong（必须有）** |
| 6.5+ `__pte_offset_map_lock` 不导出 | 手写 `_galcore_pte_offset_map_lock`（保留原 PTE 遍历） | 改走 `follow_pfn`（`gcdUSING_PFN_FOLLOW=1`） | **utlra（见下，这是唯一真正的功能性分歧）** |

### 6.1 关键分歧：`follow_pfn` 是错的

`os.c` 里 `_QuerySignal` 的声明是 `gctBOOL`、定义却是 `gceSTATUS`，而
`gc_hal_kernel_os.c:56` 又 `#include "gc_hal_kernel_linux.h"` —— 这是 **C 语言的硬编译错误**
（`conflicting types`），不是 warning，去掉 `-Werror` 也救不了。所以 MaverickLong 那个
头文件修复**必须带上**，utlra 那份单独用是编不过的。

真正值得争的是页表遍历。实测 6.6 内核源码（`mm/memory.c:5622`）：

```c
int follow_pfn(struct vm_area_struct *vma, unsigned long address, unsigned long *pfn)
{
	if (!(vma->vm_flags & (VM_IO | VM_PFNMAP)))
		return ret;          // ← -EINVAL
	...
}
EXPORT_SYMBOL(follow_pfn);
```

**`follow_pfn` 对没有 `VM_IO|VM_PFNMAP` 的 VMA 一律返回 `-EINVAL`** —— 也就是**所有普通
匿名/文件映射的用户缓冲区**。而 `gcdUSING_PFN_FOLLOW=1` 影响的正是
`import_pfn_map()`（导入用户态 tensor）和 `_QueryProcessPageTable()`（用户地址转物理地址）。
TIM-VX 在用户态 malloc 出来的输入张量走的就是这条路 —— **一切用户缓冲区导入都会失败**。

顺带一提，那条路径还有个 bug：`find_vma()` 返回 NULL 时直接 `return -ENOTTY`，
在 `PFN_FOLLOW=1` 分支下 `current->mm->mmap_sem` **没释放就返回了**，
之后该进程任何 mmap/缺页都会死锁 —— 这本身就是"NPU hang"的一种可能来源。

符号可用性也已查实：
- `pte_offset_map_lock` / `__pte_offset_map_lock`：6.6 **未导出**（`mm/pgtable-generic.c` 无任何 `EXPORT_SYMBOL`）→ 模块确实用不了，必须手写
- `pte_offset_kernel`：`include/linux/pgtable.h` 的 `static inline`（`pmd_page_vaddr + pte_index`）✅
- `pte_lockptr`：`include/linux/mm.h` 的 `static inline` ✅

### 6.2 合成补丁 = MaverickLong 打底 + utlra 两处纠正

产物：`D:\Code\radxa\galcore_6.6.98-aw2511_merged.patch`（8 文件，+109/-5），
配套完整源码树 `D:\Code\radxa\aw_nna_galcore-6.6.98-aw2511\`（4.6 MB，可直接 scp 上板）。

= MaverickLong 全部 7 个文件的漂移修复
**+** `Makefile` 移除 `-Werror`（utlra）
**+** `gcdUSING_PFN_FOLLOW` 保持 `0`，两处调用改为手写
    `_galcore_pte_offset_map_lock()`（utlra）
**+** 手写 helper 内补 `rcu_read_lock()` —— 因为解锁侧 `pte_unmap_unlock()`
    在非 `CONFIG_HIGHPTE`（arm64 就是）下等于 `rcu_read_unlock()`，不配对会破坏 RCU 计数
**+** `import_pfn_map()` 的 `!vma` 分支补 `up_read()`，堵住 mmap_sem 泄漏

> 若构建报 `pte_offset_kernel` 隐式声明，说明该 .c 的 include 顺序没带到 pgtable.h，
> 把 helper 挪到 `#include "gc_hal_kernel_allocator.h"` 之后即可。

### 6.3 另一道此前漏掉的门槛：glibc

MaverickLong 的 README 写着「官方 Radxa OS 上跑不了 Unified SDK，除非有更低 glibc 目标的
用户态驱动」。这条一直没量化。我把 `ZIFENG278/ai-sdk` 的三套用户态库做了 ELF `verneed`
静态解析（无需上板）：

| 目录 | 最高 GLIBC 需求 | 额外依赖 |
|---|---|---|
| `aarch64-none-linux-gnu` | GLIBC_2.33（来自 `libGAL.so`） | 无 |
| `glibc-gcc10_2_0` | GLIBC_2.33 | `libgcc_s.so.1` |
| `glibc-gcc13_2_0` | GLIBC_2.33 | `libgcc_s.so.1` |

**结论：三套都要求 glibc ≥ 2.33。** Debian 11 bullseye（2.31）出局；
Debian 12 bookworm（2.36）/ 13 trixie（2.41）没问题。
`6.6.98-4-aw2511` 是 trixie 分支，所以这道门槛**大概率已经跨过去了** —— 但上板第一件事
仍应是 `ldd --version` 确认。脚本 `npu_galcore_6.6_build.sh preflight` 会自动查。

三套库 MD5 各不相同（真·不同构建）。选哪套用 `sdk` 子命令实测 `ldd | grep 'not found'`：
优先 `glibc-gcc13_2_0`（trixie）/ `glibc-gcc10_2_0`（bookworm）；
`aarch64-none-linux-gnu` 不依赖 libgcc_s，是最自包含的兜底选项。

---

## 七、TIM-VX 的已知限制（影响 lyco_agent 的选型）

来自 MaverickLong 实测（A7Z，同 A733）：

- 任何矩阵乘类算子（卷积、全连接）**必须显式把操作数转成 UINT8**，否则图编译失败
- **通用 batched matmul 不work**：`tim::vx::MatMul` 不可用，要用全连接层模拟

→ 对 lyco_agent 的影响：
- ✅ `vnn_identify`（CNN 分类）：可行
- ✅ `rembg`（u2net）：卷积为主，理论可行
- ⚠️ embedding 检索（EmbeddingGemma）：Transformer 里 MatMul 密集，**风险高**，
  若 MatMul 受限则需退回 CPU 或改走 GPU

---

## 八、风险与回退

| 风险 | 处置 |
|---|---|
| 编译不过 | 先查 headers；`LINUX_VERSION_CODE` 分支是否覆盖 6.6.98 |
| 加载 panic | 不要 `rmmod`；重启后重来，必要时拔电 |
| G3 仍 hang | 说明不只是版本/API 问题，回到 radxa_utlra 的 Layer 3，需 diff `orange-pi-6.6-sun60iw2` 分支 |
| 与 VIPLite 冲突 | 我们镜像上 VIPLite 驱动本就不存在，暂无冲突；将来装了要串行排队 |

---

## 九、待办

- [x] 核对 `aw_nna_galcore` 版本宏 = `6.4.15.3.690884` ✅（`hal/inc/gc_hal_version.h:67`）
- [x] diff 9-hunk port vs `galcore_6.6_kernel_api_drift.patch` ✅（见 §六）
- [x] 合成补丁 + 源码树产出 ✅
- [x] 量化 glibc 门槛 ✅（≥ 2.33）
- [ ] 5.0 前置输出贴回（板子目前不可达，ping 100% 丢包，22 端口不通）
- [ ] `bash npu_galcore_6.6_build.sh preflight` → 确认内核分支 / glibc / headers
- [ ] G1（构建 + `/dev/galcore` 出现 + dmesg 版本串一致）
- [ ] G2（TIM-VX 与用户态握手）
- [ ] **G3 真正跑通一次计算 —— 只有这个算过**
