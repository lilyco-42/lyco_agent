#!/usr/bin/env python3
"""trace2mpkg —— 把 agent 执行 trace 收尾打成 mpkg（记忆包）

trace (ndjson) → mpkg 包目录 → `mpkg.py build` → `cache-node verify` → attestation

映射（对齐 `lystack/spec/mpkg-v0.md`）:
  prompt.text  → intent（搜索主键）
  tool  事件   → steps[]（run=arg, expect.exit=0）  ← 铁律: 只允许真实 CLI 进程
  code  事件   → atoms[]（kind=text，记 file + diff）
  revert 事件  → provenance.attempts[]（**试错 = 编曲注记**，spec §8 的 openvibe 钩子）
  final        → 收束

用法:
  trace2mpkg <trace.ndjson> [-o <pkg-dir>] [--name <name>]
"""
import argparse
import json
import os
import re

# shell 内建/占位伪工具: 不能进 requirements.tools (cache-node 会 which 它们)
SHELL_BUILTINS = {
    "mkdir", "cp", "mv", "rm", "cd", "echo", "test", "cat", "ls", "printf", "touch",
    "true", "false", "step", "verify", "shell_exec",
}

MANIFEST = {
    "mpkg": "0.1",
    "name": "trace-pkg",
    "version": "0.1.0",
    "intent": "",
    "author": "agent:lyco_agent",
    "requirements": {"tools": []},
    "steps": [],
    "verify": [],
    "atoms": [],
    "tags": ["trace"],
    "license": "MIT",
}


def kebab(s: str) -> str:
    s = re.sub(r"[^\w\s-]", "", s, flags=re.UNICODE)
    s = re.sub(r"[\s_]+", "-", s).strip("-").lower()
    return s or "trace-pkg"


def main() -> int:
    ap = argparse.ArgumentParser(prog="trace2mpkg")
    ap.add_argument("trace")
    ap.add_argument("-o", "--out", default="")
    ap.add_argument("--name", default="")
    ap.add_argument(
        "--from-pkg",
        default="",
        help="原始 mpkg 目录: 把它的 artifacts/ atoms/ 带进新包 "
        "(步骤常引用 {{pkg}}/artifacts/..., 不带过来回放会失败)",
    )
    a = ap.parse_args()

    events = []
    with open(a.trace, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                try:
                    events.append(json.loads(line))
                except json.JSONDecodeError:
                    continue
    if not events:
        print("[trace2mpkg] trace 为空")
        return 1

    intent = ""
    steps, verify, atoms, attempts = [], [], [], []
    tools = []

    for i, e in enumerate(events, 1):
        kind = e.get("kind")
        if kind == "prompt":
            intent = e.get("text", "")
        elif kind == "tool":
            name = e.get("name", "")
            arg = e.get("arg", "")
            ok = bool(e.get("ok"))
            # 优先用 tpl(带 {{pkg}}/{{work}} 占位符, 可移植); 没有才退回绝对路径
            entry = {"run": e.get("tpl") or arg, "expect": {"exit": 0}}
            if "about" in e and e["about"]:
                entry["about"] = e["about"]
            # name=verify 的事件按规范进 verify[], 其余进 steps[]
            if name == "verify":
                # 规范里 verify[] 是**字符串数组** (官方骨架 "verify": ["echo ok"]);
                # 发对象会让 cache-node 解析出空 cmd —— 实测踩坑
                verify.append(entry["run"])
            else:
                steps.append(entry)
            # 工具名要从命令推导, 不能用 trace 的 name 字段
            # (name 可能是 "step"/"verify" 这种事件标签 → 写进 requirements.tools 会让
            #  cache-node 去 which step 而失败 —— 实测踩坑)
            first = (e.get("tpl") or arg).split()[0] if (e.get("tpl") or arg) else ""
            exe = os.path.basename(first)
            if exe and exe not in SHELL_BUILTINS and exe not in tools:
                tools.append(exe)
        elif kind == "code":
            p = f"atoms/code-{i:04d}.md"
            atoms.append(
                {
                    "kind": "text",
                    "path": p,
                    "title": e.get("file", "code"),
                    "ref": f"trace:{i}",
                }
            )
        elif kind == "revert":
            attempts.append({"step": len(steps), "why": e.get("why", "")})
        elif kind == "final":
            atoms.append(
                {
                    "kind": "text",
                    "path": "atoms/final.md",
                    "title": "最终回答",
                    "ref": f"trace:{i}",
                }
            )

    name = a.name or kebab(intent)[:40]
    out = a.out or name
    os.makedirs(os.path.join(out, "atoms"), exist_ok=True)

    m = dict(MANIFEST)
    m["name"] = name
    m["intent"] = intent or name
    m["steps"] = steps
    m["verify"] = verify
    m["atoms"] = atoms
    m["requirements"]["tools"] = [{"name": t} for t in tools]
    if attempts:
        # spec §8 provenance 钩子: prompt 谱系与每步溯源 —— 试错即"编曲注记"
        m["provenance"] = {"source": "lycore-trace", "attempts": attempts}

    # 落 atoms 文件
    for i, e in enumerate(events, 1):
        if e.get("kind") == "code":
            with open(os.path.join(out, f"atoms/code-{i:04d}.md"), "w", encoding="utf-8") as f:
                f.write(f"# {e.get('file','code')}\n\n```\n{e.get('diff','')}\n```\n")
        elif e.get("kind") == "final":
            with open(os.path.join(out, "atoms/final.md"), "w", encoding="utf-8") as f:
                f.write(f"# 最终回答\n\n{e.get('answer','')}\n")

    with open(os.path.join(out, "mpkg.json"), "w", encoding="utf-8") as f:
        json.dump(m, f, ensure_ascii=False, indent=2)

    # 带过来原始包的产物 (否则 {{pkg}}/artifacts/... 回放时找不到)
    if a.from_pkg and os.path.isdir(a.from_pkg):
        import shutil

        for sub in ("artifacts", "atoms"):
            src = os.path.join(a.from_pkg, sub)
            if os.path.isdir(src):
                dst = os.path.join(out, sub)
                if os.path.isdir(dst):
                    shutil.rmtree(dst)
                shutil.copytree(src, dst)
        print(f"[trace2mpkg] 已从 {a.from_pkg} 带入 artifacts/atoms")

    print(
        f"[trace2mpkg] {len(events)} 事件 → {out}/"
        f"  steps={len(steps)} verify={len(verify)} atoms={len(atoms)}"
        f" attempts={len(attempts)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
