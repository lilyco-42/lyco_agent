# P0 MVP：mpkg 全链闭环跑通（2026-09-16）

> 目标：验证 `lilyco → lyco_agent → mpkg → cache-node → mpkg-registry` 四件套**是否真咬合**。
> 方法：`lyco` 信条 3/5 —— **adopt 官方工具链，不自造格式**；最小化验证一个真实闭环。
> 平台：CloudStudio A10（含 ffmpeg/tesseract/whisper.cpp/cache-node 二进制）+ 本机（gh 凭据用于 publish）。

## 一、结论：**五环全部跑通，双实现互证，已上架且可检索安装**

```
① 视频            /workspace/demo_tts4.mp4（20s，TTS 旁白，唯一有音轨的素材）
      ↓ 官方 kbv-distill.sh（六原子，全 CLI 进程）
② kbv.json        4 slices / 17 topic tokens
   ASR 原文: "First, cargo new hello_world 建立项目 → Then cd hello_world → 最後 cargo run 輸出 hello world"
      ↓ 建包（atoms 带 video ref + steps=真 CLI + verify）
③ .mpkg           cargo-hello-world-from-video-0.1.0-0139430f4945.mpkg（**1594 B**）
                  id = sha256:0139430f49459b629fa1e2323f622b42481d076b22e81256612cd779b1cdb311
      ↓ 回放验证（**双实现互证**）
④ Python `mpkg.py verify` → **ok=True**（steps=2, verify=2）
   Rust  `cache-node verify` → **ok=True**（同一 id）
      ↓ 上架
⑤ registry        packages/cargo-hello-world-from-video-0.1.0-0139430f4945.mpkg
      ↓ 消费侧
   `mpkg.py search cargo` → **1 hit**（id + intent 命中）
   `mpkg.py install cargo-hello-world-from-video` → **installed**
```

**包内容（真知识，来自视频）**：
- `intent`: 从视频学会用 cargo 新建 Rust 项目并运行输出 hello world
- `atoms[]`: 4 个，`ref = sha256:e69f32ea...#t=t0,t1`（**视频寻址，即 mpkg spec 预留的 `video:` 钩子**）
- `steps[]`: `cargo new hello_world` → `cd hello_world && cargo run`
- `verify[]`: `test -f hello_world/Cargo.toml` + `cargo run \| grep -q "Hello, world!"`

## 二、过程中踩的坑（都是真问题，已解决）

| 坑 | 现象 | 解法 |
|---|---|---|
| 素材无音轨 | `ffmpeg -vn audio.wav` → *"Output file does not contain any stream"* | 换 `demo_tts4.mp4`（唯一带 AAC 的），ffprobe 逐个筛 |
| A10 缺依赖 | 无 `ffmpeg` / `tesseract` | `apt-get install -y ffmpeg tesseract-ocr`（云侧安装，不违反"本机不编译"） |
| whisper 路径写死 Radxa | kbv-distill 默认 `/home/radxa/whisper/...` | 环境变量覆盖 `WHISPER=` / `MODEL=`（脚本留了钩子，**无需改源码**） |
| whisper 未装 | — | A10 clone+编译 whisper.cpp + 从 HF 拉 `ggml-base.bin`（148MB，A10 直连 HF 可用） |
| `kbv.json.slices` 是**文件名数组** | `'str' object has no attribute 'get'` | 逐文件读 `kbv_out/slices/*.json` |
| publish 需凭据 | A10 无 gh 认证 | **本机发布**（gh 已认证），token 不出本机 |

## 三、对 lyco_agent 定位的含义（重要）

- **视频蒸馏管线（kbv-distill + kbv-index）在 lystack 里已经完整存在**，且跑通。
  → `lyco_agent` 的 `learn.rs`/`verify.rs` **确实与它重叠**（此前审计的重复建设被证实）。
- **本次 MVP 里 `lyco_agent` 干的活 = 建包**（把 kbv 输出 → mpkg manifest: atoms ref / steps / verify）。
  这是**薄薄一层胶水**，不是一整套 runtime。
- 因此 lyco_agent 的正确收敛方向：
  **① 把这层"kbv→mpkg"胶水固化（可复用 CLI）；② 复用 `cache-node verify` 当验证后端，删掉自研验证器；
  ③ 保留真正独有且未被覆盖的：`shell_exec`(CLI 优先执行) + 领域 skill 索引（learn_cli 读 lilyco `--schema`）+ LVK 的领域语义。**
- 注意 registry 里已有 `asr-code-rust-book-*`、`kbv-video-distill-*` 等包 → 说明**你之前已经用这条路产过包**，本次是端到端复现 + 打通 search/install 消费侧。

## 四、待补（诚实边界）
1. **`attest:0`** —— 本包还没有独立 attestation 回执上架（信任复利未启动）。下一步：`mpkg attest -r` 把本次 ok=True 的回执推上去。
2. 素材是**玩具视频**（20s TTS）；**真实教程视频接入**仍未做（这是 P2 的"待内容"）。
3. `mpkg.json` 由我手写 → 应固化成 `lyco_agent` 的 CLI 子命令（如 `lycore mpkg --from-kbv <kbv_dir>`）。

## 五、下一步（单个动作）
**把"kbv → mpkg"固化成 `lycore` 的一个子命令 + 用 `mpkg attest` 推第一条 attestation**，
于是"学习一次 → 打包 → 验证 → 上架 → 他人复用"整条链**可一键复现**。
