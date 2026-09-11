# lyco_agent Radxa A7A 部署指南

## 1. 依赖 (Radxa, Debian/Ubuntu ARM64)
sudo apt install -y build-essential ffmpeg tesseract-clang chromium
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # rust
pip install "rembg[cpu]" onnxruntime                               # 抠图 CLI

## 2. 编译 (板子上, 一次性 ~10min)
tar xzf lyco_radxa.tar.gz && cd lycore
cargo build --release        # aarch64 原生编译

## 3. 环境变量 (.env 或 export)
export LYCO_LLM_KEY=nvapi-xxxx          # NIM key (cc-switch Nvidia provider)
export LYCO_LLM_MODEL=meta/llama-3.2-11b-vision-instruct
export LYCO_LLM_URL=https://integrate.api.nvidia.com/v1/chat/completions
# 或 OpenRouter free: LYCO_LLM_URL=https://openrouter.ai/api/v1/chat/completions
export LYV_FFMPEG=ffmpeg
export LYCO_CHROME=chromium

## 4. 运行 agent loop (0.6B FC + 工具编排)
# llama.cpp ARM build: 
git clone https://github.com/ggml-org/llama.cpp && cd llama.cpp
cmake -B build -DGGML_NATIVE=ON && cmake --build build -j8 --target llama-server
./build/bin/llama-server -m qwen3_lyco_fc_v2_q4km.gguf --port 8081 --jinja -t 8

# lyco agent:
./lycore/target/release/lycore ask --pack ../smoke/pack_merged \
  --llama http://localhost:8081 "帮我做一个登录页面然后转成视频"

## 工具链
rembg_remove  → 本地 rembg (onnxruntime CPU)
html_gen / llm_generate → NIM / OpenRouter free API
html_render_video → chromium headless + ffmpeg
video_info → ffprobe
vnn_identify → 内置 CNN (lycore/assets)
lyv_knowledge → 知识包 sqlite/FTS
