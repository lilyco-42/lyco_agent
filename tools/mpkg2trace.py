#!/usr/bin/env python3
"""mpkg2trace —— 把 mpkg（记忆包）转成 trace 事件流（ndjson），供 pigma 播放

有了它，**任何记忆包都能变成一段可播放的"过程"**：
  mpkg.steps  →  trace.tool 事件（+ 真执行则得真实 ok/revert）
  mpkg.intent →  trace.prompt
对齐 `lycore/src/trace.rs` 的 5 种事件协议（prompt/tool/code/revert/final）。

用法:
  mpkg2trace <pkg-dir> [-o trace.ndjson] [--run]
    --run  真执行 steps/verify（在临时 work 目录），得到真实 ok 与 revert（试错也入 trace）
           默认不执行: 只生成骨架（全部 ok），用于演示
"""
import argparse
import json
import os
import subprocess
import tempfile
import time


def main() -> int:
    ap = argparse.ArgumentParser(prog="mpkg2trace")
    ap.add_argument("pkg_dir")
    ap.add_argument("-o", "--out", default="trace.ndjson")
    ap.add_argument("--run", action="store_true", help="真执行 steps（得真实 ok/revert）")
    a = ap.parse_args()

    pkg = os.path.abspath(a.pkg_dir)
    manifest = json.load(open(os.path.join(pkg, "mpkg.json"), encoding="utf-8"))
    work = tempfile.mkdtemp(prefix="mpkg2trace-")

    ev = []
    i = 0
    t0 = time.time()

    def emit(kind: str, **kw):
        nonlocal i
        i += 1
        ev.append({"kind": kind, "i": i, "t_ms": int((time.time() - t0) * 1000), **kw})

    def subst(cmd: str) -> str:
        return cmd.replace("{{pkg}}", pkg).replace("{{work}}", work)

    def run(cmd: str):
        return subprocess.run(["bash", "-c", cmd], cwd=work,
                              capture_output=True, text=True, timeout=180)

    emit("prompt", text=manifest.get("intent", manifest.get("name", "")))

    fail = 0
    for n, s in enumerate(manifest.get("steps", []), 1):
        cmd = subst(s["run"])
        ok = True
        if a.run:
            r = run(cmd)
            ok = r.returncode == 0
            if not ok:
                fail += 1
        emit("tool", name="step", arg=cmd, ok=ok)
        if not ok:
            why = (r.stderr or r.stdout or "").strip().splitlines()
            emit("revert", why=f"step {n} 失败(exit={r.returncode}): {(why[-1] if why else '')[:140]}")

    for v in manifest.get("verify", []):
        cmd = subst(v)
        ok = True
        if a.run:
            r = run(cmd)
            ok = r.returncode == 0
            if not ok:
                fail += 1
        emit("tool", name="verify", arg=cmd, ok=ok)

    emit("final", answer=f"{manifest.get('name','?')} —— " + ("全部通过" if not fail else f"{fail} 步失败"))

    with open(a.out, "w", encoding="utf-8") as f:
        for e in ev:
            f.write(json.dumps(e, ensure_ascii=False) + "\n")
    print(f"[mpkg2trace] {len(ev)} 事件 → {a.out}  (pkg={manifest.get('name')}, run={a.run})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
