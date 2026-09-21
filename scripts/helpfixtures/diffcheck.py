"""逐字 diff：把「改前 / 改后」的池内动作全量对比，确认零回归是真的逐字零回归。

用法:
  python diffcheck.py save-before   # 改前（需先切到改前代码 build）
  python diffcheck.py save-after
  python diffcheck.py compare
"""
import json, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
FIX = os.path.join(HERE, "fixtures")
LYCORE = "D:/Code/lyco_agent/lycore/target/debug/lycore.exe"
NAMES = ["docker","git","cargo","npm","kubectl","jq","fd","rg","hyperfine","zoxide","bat","starship","oha","dust","sd","tokei","xh","delta","just","eza"]


def snap():
    out = {}
    for n in NAMES:
        p = os.path.join(FIX, n + ".txt")
        if not os.path.exists(p):
            continue
        r = subprocess.run([LYCORE, "help-parse", "--cli", n, "--help-text", p,
                            "--top", "9999", "--include-writes", "--json"],
                           capture_output=True, text=True, encoding="utf-8",
                           errors="replace", timeout=60)
        try:
            j = json.loads(r.stdout.strip())
        except Exception:
            continue
        out[n] = [{"c": a["full_cmd"], "d": a["desc"], "e": a.get("example")} for a in j["actions"]]
    return out


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "compare"
    if mode in ("save-before", "save-after"):
        tag = mode.split("-")[1]
        d = snap()
        json.dump(d, open(os.path.join(HERE, f"snap_{tag}.json"), "w", encoding="utf-8"),
                  ensure_ascii=False, indent=1)
        print(f"[{tag}] 存 {len(d)} 个 CLI, 共 {sum(len(v) for v in d.values())} 条动作")
    else:
        a = json.load(open(os.path.join(HERE, "snap_before.json"), encoding="utf-8"))
        b = json.load(open(os.path.join(HERE, "snap_after.json"), encoding="utf-8"))
        POOL = {"docker", "git", "cargo", "npm", "kubectl", "jq"}
        total_diff = 0
        for n in NAMES:
            if n not in a or n not in b:
                continue
            A, B = a[n], b[n]
            if A == B:
                print(f"  {n:12s} 逐字相同 ({len(A)} 条)")
                continue
            if len(A) != len(B):
                print(f"  {n:12s} ⚠️ 条数 {len(A)} → {len(B)}")
            nd = 0
            for x, y in zip(A, B):
                if x != y:
                    nd += 1
                    if nd <= 3:
                        print(f"      改: {x['c']}")
                        print(f"        → {y['c']}")
                        if x['d'] != y['d']:
                            print(f"        desc: {x['d'][:70]}")
                            print(f"           → {y['d'][:70]}")
                        if x.get('e') != y.get('e'):
                            print(f"        ex:   {x.get('e')} → {y.get('e')}")
            tag = "【池内！】" if n in POOL else "(L1)"
            print(f"  {n:12s} {tag} {nd} 条不同")
            if n in POOL:
                total_diff += nd
        print(f"\n池内逐字差异总数 = {total_diff} " + ("✅ 零回归" if total_diff == 0 else "❌ 有回归"))
