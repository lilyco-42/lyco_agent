# lycore 拆分方案（lyco_agent 审查报告 P3 项，2026-09-20）

> 现状：`lycore/` 单 crate，src 24 模块 + 1 bin ≈ 7863 行，**零 feature gate**。
> 端侧部署（Radxa aarch64 / musl 静态）被迫编译全部功能；CI 全功能编译拖慢反馈。
> 本方案是**设计文档，不动代码**；执行时按「先 features 后拆 crate」两步走，每步独立可验收。

## 1. 目标

1. 端侧最小二进制：只编 tokens/pack/search/executor 主链，VNN/学习/NPU/HTTP 可裁剪。
2. 编译时间分层：改 learn.rs 不重编 agent 主链。
3. 依赖收敛：reqwest+rustls+ring（LLM/网络）只在需要它的 crate 出现。

## 2. 分层设计（workspace 内多 crate，非独立仓库）

```text
lycore/
├── Cargo.toml                # [workspace] members, resolver = "2"
├── crates/
│   ├── core/     (~1500 行)  tokens, pack, search, trace, capability,
│   │                         project, skill, rewrite, datagen
│   │                         依赖: serde, serde_json, anyhow  ← 零网络零 sqlite
│   ├── vnn/      (~721 行)   vnn.rs, vnn_cnn.rs
│   │                         依赖: core; feature "vnn" 门权重加载
│   ├── learn/    (~1852 行)  learn.rs, lernen.rs, learn_cli.rs, toolrag.rs
│   │                         依赖: core (+rusqlite 若 learn 落库)
│   ├── agent/    (~2558 行)  executor.rs, agent.rs, tools_runtime.rs,
│   │                         verify.rs, npu_runtime.rs, llamacpp.rs
│   │                         依赖: core, vnn; 网络依赖集中在 llamacpp
│   └── cli/      (~703 行)   serve.rs, bin/lycore.rs
│                             依赖: 全部 + tiny_http + reqwest
└── src/lib.rs → 兼容壳: pub use 重导出, 老路径 import 不破（过渡期保留）
```

依赖方向严格单向：`cli → agent → {learn, vnn} → core`。
`llamacpp.rs`（134 行）是 reqwest 唯一用户 → `agent` crate 的 feature `llm` 包住。

## 3. feature 清单（第一步先行，拆 crate 前就生效）

| feature | 包住 | 默认 | 说明 |
|---|---|---|---|
| `vnn` | vnn.rs + vnn_cnn.rs + 权重解析 | ✅ | 关掉则 identify 走规则回退 |
| `learn` | learn/lernen/learn_cli/toolrag | ✅ | 学习回流闭环 |
| `npu` | npu_runtime.rs | ❌ | 仅 Radxa 板端开 |
| `llm` | llamacpp.rs + reqwest+rustls | ✅ | 关掉可去掉整条 TLS 依赖链 |
| `serve` | serve.rs + tiny_http | ✅ | MCP/HTTP 面 |

端侧发布配置：`--no-default-features --features "vnn,llm"`。
CI 矩阵加一列 `--no-default-features`（core+executor 主链必须独立编译通过）。

## 4. Cargo.lock 风险（本仓特有，照既有流程执行）

- workspace 化 + `resolver = "2"` 会触发一次全量重解析 → **预期 lock 大改一次**，
  按老流程：`git checkout -- Cargo.lock` 后手工按 CRLF 插入新成员条目，勿让 cargo 重写。
- path 依赖不新增外部 package，lock 理论上只加 5 行 `[[package]]` 条目 ——
  若 diff 出现 aws-lc-rs / bitnet-rs 等，立即停手检查（那是 feature 解析漂移的信号）。
- `cargo build --locked` 在本仓本来就失败（bitnet-rs git 依赖），不作为验收标准。

## 5. 执行顺序与验收

1. **Step A（features）**：Cargo.toml 定义上表 feature + `#[cfg(feature)]` 门 →
   验收：`cargo check --no-default-features` ✅、`cargo test --release` 全绿、lock 无漂移。
2. **Step B（拆 crate）**：git mv 模块到 crates/，lib.rs 变兼容壳 →
   验收：`cargo test --workspace` 全绿；musl 静态构建体积对比（预期 ↓15-25%）。
3. **Step C（可选）**：版本化文件名清理（fc_train_v2/v3/v4 等）在 tools/ 重组中已保留，
   约定：新版本直接改同一文件，版本归 git 历史，不再新增 `_v2` 文件名。

## 6. 不拆的理由记录（防过度工程）

- 总量 7863 行 < 1 万行，单仓多 crate 已获得全部编译隔离收益；
  独立仓库/独立版本号是负收益（AGENTS.md 的跨仓协作成本 + lock 三倍漂移面）。
- `pack.rs`/`search.rs` 是检索主链热点，learn 依赖它们但反向不依赖 ——
  这条单向边是分层成立的前提，Step A 前先用 `cargo modules` 或 grep 确认无环。
