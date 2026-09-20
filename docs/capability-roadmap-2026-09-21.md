# 能力拓宽路线：视觉 / NPU / YOLO（2026-09-21）

承接 MVP 接线（`t1gate.rs` + `router.rs` 已落地，15 单测全绿）。
本文档回答「怎么在不重训模型的前提下，把能力从 CLI 拓宽到视觉与 NPU」。

---

## 0. 前置产品定型（同日实测追加，先读这条）

**两个新事实，同时改变本路线的定位。**

### 0.1 快脑定版：自家 router_v13（横向对比实测）

| 维度 | 基线 Qwen3-0.6B-Q4 | **自家 router_v13** |
|---|---|---|
| 速度（780M Vulkan） | 98 TPS | 77–90 TPS（同档） |
| 体积 | 397MB | 484MB（+22%） |
| NL→CLI | 通用弱 | `列出没关的issue→gh issue list` ✓ / `空调26度→hw air 26` ✓ |
| License | Apache-2.0 | 干净 |

→ **同速度档下多了真 tool-call 能力，选自家 v13 作快脑。**

### 0.2 通用聊天砍掉；本产品不做闲聊

- 实测两坑：①`thinking` 必须经 `chat_template_kwargs` 关（否则空输出）；
  ②**安全拒绝 0%**——模型层不管拒绝。
- 产品决策：**小白机上只留选择题**（能力清单式引导，候选命令模型给、用户挑）。
- 代码落点：`agent.rs::OUT_OF_SCOPE_REPLY`（越域坦白回复，锁在单测里）。

### 0.3 ⚠️ 本次最重要的实测：训练是净损害（Δ_SFT = −43.3pp）

补跑了悬空已久的 **未微调底座基线**（唯一变量 = 是否 SFT，协议逐字相同）：

| 臂 | 跨域 acc_exec 均值 |
|---|---|
| **base 0.6B（零样本 + schema 注入）** | **85.0%** |
| v13（同底座 + 4000 条 SFT） | 41.7% |
| v16 / v12 / v14 / v15 | 38.3 / 21.7 / 8.3 / 3.3% |

**没有任何一个训练臂超过未训练底座。** 详见
`research-base-model-capability-2026-09-21.md` §4.5。

**这对本路线意味着什么（关键）**：
- 上一段结论「**加能力不动模型，只动子命令 + 能力表**」——
  **不但依然成立，而且被这条数据从"规避教训"升格为"唯一正确路线"**：
  既然训练本身在减分，那么「靠微调把新能力塞进权重」这条路在现阶段是**负 ROI**。
- 但反过来也**别把 base 85% 当成免费午餐**：它完全依赖 schema 注入把命令清单放进上下文
  （plain 无 schema 时 base 也是 0–8%）。**我们的产品护城河不在权重，在「注入协议 + 能力表 + T1 门」。**
- 因此视觉/NPU 的落点不变（本节以下内容全部有效），只是**优先级和理由更新了**：
  它们是「不占 GPU、不依赖训练」的纯增量，正好是当前唯一健康的拓宽方式。

---

## 核心结论：新能力 = 新 CLI 子命令，不进 router 训练

v13 router 只认 5 个域（`hw/gh/ff/lb/brush`），但 **`brush` 是通用 shell 出口**——
任何可执行程序都能经由它调起。于是：

```text
用户: 看看这张截图里有几个人
  → v13: brush lycore yolo detect x.png
  → T1 门: 推理类判只读 → 放行
  → 能力层: YOLO 推理
```

**加能力不动模型，只动「子命令 + 能力表」**。这规避了 v15/v16 的血泪教训：
异构能力直接混进训练数据会互相稀释（lb schema 净伤害 bash，gh 真实对稀释 ff_A −15.2）。

## 三块能力的现状与落点

| 能力 | 现状 | 落点 | 依赖 |
|---|---|---|---|
| **vision** 识图 | `vnn.rs` 已实现：ffmpeg rawvideo → 灰度 64×64 → 规则签名，可判 终端/GUI/自然/文档；真实 CNN 专家位留空（`needs_training`，诚实不伪装） | `lycore vision identify <img>` → `{kind, conf}` | 零新增依赖（ffmpeg 已有） |
| **yolo** 目标检测 | 无 | `lycore yolo detect --image <p> [--backend npu\|cpu]` → `[{cls, conf, box}]` | ONNX→NBG（ACUITY Docker 11GB）或 TIM-VX 直吃 ONNX |
| **npu** 板端推理 | `npu_runtime.rs` 已实现租约 + 优先级队列 + 超时回收，纯设计 + `MockNpu` 单测（板子 halt 不阻塞）；约束 VIP9000 同时只跑一个网络 | `lycore npu lease/status/load/infer` | `/dev/galcore`（板端）；`a7a-npu-cli` 可直接作为外部命令域接入 |

### 调用举例

```bash
# 识图（现有能力直接暴露）
lycore vision identify shot.png
# → {"kind": "terminal", "conf": 0.87, "needs_training": false}

# 目标检测（NPU 优先，CPU 兜底）
lycore yolo detect --image frame.jpg --backend npu
# → [{"cls": "person", "conf": 0.92, "box": [12, 34, 120, 300]}]

# NPU 租约（串行硬约束的显式表达）
lycore npu lease --owner vision --ttl 30   # 持租约者独占推理
```

## 借鉴 dsh（DeepSeek Harness）：插件协议对齐

`awesome-dsh-plugin` 的立场是 **everything is a plugin**——models、tools、sandboxes、
session storage、UI、甚至 agent loop 本身都是可替换插件。

lyco 已有同构物：**`skill.rs` 的技能描述符 `{capabilities, risk, verifier, executor}`**。
它就是 lyco 的插件协议，且比 dsh 多两样 dsh 没有的东西：

- **`risk`** —— 危险等级直接进描述符，T1 门读它，不需要模型判断（reject 0% 实证下这是唯一防线）
- **`verifier`** —— 每个能力自带验证器（mpkg 的「验证 > 参数量」同构）

对齐后的能力注册契约：

```rust
Skill {
  capabilities: ["vision.identify"],
  risk: Risk::Read,          // 推理类 → 自动放行
  verifier: Some(verify_kind_conf),
  executor: Executor::Cli("lycore vision identify {image}"),
}
```

能力进表后全链路自动打通：router 出命令 → T1 门读 `risk` 判定 → 执行 → `verifier` 验结果。
**新增能力只需要填这一个描述符。**

## 排序建议

1. **vision**（最成熟，`vnn.rs` 已能跑，当天可交付）
2. **npu**（`npu_runtime.rs` 租约逻辑已写完，只差真板子联调；可先用 `a7a-npu-cli` 外部命令打通链路）
3. **yolo**（依赖最重：ONNX→NBG 转换链，CPU 后端可先占位）

## 与 MVP 验收的关系

拓宽不影响 MVP 验收口径（5 句会域办成 + 3 句出域泛化 exec + 2 句危险拦截）。
新能力属于 `brush` 域的下游，T1 门对推理类一律判 `Read`（只读 → 自动放行），
与用户的「查询类大胆给最佳命令」一致。
