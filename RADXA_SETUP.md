# lyco_agent Radxa A7A 部署指南

## 1. 依赖 (Radxa, Debian/Ubuntu ARM64)
sudo apt install -y build-essential ffmpeg tesseract-clang chromium
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # rust
pip install "rembg[cpu]" onnxruntime                               # 抠图 CLI

## 2. lycore 二进制 (二选一)
# A) 直接用预编译静态版 (推荐): musl 静态链接, 无 glibc 依赖 → 任意 aarch64 Linux
#    可跑 (Ubuntu 22.04/24.04、Debian 12/13 皆可), 无需关心板子 glibc 版本。
# 下载: https://github.com/lilyco-42/lyco_agent/releases/latest/download/lycore-aarch64-static
chmod +x lycore-aarch64-static && mv lycore-aarch64-static lycore
# (备选: lycore-aarch64 是 gnu 动态版, 需 glibc≥2.39, 仅 Ubuntu 24.04+/Debian 13+ 满足)
# B) 板上原生编译 (一次性 ~10min, 需 §1 的 rustup):
# 源码包 (含 lycore 源码 + 演示知识包 + 训练脚本, 从 git HEAD 构建):
# 下载: https://github.com/lilyco-42/lyco_agent/releases/latest/download/lyco_radxa_src.tar.gz
tar xzf lyco_radxa_src.tar.gz && cd lycore && cargo build --release

## 3. 环境变量 (.env 或 export)
export LYCO_LLM_KEY=nvapi-xxxx          # NIM key (cc-switch Nvidia provider)
export LYCO_LLM_MODEL=meta/llama-3.2-11b-vision-instruct
export LYCO_LLM_URL=https://integrate.api.nvidia.com/v1/chat/completions
# 或 OpenRouter free: LYCO_LLM_URL=https://openrouter.ai/api/v1/chat/completions
export LYV_FFMPEG=ffmpeg
export LYCO_CHROME=chromium

## 4. 运行 agent loop (0.6B FC + 工具编排)
# FC 决策模型 (训练产物, 固定在 v0.3.0 附件; 重训后才更新):
# 下载: https://github.com/lilyco-42/lyco_agent/releases/download/v0.3.0/qwen3_lyco_fc_v4_q6k.gguf

### 4.0 ⚠️ 必须显式限制 ctx, 否则必 OOM
# Qwen3-0.6B: 28 层 × 8 KV head × head_dim 128 → KV 112 KB/token。
# 模型 max_position_embeddings = 40960 → 不指定 -c 时 KV 单吃 4.38 GB > 全板 4 GB。
# 4GB 板: -c 2048 (KV 0.22G) 起步, 余量够再上 4096 (KV 0.44G)。权重 Q6_K 另占 0.47G。

### 4.1 llama.cpp ARM build
# 板上原生编译 (CloudStudio x86 交叉编译会被节点重启打断, 实测不可靠)
# 4GB 板 -j8 有 OOM 风险 → 开 swap + -j4; SSH 一断构建就被 SIGHUP 杀 → 用 setsid nohup
sudo swapon /swapfile2 2>/dev/null || echo "无 swapfile2 (重启后需重新 swapon)"
git clone https://github.com/ggml-org/llama.cpp && cd llama.cpp
cmake -B build -DGGML_NATIVE=ON -DLLAMA_CURL=OFF
setsid nohup bash -c 'cmake --build build -j4 --target llama-server' </dev/null >build.log 2>&1 &

### 4.2 定 -t: pp 与 tg 必须分开测 (别凭直觉填 8)
# A7A = 2×A76 + 6×A55。prefill 能跨核并行, decode 不能 → 两者最优 -t 不同。
# 本 workload 是 prefill 主导: CHAT_TOOLS schema 每轮 ~600-700 token, FC 决策只吐几十 token。
taskset -c 6,7 ./build/bin/llama-bench -m qwen3_lyco_fc_v4_q6k.gguf -t 2 -p 1024 -n 0   -r 3  # pp
taskset -c 6,7 ./build/bin/llama-bench -m qwen3_lyco_fc_v4_q6k.gguf -t 2 -p 0     -n 128 -r 3  # tg
# pp 明显落后就试 -t 8 全核 (A55 也能贡献 prefill), 取权衡值填进 4.3

### 4.3 启动 server
# --cache-reuse 默认 0 (禁用), 必须显式开: system prompt + CHAT_TOOLS 每轮是同一前缀,
# 开启后后续轮次只 prefill 增量 —— 本场景最大的一笔延迟优化。
# -t 4 是占位值, 用 4.2 实测结果替换。
./build/bin/llama-server -m qwen3_lyco_fc_v4_q6k.gguf --port 8081 --jinja \
  -c 2048 -t 4 --cache-reuse 256

# lyco agent (路径 A 用 ./lycore, 路径 B 用 ./lycore/target/release/lycore):
./lycore ask --pack ../smoke/pack_merged \
  --llama http://localhost:8081 "帮我做一个登录页面然后转成视频"

### 4.4 延迟预期: LLM 不是瓶颈
# FC 决策每轮只吐几十 token (~1-2s)。真正慢的是 html_render_video (headless chrome
# 逐秒截图 + ffmpeg 合成) 与 rembg (onnxruntime CPU) —— 几十秒级, 比 LLM 慢一个数量级。
# 优化模型参数 = 打错靶子; 要提速就砍 chromium 截帧数 / 降分辨率。

## 工具链
rembg_remove  → 本地 rembg (onnxruntime CPU)
html_gen / llm_generate → NIM / OpenRouter free API
html_render_video → chromium headless + ffmpeg
video_info → ffprobe
vnn_identify → 内置 CNN (lycore/assets)
lyv_knowledge → 知识包 sqlite/FTS

<!-- 提交归属说明: §4.0-4.4 (OOM 限制/llama-bench/cache-reuse/延迟预期) 由并发会话
     撰写, 在 95d1d89 (路径 A 改静态) 提交时因整文件 git add 被一并卷入, commit message
     未反映该部分。内容有效, 此处补正归属。 -->
