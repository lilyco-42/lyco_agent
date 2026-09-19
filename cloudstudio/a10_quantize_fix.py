# Fix router_v4 -> GGUF: tokenizer_config.json 的 extra_special_tokens 是 list(应为 dict),
# 导致 convert_hf_to_gguf.py 里 AutoTokenizer 加载失败。规范化; 不行则用 grpo 的 tokenizer 拼干净目录兜底。
import os, json, shutil, subprocess, time

HOME = os.path.expanduser("~")
LLAMA = os.path.join(HOME, "llama.cpp")
REPO = os.path.join(HOME, "lyco_agent")
PY = f"{REPO}/.venv/bin/python"
PIP = f"{REPO}/.venv/bin/pip"
D = "/workspace/qwen3_router_v4"
GOOD_TOK_DIR = "/workspace/qwen3_lyco_grpo"     # 已验证可转换
OUT_GGUF = f"{HOME}/router_v4.gguf"
OUT_Q = f"{HOME}/router_v4-Q4_K_M.gguf"


def run(cmd, timeout=1800, quiet=False):
    if not quiet:
        print("$ " + cmd, flush=True)
    t0 = time.time()
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=timeout)
    if r.stdout and not quiet:
        print(r.stdout[-1500:], flush=True)
    if r.returncode != 0:
        print("STDERR:", (r.stderr or "")[-2500:], flush=True)
    print(f"[exit={r.returncode} dt={time.time()-t0:.1f}s]", flush=True)
    return r


print("=== fix router_v4 tokenizer + convert + quantize ===", flush=True)

# 1) 诊断并规范化 tokenizer_config.json
cfg = os.path.join(D, "tokenizer_config.json")
with open(cfg, encoding="utf-8") as f:
    j = json.load(f)
est = j.get("extra_special_tokens", "<absent>")
print(f"extra_special_tokens type = {type(est).__name__}  value={str(est)[:120]}", flush=True)
if isinstance(est, list):
    j["extra_special_tokens"] = {}
    with open(cfg, "w", encoding="utf-8") as f:
        json.dump(j, f, ensure_ascii=False, indent=2)
    print("=> normalized extra_special_tokens to {}", flush=True)

# 2) protobuf (convert 脚本的某些分支需要)
run(f"{PIP} install -q protobuf", timeout=600)

# 3) 先就地转换
r = run(f"{PY} {LLAMA}/convert_hf_to_gguf.py {D} --outfile {OUT_GGUF}", timeout=1800)
ok = r.returncode == 0 and os.path.exists(OUT_GGUF)

# 4) 兜底: 拼干净目录 (v4 权重 + grpo 的 tokenizer)
if not ok:
    print("=> in-place convert failed, building clean dir (v4 weights + grpo tokenizer)", flush=True)
    TMP = "/workspace/conv_v4_clean"
    os.makedirs(TMP, exist_ok=True)
    for fn in ("config.json", "model.safetensors", "generation_config.json"):
        src = os.path.join(D, fn)
        if os.path.exists(src):
            shutil.copy2(src, os.path.join(TMP, fn))
    got = []
    for fn in ("tokenizer.json", "tokenizer_config.json", "vocab.json", "merges.txt",
               "added_tokens.json", "special_tokens_map.json", "chat_template.jinja",
               "tokenizer.model"):
        src = os.path.join(GOOD_TOK_DIR, fn)
        if os.path.exists(src):
            shutil.copy2(src, os.path.join(TMP, fn))
            got.append(fn)
    print("copied tokenizer files:", got, flush=True)
    r = run(f"{PY} {LLAMA}/convert_hf_to_gguf.py {TMP} --outfile {OUT_GGUF}", timeout=1800)
    ok = r.returncode == 0 and os.path.exists(OUT_GGUF)

if not ok:
    print("ROUTER_V4_QUANT_FAILED: convert 仍失败", flush=True)
    raise SystemExit(1)

# 5) 量化
run(f"{LLAMA}/build/bin/llama-quantize {OUT_GGUF} {OUT_Q} Q4_K_M", timeout=1800)
print(f"router_v4 Q4_K_M -> {OUT_Q}  ({os.path.getsize(OUT_Q)/1e6:.1f} MB)", flush=True)
print("ROUTER_V4_QUANT_DONE", flush=True)
