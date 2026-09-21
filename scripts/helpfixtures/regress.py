"""固定口径跑 help-parse 回归 + L1 探针。

口径（唯一）: --include-writes --top 9999 --json
  · total_parsed = 全部解析出的动作数（含写操作）—— **这是回归比对的主数字**
  · readonly     = --json 里的 readonly 字段（= 过滤写操作后的 acts.len()）

用法: python regress.py [--baseline]
  --baseline : 把当前读数写进 baseline.json（用于后续比对）
  无参数      : 与 baseline.json 比对，打印 delta
"""
import json, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
FIX = os.path.join(HERE, "fixtures")
BASE = os.path.join(HERE, "baseline.json")
LYCORE = "D:/Code/lyco_agent/lycore/target/debug/lycore.exe"

# 池内（回归基线，不能动）+ L1（外部陌生，越大越好）
POOL = ["docker", "git", "cargo", "npm", "kubectl", "jq"]
L1 = ["fd", "rg", "hyperfine", "zoxide", "bat", "starship", "oha"]


def run_one(name):
    p = os.path.join(FIX, name + ".txt")
    if not os.path.exists(p):
        return None
    r = subprocess.run([LYCORE, "help-parse", "--cli", name, "--help-text", p,
                        "--top", "9999", "--include-writes", "--json"],
                       capture_output=True, text=True, encoding="utf-8",
                       errors="replace", timeout=60)
    out = (r.stdout or "").strip()
    try:
        j = json.loads(out)
    except Exception:
        return {"err": f"rc={r.returncode} {out[:120]} {r.stderr[:120]}"}
    acts = j.get("actions") or []
    return {
        "total": j.get("total_parsed"),
        "readonly": j.get("readonly"),
        "returned": len(acts),
        # 前 8 条命令文本：**逐字比对**用（条数相同但内容变了也要能发现）
        "head": [a.get("cmd") or a.get("full_cmd") or str(a)[:60] for a in acts[:8]],
    }


def main():
    names = POOL + L1
    cur = {}
    for n in names:
        r = run_one(n)
        if r is None:
            cur[n] = None
            continue
        cur[n] = r

    if "--baseline" in sys.argv:
        with open(BASE, "w", encoding="utf-8") as f:
            json.dump(cur, f, ensure_ascii=False, indent=1)
        print(f"[baseline] 写入 {BASE}")
        for n in names:
            r = cur[n]
            if r and "err" not in r:
                print(f"  {n:12s} total={r['total']:4d} readonly={r['readonly']:4d}")
        return

    old = {}
    if os.path.exists(BASE):
        old = json.load(open(BASE, encoding="utf-8"))

    print(f"{'cli':12s} {'组':>4s} {'旧total':>8s} {'新total':>8s} {'delta':>7s}  head8")
    bad = []
    for n in names:
        r = cur[n]
        grp = "池内" if n in POOL else "L1"
        if r is None:
            print(f"{n:12s} {grp:>4s}        --       MISS  (无样本)")
            continue
        if "err" in r:
            print(f"{n:12s} {grp:>4s}   ERROR  {r['err']}")
            bad.append(n)
            continue
        o = old.get(n) or {}
        ot = o.get("total")
        dt = "" if ot is None else f"{r['total'] - ot:+d}"
        flag = ""
        if ot is not None and r["total"] != ot:
            flag += "  ⚠️条数变了!"
            if n in POOL:
                flag += "【池内回归】"
            bad.append(n)
        if ot is not None and o.get("head") != r.get("head"):
            flag += "  ⚠️head内容变了!"
            if n in POOL:
                bad.append(n)
        print(f"{n:12s} {grp:>4s} {str(ot):>8s} {r['total']:>8d} {dt:>7s}{flag}")
    if bad:
        print(f"\n❌ 有回归/错误: {bad}")
        sys.exit(1)
    print("\n✅ 池内零回归")


if __name__ == "__main__":
    main()
