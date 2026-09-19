# A10/V100 CloudStudio: pull real NL->command datasets from HuggingFace,
# normalize + filter, write to /workspace/clidata/. Runs INSIDE kernel (network only, no GPU).
import os, json, subprocess, sys, re, time

OUT = "/workspace/clidata"


def run(cmd, timeout=600):
    print(f"$ {cmd}", flush=True)
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=timeout)
    if r.stdout:
        print(r.stdout[-3000:], flush=True)
    if r.returncode != 0 and r.stderr:
        print("STDERR:", r.stderr[-2500:], flush=True)
    return r


def try_install():
    # datasets lib should already be in the venv from stage2
    code = "import datasets, huggingface_hub; print('ok', datasets.__version__, huggingface_hub.__version__)"
    py = os.path.expanduser("~/lyco_agent/.venv/bin/python")
    r = subprocess.run(f"{py} -c \"{code}\"", shell=True, capture_output=True, text=True)
    if r.returncode != 0:
        print("installing datasets/huggingface_hub...", flush=True)
        run(f"{os.path.expanduser('~/lyco_agent/.venv/bin/pip')} install -U datasets huggingface_hub", timeout=900)
    return True


def ok_pair(prompt, cmd):
    """Keep only crisp single-line commands (like our 'hw gpio get 0 97' targets)."""
    if not prompt or not cmd:
        return False
    cmd = cmd.strip()
    if len(cmd) > 60 or "\n" in cmd:
        return False
    if len(prompt.strip()) < 8:
        return False
    # drop placeholders / non-commands
    if cmd.lower() in ("n/a", "echo n/a", "nan", "none"):
        return False
    if re.search(r"<\w+>", cmd):  # <path> style templates
        return False
    return True


SOURCES = [
    # (repo_id, kind)
    ("emirkaanozdemr/bash_command_data_6K", "prompt_completion"),
    ("dilkushsingh/NL2Bash", "nl_bash"),
    ("Eng-Elias/multios-terminal-commands", "multios"),
]


def load_all(endpoint=None):
    if endpoint:
        os.environ["HF_ENDPOINT"] = endpoint
    from datasets import load_dataset

    pairs = []
    for repo, kind in SOURCES:
        print(f"\n--- {repo} ({kind}) ---", flush=True)
        try:
            ds = load_dataset(repo)
        except Exception as e:
            print(f"  load failed: {type(e).__name__}: {str(e)[:300]}", flush=True)
            continue
        split = "train" if "train" in ds else list(ds.keys())[0]
        n0 = len(ds[split])
        got = 0
        for row in ds[split]:
            if kind == "prompt_completion":
                p, c = row.get("prompt"), row.get("completion")
                cands = [(p, c)]
            elif kind == "nl_bash":
                p = row.get("nl")
                cands = [(p, row.get("bash")), (p, row.get("bash2"))]
            else:  # multios
                p = row.get("instruction")
                os_tag = (row.get("input") or "").upper()
                cands = [(p, row.get("output"))] if "LINUX" in os_tag or not os_tag else []
            for pp, cc in cands:
                if ok_pair(pp, cc):
                    pairs.append({"prompt": str(pp).strip(), "command": str(cc).strip(), "src": repo})
                    got += 1
        print(f"  rows={n0} kept={got}", flush=True)
    return pairs


print("=== HF pull: NL->command datasets ===", flush=True)
try_install()
os.makedirs(OUT, exist_ok=True)

pairs = load_all()
if not pairs:
    print("\n[!] direct HF failed, retrying via hf-mirror.com ...", flush=True)
    pairs = load_all("https://hf-mirror.com")
if not pairs:
    print("HF_PULL_FAILED: 直连和镜像都没拿到数据 (可能需要 GFW 出口/token)", flush=True)
    sys.exit(1)

# dedup
seen, uniq = set(), []
for p in pairs:
    k = (p["prompt"].lower(), p["command"].lower())
    if k in seen:
        continue
    seen.add(k)
    uniq.append(p)
print(f"\ntotal kept={len(pairs)} unique={len(uniq)}", flush=True)

from collections import Counter
print("by source:", Counter(p["src"] for p in uniq), flush=True)

# write train/eval split (90/10, deterministic)
import random
random.Random(42).shuffle(uniq)
cut = int(len(uniq) * 0.9)
train, ev = uniq[:cut], uniq[cut:]


def dump(path, rows):
    with open(path, "w", encoding="utf-8") as f:
        for r in rows:
            json.dump({"messages": [
                {"role": "user", "content": r["prompt"]},
                {"role": "assistant", "content": r["command"]},
            ]}, f, ensure_ascii=False)
            f.write("\n")
    print(f"wrote {path}  n={len(rows)}", flush=True)


dump(os.path.join(OUT, "train.jsonl"), train)
dump(os.path.join(OUT, "eval.jsonl"), ev)
print("\nHF_PULL_DONE", flush=True)
print("samples:", json.dumps(uniq[:3], ensure_ascii=False), flush=True)
