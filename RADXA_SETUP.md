# lyco_agent Radxa A7A 部署指南

## 1. 依赖 (Radxa, Debian/Ubuntu ARM64)
sudo apt install -y build-essential ffmpeg tesseract-clang chromium
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # rust
pip install "rembg[cpu]" onnxruntime                               # 抠图 CLI

## 2. lycore 二进制 (二选一)
# A) 直接用预编译 (推荐): release 附件 lycore-aarch64, glibc 动态链接 (需 GLIBC≥2.39,
#    Ubuntu 24.04 / Debian 13+ 满足; 更旧镜像需走 B 或 musl 静态重编)
chmod +x lycore-aarch64 && mv lycore-aarch64 lycore
# B) 板上原生编译 (一次性 ~10min, 需 §1 的 rustup):
# 源码包 (含 lycore 源码 + 演示知识包 + 训练脚本, 从 git HEAD 构建):
# 下载: https://github.com/lilyco-42/lyco_agent/releases/download/v0.3.0/lyco_radxa_src.tar.gz
tar xzf lyco_radxa_src.tar.gz && cd lycore && cargo build --release

## 3. 环境变量 (.env 或 export)
export LYCO_LLM_KEY=nvapi-xxxx          # NIM key (cc-switch Nvidia provider)
export LYCO_LLM_MODEL=meta/llama-3.2-11b-vision-instruct
export LYCO_LLM_URL=https://integrate.api.nvidia.com/v1/chat/completions
# 或 OpenRouter free: LYCO_LLM_URL=https://openrouter.ai/api/v1/chat/completions
export LYV_FFMPEG=ffmpeg
export LYCO_CHROME=chromium

## 4. 运行 agent loop (0.6B FC + 工具编排)
# llama.cpp ARM build (板上原生编译 — CloudStudio x86 交叉编译会被节点重启打断, 实测不可靠)
git clone https://github.com/ggml-org/llama.cpp && cd llama.cpp
cmake -B build -DGGML_NATIVE=ON -DLLAMA_CURL=OFF && cmake --build build -j8 --target llama-server
./build/bin/llama-server -m qwen3_lyco_fc_v4_q6k.gguf --port 8081 --jinja -t 8

# lyco agent (路径 A 用 ./lycore, 路径 B 用 ./lycore/target/release/lycore):
./lycore ask --pack ../smoke/pack_merged \
  --llama http://localhost:8081 "帮我做一个登录页面然后转成视频"

## 工具链
rembg_remove  → 本地 rembg (onnxruntime CPU)
html_gen / llm_generate → NIM / OpenRouter free API
html_render_video → chromium headless + ffmpeg
video_info → ffprobe
vnn_identify → 内置 CNN (lycore/assets)
lyv_knowledge → 知识包 sqlite/FTS
