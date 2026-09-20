# 能力拓宽路线：视觉 / NPU / YOLO（2026-09-21）

承接 MVP 接线（`t1gate.rs` + `router.rs` 已落地，15 单测全绿）。
本文档回答「怎么在不重训模型的前提下，把能力从 CLI 拓宽到视觉与 NPU」。

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
