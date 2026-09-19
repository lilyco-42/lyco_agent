import shutil, os
for g in ("grpo-Q4_K_M.gguf", "router_v4-Q4_K_M.gguf"):
    src = "/root/" + g
    dst = "/workspace/" + g
    if not os.path.exists(src):
        print("missing src", src, flush=True)
        continue
    if os.path.exists(dst) and os.path.getsize(dst) == os.path.getsize(src):
        print("already there", dst, f"{os.path.getsize(dst)/1e6:.1f} MB", flush=True)
        continue
    shutil.copy2(src, dst)
    print("copied", dst, f"{os.path.getsize(dst)/1e6:.1f} MB", flush=True)
print("COPY_DONE", flush=True)
