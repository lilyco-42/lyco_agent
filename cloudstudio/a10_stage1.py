# A10/V100 CloudStudio stage1: clone lyco_agent + venv + torch (CUDA).
# Runs INSIDE the workspace via the JPS python3 kernel. Not on local machine.
import subprocess, sys, os, time

REPO = os.path.join(os.path.expanduser("~"), "lyco_agent")
LOG = []

def run(cmd, cwd=None, timeout=3500):
    LOG.append(f"$ {cmd}  (cwd={cwd})")
    t0 = time.time()
    r = subprocess.run(cmd, shell=True, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    dt = time.time() - t0
    out = (r.stdout or "")
    err = (r.stderr or "")
    LOG.append(out)
    if err.strip():
        LOG.append("--- STDERR ---\n" + err)
    LOG.append(f"[exit={r.returncode} dt={dt:.1f}s]")
    if r.returncode != 0:
        print("\n".join(LOG))
        raise SystemExit(f"FAILED: {cmd} -> {r.returncode}")
    return r

print("=== stage1 clone+torch", flush=True)
print(run("nvidia-smi -L || true").stdout, flush=True)
print(run("python3 --version").stdout, flush=True)

if not os.path.isdir(REPO):
    run(f"git clone https://github.com/lilyco-42/lyco_agent.git {REPO}")
else:
    print("repo exists -> git pull --ff-only", flush=True)
    run("git pull --ff-only", cwd=REPO)

os.chdir(REPO)
run("python3 -m venv .venv")
run(".venv/bin/python -m pip install -U pip wheel setuptools", cwd=REPO)
# torch (CUDA build, default PyPI wheel bundles CUDA 12.x)
run(".venv/bin/pip install torch", cwd=REPO, timeout=3500)
print("STAGE1_DONE", flush=True)
