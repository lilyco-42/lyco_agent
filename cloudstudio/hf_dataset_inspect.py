# Inspect candidate HF datasets: verify schema really is NL(text) -> command.
# Uses datasets-server first-rows API (no big download). Local run.
import json, ssl, urllib.request, urllib.parse, urllib.error

ctx = ssl.create_default_context()
BASE = "https://datasets-server.huggingface.co"

CANDIDATES = [
    "jiacheng-ye/nl2bash",
    "dilkushsingh/NL2Bash",
    "Frost2o24/bash-instruct-III-55k",
    "emirkaanozdemr/bash_command_data_6K",
    "mshojaei77/terminal-command-execution-sft",
    "Eccentricity/bashbench2",
    "huytd189/command-line-suggestions",
    "Canstralian/ShellCommands",
    "kipasyangin5/terminal-cli-commands-dataset",
    "Eng-Elias/multios-terminal-commands",
    "PocketDoc/Dans-Toolmaxx-ShellCommands",
]


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    with urllib.request.urlopen(req, timeout=45, context=ctx) as r:
        return json.load(r)


def look(dataset):
    print("\n" + "=" * 70)
    print(dataset)
    try:
        sp = get(f"{BASE}/splits?dataset={urllib.parse.quote(dataset)}")
        splits = sp.get("splits", [])
        if not splits:
            print("  no splits (maybe gated / unsupported)")
            return
        cfg = splits[0]["config"]
        split = splits[0]["split"]
        print(f"  config={cfg} split={split}  (total splits={len(splits)})")
        fr = get(f"{BASE}/first-rows?dataset={urllib.parse.quote(dataset)}"
                 f"&config={urllib.parse.quote(cfg)}&split={urllib.parse.quote(split)}")
        feats = [f["name"] for f in fr.get("features", [])]
        print(f"  features: {feats}")
        rows = fr.get("rows", [])[:2]
        for i, rw in enumerate(rows):
            row = rw.get("row", {})
            # trim long values for readability
            trimmed = {k: (str(v)[:160] + ("..." if len(str(v)) > 160 else "")) for k, v in row.items()}
            print(f"  row[{i}]: {json.dumps(trimmed, ensure_ascii=False)[:600]}")
    except urllib.error.HTTPError as e:
        print(f"  HTTP {e.code}: {e.reason}")
    except Exception as e:
        print(f"  ERR: {type(e).__name__}: {e}")


for d in CANDIDATES:
    look(d)
