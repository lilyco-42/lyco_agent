# 手机端本地 Agent APK（2026-09-23 立项）

> 需求（用户原话）：**「就做本地手机 app（apk）用 qwen 翻译英文 cli，借鉴我的项目 radxa_monitor gh repo，
> 就做本地手机 app webview 就可以了，作为一个伪装的 agent 服务端」**
>
> 场景：**说「关灯」→ App 接任务 → 去执行 `hw --help` → 找到 `hw led blue off` → 执行，灯灭了。**

## 0. 一句话目标

**Android APK = WebView UI + 本地确定的能力内核 + JSch 到板子执行；
「伪装的服务端」是手机上 127.0.0.1 的本地 HTTP 服务，WebView 以为自己在连远端 agent。**

离线可用、普通人可用、新 CLI 出现零重训 —— 与 lycore 的需求基线一致。

## 1. 借鉴 `radxa-monitor`（已逐行核对，不是转述）

仓库：`lilyco-42/radxa-monitor`，Java，456 行单文件 `MainActivity.java`。

| 已有能力 | 位置 | 我们直接用 |
|---|---|---|
| **SSH 远程命令执行** | `runSsh(host,user,pass,cmd)` L329-356（JSch `ChannelExec`） | ✅ **「关灯」的执行通道就是它** |
| mDNS 自动发现 | `nsdDiscover()` L192（`_http._tcp.` / marker `radxa-cubie-a7a`） | ✅ 找板子，不用手填 IP |
| WebView | L53 / `WebViewClient` | ✅ UI 容器 |
| 手写 UI（无 XML layout） | `mkBtn` / `mkEdit` | ✅ 沿用风格，省一套布局 |
| 凭据持久化 | `SharedPreferences("radxa_pref")` | ✅ 记住板子的 user/pass |
| 默认 IP | `192.168.10.165` | ⚠️ 沿用但必须可覆盖（hostkey 会漂，见用户记忆） |

**构建链（无 Android Studio）**：`aapt2 + javac -encoding UTF-8 + d8 + zipalign + apksigner`，
JSch 0.2.17 合入 dex。README 明写：**d8 需要 build-tools 36.1，34 的 d8 对 JSch 字节码会 NPE**。

### 本机环境核对（实查）

| 项 | 值 |
|---|---|
| `ANDROID_SDK_ROOT` | `D:/android_sdk` ✅ |
| build-tools | 30.0.3 / 34.0.0 / 35.0.0 / **36.1.0** / 37.0.0 ✅ 有 README 要求的那一版 |
| platforms | android-33/34/35/36 ✅ |
| NDK | **25.2.9519653 与 30.0.15729638** ✅ 两套都在 |
| Rust android target | ❌ 未装 → `rustup target add aarch64-linux-android` |
| keystore | `radxa.keystore`（仓库内）✅ 可复用签名 |

## 2. 架构

```text
┌──────────────── APK（Java Activity，沿用 radxa-monitor 风格）────────────────┐
│  WebView UI ──http://127.0.0.1:PORT──┐                                       │
│                                      ▼                                       │
│                        「伪装的服务端」本地 HTTP                              │
│                        ┌───────────────────────────┐                         │
│                        │ ① 抓 help（经 SSH）        │                         │
│                        │ ② parse_help（确定性）     │                         │
│                        │ ③ cli_zh 中文翻译（词典）   │  ← 2026-09-23 已实现     │
│                        │ ④ choices 判定式候选       │                         │
│                        │ ⑤ 【模型只做选编号】        │  ← Qwen3-0.6B（P1）      │
│                        │ ⑥ t1gate + paramcheck      │                         │
│                        │ ⑦ SSH 执行 + exit_code     │                         │
│                        │ ⑧ lbrush 落盘（金标准）     │                         │
│                        └───────────────────────────┘                         │
│      ▲ JNI                                    │ JSch ChannelExec             │
│  liblycore.so（确定性内核）                     ▼                              │
└────────────────────────────────────────  A7A / 局域网任意主机 ───────────────┘
                                            hw led blue off（灯真的灭了）
```

## 3. 三条设计决定（都出自实证，不是偏好）

### 3.1 模型只做「选编号」，**不做翻译**

用户说「用 qwen 翻译英文 CLI」，但全部 3 域 × 5 配置 × 2 运行时的数据指向同一结论：
**0.6B 做信息抽取不可靠**（v17b：docker 掉前缀、cargo 吐 `cargo:build`），
而 Δ_SFT = −43.3pp 说明能力来源不是权重。

所以分工：

| 环节 | 谁做 | 为什么 |
|---|---|---|
| 英文 help → 中文说明 | **本地词典**（`cli_zh`，零模型） | 翻译会成为训练数据；把幻觉写进语料 = v13 失败重演 |
| 「关灯」→ 选哪条候选 | **Qwen3-0.6B**（只回编号） | 判定式 100% vs 生成式 41.7%（Jev 对照） |
| 命令组装 | **Rust 确定性** | 模型不许产出命令串 |

同一把翻译也应 Rigorous 校验：译文里**混排的英文就是没翻的部分**（`coverage` 字段量化），
用户/下游一眼能看出哪句不可信。

### 3.2 Phase 1 **不带模型**

