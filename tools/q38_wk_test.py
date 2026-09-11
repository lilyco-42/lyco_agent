# -*- coding: utf-8 -*-
"""q38_wk_test.py — 单 kernel 会话: 8B CPU 世界知识问答 (起+等+测一体)"""
import json
import subprocess
import time
import urllib.request


def main():
    p = subprocess.Popen(
        ["bash", "-c",
         "cd /workspace && exec ./llama-b10883/llama-server -m qwen3_8b_q4.gguf "
         "--port 8081 --jinja -c 4096 -t 28 > llama8b.log 2>&1"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        start_new_session=True)
    print("pid:", p.pid, flush=True)

    ok = False
    for i in range(120):
        try:
            urllib.request.urlopen("http://127.0.0.1:8081/health", timeout=3)
            ok = True
            print(f"READY at {i*2}s", flush=True)
            break
        except Exception:
            time.sleep(2)
    if not ok:
        print("LOAD_TIMEOUT")
        return

    for q in ["天空是什么颜色", "鱼会游泳吗", "太阳从哪边落下"]:
        body = json.dumps({
            "messages": [{"role": "user", "content": q}],
            "max_tokens": 40, "temperature": 0.7}).encode("utf-8")
        req = urllib.request.Request(
            "http://127.0.0.1:8081/v1/chat/completions", data=body, method="POST")
        d = json.loads(urllib.request.urlopen(req, timeout=60).read())
        print(f"{q}: {d['choices'][0]['message']['content'][:50]}", flush=True)

    p.terminate()


if __name__ == "__main__":
    main()
