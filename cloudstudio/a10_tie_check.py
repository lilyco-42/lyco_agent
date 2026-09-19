# 判定 router_v4 的 lm_head.weight 是否只是 embed_tokens 的副本 (若相同 -> 绑定无损, 可省 ~83MB)
import os, json, torch

DIRS = [("grpo", "/workspace/qwen3_lyco_grpo"), ("router_v4", "/workspace/qwen3_router_v4")]

for name, d in DIRS:
    print(f"\n=== {name} : {d} ===", flush=True)
    st = os.path.join(d, "model.safetensors")
    if not os.path.exists(st):
        print("  no safetensors", flush=True)
        continue
    try:
        from safetensors import safe_open
        with safe_open(st, framework="pt") as f:
            keys = list(f.keys())
        print(f"  tensors={len(keys)}", flush=True)
        has_lh = "lm_head.weight" in keys
        has_emb = "model.embed_tokens.weight" in keys
        print(f"  has lm_head={has_lh}  has embed_tokens={has_emb}", flush=True)
        if has_lh and has_emb:
            with safe_open(st, framework="pt") as f:
                lh = f.get_tensor("lm_head.weight").float()
                emb = f.get_tensor("model.embed_tokens.weight").float()
            same = torch.equal(lh, emb)
            md = (lh - emb).abs().max().item()
            print(f"  lm_head == embed_tokens ? {same}   max_abs_diff={md:.6g}", flush=True)
            del lh, emb
    except Exception as e:
        print("  ERR:", type(e).__name__, e, flush=True)

    # 同时打印 config 里和 tie 相关的字段
    try:
        cfg = json.load(open(os.path.join(d, "config.json"), encoding="utf-8"))
        print("  config.tie_word_embeddings =", cfg.get("tie_word_embeddings"), flush=True)
        print("  transformers_version(save) =", cfg.get("transformers_version"), flush=True)
    except Exception as e:
        print("  config ERR:", e, flush=True)

print("\nTIE_CHECK_DONE", flush=True)
