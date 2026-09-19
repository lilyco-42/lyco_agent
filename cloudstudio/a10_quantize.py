# A10/V100 CloudStudio stage4: quantize trained Qwen3-0.6B checkpoints -> GGUF Q4_K_M.
# Builds llama.cpp (CPU quant is fine), converts HF checkpoints, quantizes.
# Runs INSIDE the workspace via the JPS python3 kernel. Not on local machine.
import subprocess, os, sys, time, shutil

HOME = os.path.expanduser("~")
LLAMA = os.path.join(HOME, "llama.cpp")
REPO = os.path.join(HOME, "lyco_agent")
PY = os.path.join(REPO, ".venv", "bin", "python")

# trained outputs to quantize.
# grpo  = 工具调用/驾驶模型 (tools/qwen_grpo_train.py, FC 80%)
# v4    = CLI 路由器 v4 (拒绝率 100%, 与 v1 准确率持平但不再乱吐命令) <- 采用版
TARGETS = [
    ("grpo", "/workspace/qwen3_lyco_grpo/final"),
    ("grpo", "/workspace/qwen3_lyco_grpo"),
    ("router_v4", "/workspace/qwen3_router_v4"),
]

def run(cmd, cwd=None, timeout=2400):
    print(f"$ {cmd}", flush=True)
    t0 = time.time()
    r = subprocess.run(cmd, shell=True, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    if r.stdout: print(r.stdout, flush=True)
    if r.returncode != 0:
        if r.stderr: print("STDERR:", r.stderr[-4000:], flush=True)
        raise SystemExit(f"exit {r.returncode}: {cmd}")
    print(f"[dt={time.time()-t0:.1f}s]", flush=True)
    return r

# 1) clone + build llama.cpp (CPU build; quantize is CPU-bound anyway)
if not os.path.isdir(LLAMA):
    run(f"git clone --depth 1 https://github.com/ggerganov/llama.cpp {LLAMA}")
# build tools
run(f"cmake -S {LLAMA} -B {LLAMA}/build -DGGML_CUDA=OFF -DCMAKE_BUILD_TYPE=Release", timeout=600)
run(f"cmake --build {LLAMA}/build --config Release -j$(nproc)", timeout=1800)

QUANT = f"{LLAMA}/build/bin/llama-quantize"
CONV = f"{LLAMA}/convert_hf_to_gguf.py"

# 2) convert + quantize each trained ckpt that exists
done = 0
for name, hf in TARGETS:
    # pick the first existing path for this name
    if not os.path.isdir(hf):
        continue
    gguf = f"{HOME}/{name}.gguf"
    q = f"{HOME}/{name}-Q4_K_M.gguf"
    print(f"\n=== {name}: {hf} ===", flush=True)
    run(f"{PY} {CONV} {hf} --outfile {gguf}")
    run(f"{QUANT} {gguf} {q} Q4_K_M")
    sz = os.path.getsize(q) / 1e6
    print(f"Q4_K_M -> {q}  ({sz:.1f} MB)", flush=True)
    done += 1

if done == 0:
    print("NO_TRAINED_CKPT: 先在 stage3 跑 qwen_grpo_train.py / sft_cli.py 产出 checkpoint 再量化。", flush=True)
else:
    print(f"QUANTIZE_DONE ({done} model(s))", flush=True)
