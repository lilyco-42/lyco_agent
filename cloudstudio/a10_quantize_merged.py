# 把 v3⊕v4 合并模型量化为 GGUF Q4_K_M（它是目前最优：heldA 98.5% / heldB 79.7% / reject 100%）
import os, json, shutil, subprocess, time

HOME = os.path.expanduser("~")
LLAMA = os.path.join(HOME, "llama.cpp")
REPO = os.path.join(HOME, "lyco_agent")
PY = f"{REPO}/.venv/bin/python"
D = "/workspace/qwen3_router_merged"
OUT_GGUF = f"{HOME}/router_merged.gguf"
OUT_Q = f"{HOME}/router_merged-Q4_K_M.gguf"


def run(cmd, timeout=1800, quiet=False):
    if not quiet: print("$ " + cmd, flush=True)
    t0 = time.time()
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=timeout)
    if r.stdout and not quiet: print(r.stdout[-1200:], flush=True)
    if r.returncode != 0: print("STDERR:", (r.stderr or "")[-2000:], flush=True)
    print(f"[exit={r.returncode} dt={time.time()-t0:.1f}s]", flush=True)
    return r


print("=== quantize MERGED (v3⊕v4) -> Q4_K_M ===", flush=True)
if not os.path.isdir(D):
    print("MISSING merged dir:", D, flush=True); raise SystemExit(1)
print("dir contents:", sorted(os.listdir(D)), flush=True)

# tokenizer_config 的 extra_special_tokens 若是 list 会让 convert 挂掉（v4 同款问题）
cfg = os.path.join(D, "tokenizer_config.json")
if os.path.exists(cfg):
    j = json.load(open(cfg, encoding="utf-8"))
    est = j.get("extra_special_tokens", "<absent>")
    print("extra_special_tokens type:", type(est).__name__, flush=True)
    if isinstance(est, list):
        j["extra_special_tokens"] = {}
        json.dump(j, open(cfg, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
        print("=> normalized to {}", flush=True)

r = run(f"{PY} {LLAMA}/convert_hf_to_gguf.py {D} --outfile {OUT_GGUF}", timeout=1800)
if r.returncode != 0 or not os.path.exists(OUT_GGUF):
    print("QUANT_FAILED: convert", flush=True); raise SystemExit(1)

run(f"{LLAMA}/build/bin/llama-quantize {OUT_GGUF} {OUT_Q} Q4_K_M", timeout=1800)
print(f"Q4_K_M -> {OUT_Q}  ({os.path.getsize(OUT_Q)/1e6:.1f} MB)", flush=True)

# 复制到 JPS root 以便下载
dst = f"/workspace/router_merged-Q4_K_M.gguf"
shutil.copy2(OUT_Q, dst)
print(f"staged for download: {dst} ({os.path.getsize(dst)/1e6:.1f} MB)", flush=True)

# 顺手 bench 一下
run(f"{LLAMA}/build/bin/llama-bench -m {OUT_Q} -p 64 -n 32 -r 3 -t 8", timeout=900)
print("MERGED_QUANT_DONE", flush=True)
