# 全项目 ROI 评估（127 仓库）— 2026-09-16

> 方法：`gh api /user/repos --paginate` 拉全（含私有，共 **127** 个）。按 `lyco` 的 WEIGH：
> **ROI = 价值(需求价值·补缺·可复用) ÷ 投入(工时·风险·验证)**；打分为 1-5，**成本高分=便宜**（沿用 `lystack` 表约定）。
> 门槛：价值≥4 且 成本≥4 → **P0 继续**；价值≥3 → **P1 收敛**；价值≤2 → **P3 冻结**。

## 一、按桶评估

| 桶 | 代表仓库（数量） | 价值 | 成本 | 护城河 | ROI 判定 |
|---|---|---|---|---|---|
| **mpkg 记忆资产层** | `mpkg`(spec) `mpkg-registry` `cache-node`（3） | 5 | 4 | **5** | **P0 继续** — 已完成大半（verify/attestation/市场前脸上线） |
| **lilyco 能力总线** | `lilyco` `lly` `lf` `dsh-tool-brush` `dsh-tool-nu`（5） | 5 | 4 | 3 | **P0 继续** — CLI 优先的执行底座，已可跑 |
| **lyco_agent（视频/CLI→mpkg 管线）** | `lyco_agent` `lyco-whisper`（2） | 5 | 3 | **5** | **P0 收敛** — 只做「生产/消费 mpkg」，砍掉重复建设 |
| **DSH / 自研 harness** | `deepseek-harness` `zerostack-launchpad` `lyco_chat` `bitnet-rs`（4+） | 4 | 2 | 3 | **P1 收敛** — 与"采纳 zerostack"路线冲突，二选一；BitNet 留端侧 |
| **游戏/内容产品（营收）** | `zombie-raid` `cute-pet` `meowcat_app` `rembg-ui` `character-theme-kit` `mirage-tank` `buried-treasure-finder` `letsgal-ai` `acgn-studio`（9+） | 4 | 3 | 2 | **P1 择一主推** — 8 个并行=0 个能变现 |
| **Minecraft / Radxa 工具群** | `minecraft` `mc-server-controller` `mc-webview-shell` `radxa-monitor` `radxa_utlra` `awesome-radxa-a733`（6+） | 3 | 3 | 2 | **P2 合并** — 收成 1 个「Radxa 工具箱」 |
| **基建 / 服务** | `lain42` `lilyco-42.github.io` `proxy-panel` `ghboost`（4） | 3 | 4 | 2 | **P2 维持** — lain42 是**信令/结算宿主**，必须活着 |
| **UI / WebView 框架群** | `Lazy-UI` `webview-mini` `webview-capi` `EUI-NEO` `mc-webview-shell`（5） | 2 | 2 | 2 | **P3 冻结** — 与主线无关，投入最重 |
| **学习 / 模板 / 技能** | `study_rust` `clings` `C` `java-learning` `clap_todo` `minigrep-cn` `student_managent` `rust-android-template` `fabric-mod-template` `lyco-skill`…（20+） | 2 | 5 | 1 | **P3 冻结**（模板类留作复用资产） |
| **归档 / 私有备份** | `weq-archive` `soka` `Linux-android-arm64` `WSL2-Linux-Kernel` `xmake-repo` `system-reinstall-backup` `imgui`…（20+） | 1 | 5 | 0 | **P3 不动** — 零成本放着，不投精力 |

## 二、结论（三条）

1. **最大的问题不是缺项目，而是主线被稀释**：127 个仓库里，真正支撑"**护城河 = 语料与验证**"的只有 **~6 个**（`mpkg` / `mpkg-registry` / `cache-node` / `lilyco` / `lly` / `lyco_agent`）。**其余 90% 不产生复利。**
2. **P0 三桶应合并成一条最短闭环**：`lilyco`(造 CLI) → `lyco_agent`(从视频/CLI 造 atoms+steps) → `mpkg`(打包) → `cache-node verify`(回放产出 attestation) → `mpkg-registry`(市场)。**这是唯一有护城河且已具备全部零件的链路。**
3. **两处必须做减法**：
   - `deepseek-harness`(自研 harness) **vs** 采纳的 `zerostack` —— 二选一，别并跑；
   - 游戏产品 9 个 → **只主推 1 个**（其余冻结）。

## 三、下一步（单个动作）

**把 P0 闭环的第一个最小样例跑通**：`lyco_agent` 用一段真实短视频产出 **1 个 `.mpkg`**（atoms 带 `video:<id>#t=` ref），
再用 **`cache-node verify` 回放**产出 attestation，上架 `mpkg-registry`。**这一步同时验证四件套是否真的咬合。**
