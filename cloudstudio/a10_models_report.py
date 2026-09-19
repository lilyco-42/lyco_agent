# Inspect tie/lm_head for both saved models + run llama-bench throughput on the GGUFs.
import os, json, subprocess, time

HOME = os.path.expanduser("~")
LLAMA = os.path.join(HOME, "llama.cpp")
BIN = os.path.join(LLAMA, "build", "bin")

MODELS = [
    ("grpo", "/workspace/qwen3_lyco_grpo", f"{HOME}/grpo-Q4_K_M.gguf"),
    ("router_v4", "/workspace/qwen3_router_v4", f"{HOME}/router_v4-Q4_K_M.gguf"),
]

for name, d, gguf in MODELS:
    print(f"\n=== {name} ===", flush=True)
    try:
        cfg = json.load(open(os.path.join(d, "config.json"), encoding="utf-8"))
        print("  tie_word_embeddings:", cfg.get("tie_word_embeddings"), flush=True)
        print("  vocab_size:", cfg.get("vocab_size"), " hidden:", cfg.get("hidden_size"), flush=True)
    except Exception as e:
        print("  config err:", e, flush=True)
    st = os.path.join(d, "model.safetensors")
    if os.path.exists(st):
        try:
            from safetensors.torch import load_file
            sd = load_file(st, framework="pt", device="cpu")
            ks = list(sd.keys())
            print(f"  tensors={len(ks)}  has_lm_head={'lm_head.weight' in ks}", flush=True)
            del sd
        except Exception as e:
            print("  st err:", e, flush=True)
    print("  gguf exists:", os.path.exists(gguf),
          f"{os.path.getsize(gguf)/1e6:.1f} MB" if os.path.exists(gguf) else "", flush=True)

print("\n=== llama-bench (CPU, Q4_K_M) ===", flush=True)
for name, _, gguf in MODELS:
    if not os.path.exists(gguf):
        print(f"skip {name}: no gguf", flush=True)
        continue
    cmd = f"{BIN}/llama-bench -m {gguf} -p 64 -n 32 -r 3 -t 8"
    print(f"$ {cmd}", flush=True)
    t0 = time.time()
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=1800)
    out = (r.stdout or "")
    print(out[-2000:], flush=True)
    if r.returncode != 0:
        print("STDERR:", (r.stderr or "")[-1500:], flush=True)
    print(f"[exit={r.returncode} dt={time.time()-t0:.1f}s]", flush=True)

print("REPORT_DONE", flush=True)
