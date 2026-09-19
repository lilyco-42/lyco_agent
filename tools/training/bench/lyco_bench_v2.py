# -*- coding: utf-8 -*-
"""lyco_bench_v2.py — 评测集 v2: 检索质量分级 + 泛化盲区量化

v1 (15 case) 的升级:
  1. D1 拆分: D1a 命中率 (有结果) / D1b 意图正确率 (intent 对) — FTS 兜底
     命中≠意图正确, 混在一起会高估
  2. 新增 D4 泛化: 同义改写/口语化/错别字/英文 四类变体的意图保持率
     — 量化"领域词典硬编码"的泛用性缺口 (CBAM A1 配置化的前置数据)
  3. 期望值对齐包内真实 intent (v1 教训)

评测对象: lycore serve /ask (纯检索模式)。
"""
import json
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

PACK = "smoke/pack_final"
PORT = 8667
BASE = f"http://127.0.0.1:{PORT}"

# ---------- D1: 知识库内, 意图明确 ----------
D1_CASES = [
    ("怎么新建 rust 项目", {"rust.project.create"}),
    ("帮我运行这个程序", {"rust.project.run"}),
    ("进入项目目录的命令", {"fs.chdir", "cd.hello"}),
    ("如何创建 rust 项目", {"rust.project.create"}),
    ("cargo run 怎么用", {"rust.project.run"}),
    ("如何初始化项目", {"rust.project.create"}),
]
# ---------- D3: 知识库外 ----------
D3_CASES = [
    "怎么配置 nginx 反向代理", "怎么做红烧肉", "怎么修汽车发动机",
    "如何报考驾照", "怎么写一首诗",
]
# ---------- D4: 泛化变体 (期望 intent 集合) ----------
D4_CASES = [
    # 同义改写
    ("建立一个新的Rust工程", {"rust.project.create"}, "同义改写"),
    ("初始化一个Rust项目", {"rust.project.create"}, "同义改写"),
    ("项目跑起来的方法", {"rust.project.run"}, "同义改写"),
    # 口语化
    ("我想写个Rust程序第一步干啥", {"rust.project.create"}, "口语化"),
    ("程序怎么让他动起来", {"rust.project.run"}, "口语化"),
    # 错别字
    ("怎么新建rust项木", {"rust.project.create"}, "错别字"),
    # 英文
    ("how to create a rust project", {"rust.project.create"}, "英文"),
    ("how to run cargo", {"rust.project.run"}, "英文"),
]

def start_server():
    proc = subprocess.Popen(
        ["./lycore/target/release/lycore", "serve", "--pack", PACK,
         "--port", str(PORT)],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1.5)
    return proc

def ask(question):
    body = json.dumps({"question": question}).encode("utf-8")
    req = urllib.request.Request(f"{BASE}/ask", data=body, method="POST")
    return json.loads(urllib.request.urlopen(req, timeout=15).read())

def stop(proc):
    proc.terminate()
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        proc.kill()

def main():
    proc = start_server()
    r = {"d1a_hit": [], "d1b_intent": [], "d3_honesty": [], "d4_general": []}
    try:
        for q, expect in D1_CASES:
            resp = ask(q)
            r["d1a_hit"].append({"q": q, "ok": resp.get("route") == "retrieval"})
            r["d1b_intent"].append({
                "q": q, "ok": resp.get("intent", "") in expect,
                "got": resp.get("intent")})
        for q in D3_CASES:
            resp = ask(q)
            r["d3_honesty"].append({
                "q": q, "ok": resp.get("route") == "learning_queue"})
        for q, expect, cat in D4_CASES:
            resp = ask(q)
            got = resp.get("intent", "")
            r["d4_general"].append({
                "q": q, "cat": cat, "ok": got in expect, "got": got})
    finally:
        stop(proc)

    def rate(cases):
        return sum(1 for c in cases if c["ok"]) / len(cases) if cases else 0.0

    d1a = rate(r["d1a_hit"])
    d1b = rate(r["d1b_intent"])
    d3 = rate(r["d3_honesty"])
    d4 = rate(r["d4_general"])
    # 泛化按类细分
    cats = {}
    for c in r["d4_general"]:
        cats.setdefault(c["cat"], []).append(c["ok"])
    cat_rates = {k: f"{sum(v)}/{len(v)}" for k, v in cats.items()}

    summary = {
        "D1a_命中率": d1a, "D1b_意图正确率": d1b,
        "D3_诚实降级": d3, "D4_泛化": round(d4, 3),
        "D4_分类明细": cat_rates,
        "总分(三维)": round((d1a + d1b + d3) / 3 * 100, 1),
    }
    print(json.dumps(summary, ensure_ascii=False, indent=1))
    # 失败明细
    for c in r["d4_general"]:
        if not c["ok"]:
            print(f"  D4 FAIL [{c['cat']}] {c['q']} -> {c['got']}")
    json.dump({"summary": summary, "detail": r}, open("bench_v2.json", "w",
              encoding="utf-8"), ensure_ascii=False, indent=1)
    return 0

if __name__ == "__main__":
    sys.exit(main())
