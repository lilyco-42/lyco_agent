import os
from huggingface_hub import HfApi

REPO = "lyco42/lyco-agent-qwen3-0.6b-ondevice"
LOCAL = r"D:\Code\lyco_agent\models"
api = HfApi()

info = api.model_info(REPO, files_metadata=True)
print("repo:", REPO)
print("private:", info.private, "| sha:", (info.sha or "")[:12], "| lastModified:", info.lastModified)
print("\n=== 远端文件 ===")
remote = {}
for f in info.siblings:
    remote[f.rfilename] = f.size
    print(f"  {f.rfilename:34s} {f.size}")

print("\n=== 与本地比对 ===")
for fn in ("README.md", "router_merged-Q4_K_M.gguf", "grpo-Q4_K_M.gguf"):
    r = remote.get(fn)
    lp = os.path.join(LOCAL, fn)
    l = os.path.getsize(lp) if os.path.exists(lp) else None
    if fn == "README.md":
        l = os.path.getsize(os.path.join(LOCAL, "HF_README.md")) if os.path.exists(os.path.join(LOCAL, "HF_README.md")) else None
    ok = (r is not None and l is not None and r == l)
    print(f"  {fn:34s} remote={r} local={l} match={'YES' if ok else 'no/na'}")

print("\nURL: https://huggingface.co/" + REPO)
