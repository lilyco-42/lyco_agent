# 1) 判定 lm_head 是否为 embed 副本  2) 找出 JPS 的真实 root_dir  3) 把 GGUF 复制到那里以便下载
import os, json, shutil, subprocess, torch

print("=== 1) tie check ===", flush=True)
for name, d in (("grpo", "/workspace/qwen3_lyco_grpo"), ("router_v4", "/workspace/qwen3_router_v4")):
    st = os.path.join(d, "model.safetensors")
    print(f"\n-- {name} --", flush=True)
    if not os.path.exists(st):
        print("  no safetensors", flush=True)
        continue
    try:
        from safetensors import safe_open
        with safe_open(st, framework="pt") as f:
            keys = list(f.keys())
        has_lh = "lm_head.weight" in keys
        print(f"  tensors={len(keys)}  has_lm_head={has_lh}", flush=True)
        if has_lh:
            with safe_open(st, framework="pt") as f:
                lh = f.get_tensor("lm_head.weight").float()
                emb = f.get_tensor("model.embed_tokens.weight").float()
            print(f"  lm_head==embed ? {torch.equal(lh, emb)}  maxdiff={(lh-emb).abs().max().item():.6g}", flush=True)
            del lh, emb
    except Exception as e:
        print("  ERR:", type(e).__name__, e, flush=True)

print("\n=== 2) find jupyter root_dir ===", flush=True)
try:
    ps = subprocess.run("ps auxww | grep -i jupyter | grep -v grep", shell=True,
                        capture_output=True, text=True).stdout
    print(ps[:4000], flush=True)
    root_dir = None
    for tok in ps.split():
        if tok.startswith("--ServerApp.root_dir=") or tok.startswith("--NotebookApp.notebook_dir="):
            root_dir = tok.split("=", 1)[1]
    print("parsed root_dir =", root_dir, flush=True)
except Exception as e:
    print("ps ERR:", e, flush=True)
    root_dir = None

if not root_dir:
    # 兜底: 试常见位置, 挑一个存在且含 Cargo.toml 的
    for cand in ("/workspace", "/root", os.path.expanduser("~"), "/home", "/data"):
        if os.path.isdir(cand):
            print(f"  cand {cand}: {sorted(os.listdir(cand))[:8]}", flush=True)

print("\n=== 3) copy ggufs to jps root ===", flush=True)
GGS = ["grpo-Q4_K_M.gguf", "router_v4-Q4_K_M.gguf"]
targets = []
if root_dir and os.path.isdir(root_dir):
    targets.append(root_dir)
# 无论解析成功与否, 也复制到内核 HOME (备查)
targets.append(os.path.expanduser("~"))
seen = set()
for t in targets:
    if t in seen:
        continue
    seen.add(t)
    for g in GGS:
        src = os.path.join(os.path.expanduser("~"), g)
        if os.path.exists(src):
            dst = os.path.join(t, g)
            if os.path.abspath(src) == os.path.abspath(dst):
                print(f"  already in {t}: {g}", flush=True)
                continue
            shutil.copy2(src, dst)
            print(f"  copied {g} -> {dst} ({os.path.getsize(dst)/1e6:.1f} MB)", flush=True)

print("\nPREP_DONE", flush=True)
