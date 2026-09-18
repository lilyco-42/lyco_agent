# A10/V100 CloudStudio stage2: install the rest of the training stack (pinned, coherent)
# and verify the env. Runs INSIDE the workspace via the JPS python3 kernel.
import subprocess, os, sys, time

REPO = os.path.join(os.path.expanduser("~"), "lyco_agent")
PIP = f"{REPO}/.venv/bin/pip"
PY = f"{REPO}/.venv/bin/python"

# Coherent mid-2025 stack that runs the repo's qwen_grpo_train.py / sft_cli.py unmodified.
# (torch 2.5.1 cu121 wheel runs fine on A10's CUDA 12.x driver.)
PIN = "torch==2.5.1 transformers==4.51.3 trl==0.16.1 accelerate==1.6.0 peft==0.15.2 datasets==3.3.2 bitsandbytes==0.45.5 numpy==1.26.4 sentencepiece"

def run(cmd, timeout=3500):
    print(f"$ {cmd}", flush=True)
    t0 = time.time()
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=timeout)
    if r.stdout: print(r.stdout, flush=True)
    if r.returncode != 0:
        if r.stderr: print("STDERR:", r.stderr[-3000:], flush=True)
        raise SystemExit(f"exit {r.returncode}: {cmd}")
    print(f"[dt={time.time()-t0:.1f}s]", flush=True)
    return r

# 0) sanity: did stage1 clone + venv succeed?
if not os.path.isfile(f"{REPO}/.venv/bin/python"):
    raise SystemExit("venv missing — stage1 did not finish. Wait for stage1.")
if not os.path.isfile(f"{REPO}/tools/qwen_grpo_train.py"):
    raise SystemExit("repo tools missing — stage1 clone failed.")

# 1) pin the whole stack (downgrades torch if stage1 pulled newer)
run(f"{PIP} install -U {PIN}")

# 2) API smoke test (catches trl API drift before a long training run)
check = '''
import inspect
mods = {}
for m in ("torch","transformers","trl","peft","datasets","accelerate","bitsandbytes"):
    try: mods[m] = __import__(m).__version__
    except Exception as e: mods[m] = f"ERR:{e}"
print("VERSIONS", mods)
import torch
print("CUDA", torch.cuda.is_available(), "n=", torch.cuda.device_count(),
      torch.cuda.get_device_name(0) if torch.cuda.is_available() else "NA")
from trl import GRPOConfig, GRPOTrainer
sig = inspect.signature(GRPOTrainer.__init__)
print("API reward_funcs=", "reward_funcs" in sig.parameters,
      "processing_class=", "processing_class" in sig.parameters)
'''
open("/tmp/check_api.py", "w").write(check)
r = run(f"{PY} /tmp/check_api.py")
# fallback: if reward_funcs missing (trl>=0.20 renamed it), drop to 0.15.2
if "reward_funcs= False" in r.stdout or "reward_funcs= False" in r.stdout:
    print("!! trl lacks reward_funcs — falling back to trl==0.15.2", flush=True)
    run(f"{PIP} install 'trl==0.15.2'")
    run(f"{PY} /tmp/check_api.py")

# 3) confirm repo tooling present
run(f"ls -1 {REPO}/tools/qwen_grpo_train.py {REPO}/tools/sft_cli.py {REPO}/tools/fc_grpo_v4.py")
run(f"ls -1 {REPO}/tools/cgidata/ 2>/dev/null || echo 'no cgidata dir'")
print("STAGE2_DONE", flush=True)
