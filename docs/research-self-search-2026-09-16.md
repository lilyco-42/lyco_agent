# 研究：agent 自主检索（「懂得自己去搜」）+ 检索结果学习进知识包

> 日期：2026-09-16 | 触发：用户必须性指令 *"…**懂得自己去搜** paper minecraft 知道学习怎么启动"*
> 关联：`docs/research-architecture-2026-09-16.md`（架构总纲）、`lycore/src/project.rs`（项目域能力链）
> 结论先行：**补上了指令链的唯一断点**；实现为 `lycore/src/search.rs`（可插换后端 + 离线可测），
> 且**刻意不进 CHAT_TOOLS 常驻工具集**（隐私卖点 + 现模型 7 工具契约 + 需重训），见第三节。

---

## 一、问题：指令链的断点

用户指令 `写一个 8:00-22:00 自动启动 java 起 Minecraft 的程序` 隐含 5 环，现状对照：

| 环节 | 现状 |
|---|---|
| 学 java CLI 参数 | ✅ `learn_cli::index_and_build("java", pack)` |
| 扫目录认得 `paper.jar` | ✅ `project::scan`（commit 86eb7ea） |
| 关联 Minecraft 知识 | ✅ `project::knowledge_cues`（内置领域知识入库） |
| 生成时间窗启动脚本 | ✅ `project::launch_script` |
| **懂得自己去搜 paper minecraft** | ❌ **断点** —— 7 工具（`CHAT_TOOLS`）里**零检索能力** |

`project::MINECRAFT_KNOWLEDGE` 是**内置硬编码**的领域知识。它能覆盖 Minecraft 是因为我们
预先写死了；换成任意长尾领域（某小众软件/某企业内网工具）就无知识可用 —— 这正是必须
补上「自主检索」的原因：**让 agent 自己把未知领域补进知识包**。

---

## 二、设计：`lycore/src/search.rs`

```
query ──▶ SearchBackend ──▶ SearchResult[] ──▶ to_cues ──▶ LVK 知识包(prefix="search")
                                                              └─▶ 此后 lyv_knowledge 可答
```

- **`SearchBackend` trait**（可插换）：
  - `SearxngBackend { base }` —— 自建 SearXNG，`GET {base}/search?q=&format=json`；
    隐私可控（不把 query 交给大厂），可跑在用户自有 lain42 服务器上。
  - `StubSearch` —— 离线确定性占位，**不触网**，供单测与无网环境（默认）。
- **`parse_searxng(body, k)`** —— 纯函数解析（丢空 url、trim、限 k），**可离线单测**。
- **`to_cues(query, results)`** —— 每条带 **来源 URL**（`(来源: …)`），为后续验证/溯源留钩子。
- **`search_and_learn(pack_dir, query, backend, k)`** —— 端到端；幂等（`append_cues` 按 prefix 先删后插）。
- **CLI**：`lycore search --query <...> [--pack <dir>] [--url <searxng>] [--k 5]`
  （无 `--url` → 离线占位并打印提示，保证零网可用）。

**复用而非新造**：入库走 `learn_cli::append_cues`（P1 已建，同一 FTS5 表、幂等、不覆盖他人条目），
与 `project` 模块的知识注入同一通路 —— 一处能力，两处复用。

### 验证器（Verifier 解耦原则的延伸）
检索结果天然不可信（可能过时/错）。按 §1.3「Verifier 与模型解耦」：
- **一阶（已实现）**：保留来源 URL，答案可溯源；
- **二阶（待做）**：`fetch(url)` → 内容哈希/时间戳 → 与本地知识冲突时标记 `stale`；
  这正是 `verify.rs` `VerifierId` 可再添一个 `SearchSource` 变体的位置。

---

## 三、关键取舍：为什么**先不进** CHAT_TOOLS（诚实记录）

诱人的做法是把 `web_search` 加成**第 8 个技能**，让模型自己决定何时搜。**暂不这么做**，理由：

1. **隐私卖点冲突**：lyco 差异化是「本地隐私 + 视频知识」。把联网检索塞进常驻工具集，
   等于默认把用户 query 送出网络 —— 与卖点矛盾。检索应是**显式、opt-in** 的摄取动作。
2. **破坏现模型契约**：现 FC 模型（`qwen3_lyco_fc_corpus`，A10 实测 88%）是 **7 工具**训练产物。
   改 `CHAT_TOOLS` 会立刻造成推理期 schema 与权重不匹配。
3. **单一真源要求三处同改**：`capability::TOOL_CAPS`、`skill::ALL_SKILLS`、`llamacpp::CHAT_TOOLS`
   必须同步 + `executor::execute` 加分支 + 更新 `tests/executor_test.rs`（断言 7 工具）。

### 晋升为第 8 技能的路径（待用户拍板，与 DESIGN issue② 同一决策点）
```
1. capability.rs: TOOL_CAPS += ("web_search", &[Capability::Network])
2. skill.rs:      ALL_SKILLS += Skill{ web_search, risk=Medium, verifier=SearchSource, executor=web_search }
3. llamacpp.rs:   CHAT_TOOLS += web_search 条目 (参数: query, k)
4. executor.rs:   加 "web_search" 分支 → search::search_and_learn
5. tests:         7 → 8 (executor_test / capability / skill 三处断言)
6. 重训 FC:       fc_grpo_corpus.py 语料加"该搜"样本; 评测 BFCL 式检索触发准确率
```
> 建议：**默认关闭，配置开启**（`LYCO_SEARCH_URL` 存在才注册该技能），兼顾隐私与能力。

---

## 四、与 escalation（DESIGN issue②）的关系

检索是 escalation 的**廉价前置**。三级成本阶梯：

```
本地知识包 (lyv_knowledge, ~ms)  →  自主检索 (search, ~100ms, 免费)  →  云端前沿模型 (escalation, $)
      命中即答                            补齐长尾知识                        越权/超难才升级
```
先把长尾用检索补进知识包，能**显著压低**升级到大模型的频率 —— escalation 的触发条件
（Capability 越权 / 知识包 NO_HIT）中，「NO_HIT」这一支可先由 search 兜住。

---

## 五、任务清单

1. ✅ **`search.rs`**：`SearchBackend`(Searxng/Stub) + `parse_searxng` + `to_cues` + `search_and_learn`（本次）。
2. ✅ **CLI `lycore search`**（本次，离线默认）。
3. ⏳ **SearXNG 落地**：在 lain42 起一个 SearXNG（docker），`LYCO_SEARCH_URL` 指向它 → 真联网检索。
4. ⏳ **`SearchSource` 验证器**：`fetch + 内容哈希`，接 `verify.rs` 注册表。
5. ⏳ **晋升第 8 技能**（第三节路径，需用户拍板 + 重训）。
6. ⏳ **检索缓存**：同一 query 结果落盘 TTL，避免重复触网（隐私+成本）。

---

## 六、验证记录（CloudStudio A10，spaceKey `b0d0f5fb…`）

- `search.rs` 6 条单测（解析/限 k/丢空 url/trim/幂等/FTS 落库）—— 全过。
- CLI 离线可跑：`lycore search --query "paper minecraft 怎么启动" --pack <pack>` →
  占位结果 → `to_cues` → 入库 2 条 `search.*` → 此后 `lyv_knowledge` 可答。
- 全量 `cargo test`：见 commit 记录（lib 75→81 tests 量级）。
