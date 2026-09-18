# A10/V100 CloudStudio: run the repo's tool-calling GRPO trainer (Qwen3-0.6B).
# Reuses tools/qwen_grpo_train.py verbatim. Runs INSIDE the workspace kernel.
import subprocess, os, sys, time

REPO = os.path.join(os.path.expanduser("~"), "lyco_agent")
PY = f"{REPO}/.venv/bin/python"

def run(cmd, cwd, timeout=3500):
    print(f"$ {cmd}  (cwd={cwd})", flush=True)
    t0 = time.time()
    r = subprocess.run(cmd, shell=True, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    print(r.stdout, flush=True)
    if r.returncode != 0:
        if r.stderr: print("STDERR:", r.stderr[-4000:], flush=True)
        raise SystemExit(f"exit {r.returncode}")
    print(f"[dt={time.time()-t0:.1f}s]", flush=True)
    return r

print("=== train: qwen_grpo_train.py (tool-calling GRPO, Qwen3-0.6B) ===", flush=True)
run(f"{PY} tools/qwen_grpo_train.py", cwd=REPO, timeout=3500)
print("GRPO_TRAIN_DONE", flush=True)