「关灯」这类短指令句式简单，`hw --help` 的候选里直接能命中（`light off`）。
`lycore manual` + `lycore do` 已经在 PC 上证明了这条链路可以零模型跑通。
先交付能用的，再挂模型 —— 而不是先纠结 llama.cpp 的 Android 编译。

### 3.3 执行端在板子，推理端在手机

`hw` 命令是 A7A 上的程序，命令**必须在板子上执行**。手机只负责理解 + 展示。
这也让 radxa-monitor 的 SSH 能力成为核心而非附属。

## 4. 「关灯」的完整时序（验收脚本）

```text
用户："关灯"
 1. SSH: hw --help                    → help 原文（板子上真实存在，不是我们编的）
 2. parse_help                        → N 条候选，其中含 "hw led blue off"
 3. cli_zh 翻译                        → "关闭灯"（coverage 可查）
 4. render_choices                    → 编号候选列表
 5. 【P1】Qwen 回编号 / 【P0】确定性关键词命中 → 选中 hw led blue off
 6. assemble + paramcheck             → 命令完整、不缺参
 7. t1gate::classify                  → light off 判 Write → **需用户确认**（不是自动执行）
 8. 用户点确认 → SSH 执行               → exit_code
 9. lbrush 记录 (nl="关灯", cmd="hw led blue off", exit_code=0, judge=human/model)
```

第 7 步是这个产品的**存在理由**：实测 `reject = 0%`，模型层和 SFT 层都不拒绝，
唯一防线是执行层的确定性闸门。

## 5. 分阶段

| 阶段 | 内容 | 交付判据 |
|---|---|---|
| **P0** | Java APK 骨架 + WebView + SSH + mDNS + 「关灯」硬编码链路走通 | 手机上点一下，A7A 的灯真的灭了 |
| **P1** | JNI 接 `liblycore.so`：确定性内核（help 解析 / 中文翻译 / 判定式 / T1 门）上线 | `hw` 换成任意 CLI（`docker`/`git`）无需改 App |
| **P2** | 接本地 Qwen3-0.6B（llama.cpp Android），只做候选编号判定 | 模糊说法（"把灯关了"/"关一下灯"）也能命中 |
| **P3** | lbrush 采集回流：手机上的成功调用变成训练/评测素材 | ndjson 里有 `exit_code=0` 的真实记录 |

## 6. 交叉编译 —— **已验证可用**（2026-09-24 00:12）

### 产物

```
target/aarch64-linux-android/release/liblycore.so
  303.6 KB · ELF 64-bit little-endian · machine=183 (EM_AARCH64)
```

内含：`help_parse`(2186 行) / `cli_zh` 词典 / `choices` 判定式脊 / `t1gate` 安全门 /
`paramcheck` / rusqlite(bundled SQLite) / reqwest+rustls / tiny_http。**303KB，strip 过。**

### 可用命令（本机实跑，勿改）

```bash
cd /d/Code/lyco_agent/lycore
export NDK_BIN=/d/android_sdk/ndk/30.0.15729638/toolchains/llvm/prebuilt/windows-x86_64/bin
# 🔴 必须 .cmd 后缀！无扩展名的那个是 Unix shell 脚本，Windows 认不了
export CC_aarch64_linux_android="$NDK_BIN/aarch64-linux-android24-clang.cmd"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
export AR_aarch64_linux_android="$NDK_BIN/llvm-ar"
cargo build --release --lib --target aarch64-linux-android
```

### 踩过的坑（Windows 专属，记下来别再踩）

| 坑 | 现象 | 解法 |
|---|---|---|
| linker 用无扩展名的 `clang` | 编 rlib 通过、编 cdylib 报 `%1 不是有效的 Win32 应用程序 (os error 193)` | **改用 `clang.cmd`**。rlib 不链接所以不暴露，cdylib 才暴露 |
| `cargo-zigbuild` + zig 0.16 | cc-rs 找 `wrappers/1438/8571.exe`，磁盘上却是 `zigcc-aarch64-linux-android-8571.bat` | **Windows 上暂不可用**，直接走 NDK |
| `~/.cargo/bin` 找不到 cargo-zigbuild | `CARGO_HOME` 在 `D:\app\scoop\persist\rustup\.cargo`（scoop 装的 rustup） | `which cargo-zigbuild` 定位，别手敲路径 |

### 风险与备选

| 风险 | 备选方案 |
|---|---|
| ~~lycore 交叉编不过 Android~~ | **已解决**（见上） |
| llama.cpp Android 二进制 | 官方 release 的 android-arm64 包；不行则 MNN-LLM / ExecuTorch |
| JSch + build-tools 36.1 | 本机已确认有 36.1.0 ✅ |

## 7. 待用户拍板

1. **目标设备**：你有几台 Android？（作者_ring χωρίς 至少要一台 Android 7+ 的真机才能验 P0）
2. **命令对象**：先做 `hw`（板子的灯）还是先看 `hw --help` 里到底有没有 `light off`？
   这一条决定了 P0 能不能当天跑通 —— **需要先上板子确认 `hw` 的子命令名**。
3. **要不要新建仓库**（建议 `lyco-agent-apk`）还是在 `radxa-monitor` 里开分支？
