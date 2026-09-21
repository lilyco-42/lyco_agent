"""采集 help 样本 + 跑 lycore help-parse，输出 JSON 供 ground 校验 + 回归比对。

用法:
  python collect.py            # 采集所有 CLI 的 help 到 fixtures/
  python collect.py --parse    # 对已采集的样本跑 help-parse 并写 parsed/<cli>.json
"""
import json, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
FIX = os.path.join(HERE, "fixtures")
PARSED = os.path.join(HERE, "parsed")
LYCORE = "D:/Code/lyco_agent/lycore/target/debug/lycore.exe"

# (cli, 取 help 的 argv)  —— 池内 + L1
TARGETS = [
    # 池内（回归基线）
    ("docker", ["docker", "--help"]),
    ("git", ["git", "--help"]),
    ("cargo", ["cargo", "--help"]),
    ("npm", ["npm", "--help"]),
    ("kubectl", ["kubectl", "--help"]),
    ("jq", ["jq", "--help"]),
    # L1（外部陌生）
    ("fd", ["fd", "--help"]),
    ("rg", ["rg", "--help"]),
    ("hyperfine", ["hyperfine", "--help"]),
    ("zoxide", ["zoxide", "--help"]),
    ("bat", ["bat", "--help"]),
    ("starship", ["starship", "--help"]),
    ("oha", ["oha", "--help"]),
    # L1 候选扩充
    ("dust", ["dust", "--help"]),
    ("sd", ["sd", "--help"]),
    ("tokei", ["tokei", "--help"]),
    ("xh", ["xh", "--help"]),
    ("delta", ["delta", "--help"]),
    ("procs", ["procs", "--help"]),
    ("bottom", ["bottom", "--help"]),
    ("just", ["just", "--help"]),
    ("eza", ["eza", "--help"]),
]


def collect():
    os.makedirs(FIX, exist_ok=True)
    ok, miss = [], []
    for name, argv in TARGETS:
        try:
            r = subprocess.run(argv, capture_output=True, text=True,
                               timeout=20, encoding="utf-8", errors="replace")
            txt = (r.stdout or "") + (r.stderr or "")
            if not txt.strip():
                miss.append((name, "empty"))
                continue
            with open(os.path.join(FIX, name + ".txt"), "w", encoding="utf-8") as f:
                f.write(txt)
            ok.append((name, len(txt)))
        except FileNotFoundError:
            miss.append((name, "not-installed"))
        except Exception as e:
            miss.append((name, f"{type(e).__name__}:{e}"))
    print(f"[collect] 采到 {len(ok)} 个 / 缺 {len(miss)} 个")
    for n, l in ok:
        print(f"  [ok]  {n:12s} {l:6d}B")
    for n, why in miss:
        print(f"  [--]  {n:12s} {why}")
    return ok, miss


def parse():
    os.makedirs(PARSED, exist_ok=True)
    rows = []
    for name, _ in TARGETS:
        p = os.path.join(FIX, name + ".txt")
        if not os.path.exists(p):
            continue
        r = subprocess.run([LYCORE, "help-parse", "--cli", name,
                            "--help-text", p, "--top", "200", "--json"],
                           capture_output=True, text=True, encoding="utf-8",
                           errors="replace", timeout=30)
        out = (r.stdout or "").strip()
        try:
            j = json.loads(out)
        except Exception:
            print(f"  [!!] {name}: 非 JSON 输出 rc={r.returncode} {out[:200]} {r.stderr[:200]}")
            continue
        with open(os.path.join(PARSED, name + ".json"), "w", encoding="utf-8") as f:
            json.dump(j, f, ensure_ascii=False, indent=1)
        acts = j.get("actions") or j.get("items") or []
        ro = [a for a in acts if (a.get("safety") or a.get("access") or "").lower() in ("read", "read_only", "ro")]
        rows.append((name, len(acts), len(ro), j))
    print(f"{'cli':12s} {'actions':>8s} {'readonly':>9s}")
    for n, a, ro, _ in rows:
        print(f"{n:12s} {a:8d} {ro:9d}")


if __name__ == "__main__":
    if "--parse" in sys.argv:
        parse()
    else:
        collect()
