# -*- coding: utf-8 -*-
"""session8b_test.py — 单 kernel 会话内: vLLM 起 8B → 等加载 → FC 测试 → 输出结果"""
import json
import subprocess
import time
import urllib.request

def main():
    # 1. 起 server
    p = subprocess.Popen(
        ["bash", "/workspace/start_8b_vllm.sh"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        start_new_session=True)
    print("vLLM pid:", p.pid, flush=True)

    # 2. 等加载 (最多 200s)
    ok = False
    for i in range(100):
        try:
            r = urllib.request.urlopen("http://127.0.0.1:8081/v1/models", timeout=3)
            ok = True
            print(f"READY at {i*2}s", flush=True)
            break
        except Exception:
            time.sleep(2)
    if not ok:
        print("LOAD_TIMEOUT")
        return

    # 3. FC 测试
    body = json.dumps({
        "messages": [
            {"role": "user", "content": "怎么新建 rust 项目"}],
        "tools": [{
            "type": "function",
            "function": {
                "name": "lyv_knowledge",
                "description": "查询视频知识库",
                "parameters": {
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                    "required": ["query"]}}}],
        "max_tokens": 150,
    }).encode("utf-8")
    req = urllib.request.Request(
        "http://127.0.0.1:8081/v1/chat/completions", data=body, method="POST")
    d = json.loads(urllib.request.urlopen(req, timeout=60).read())
    msg = d["choices"][0]["message"]
    tc = msg.get("tool_calls")
    print("FC:", json.dumps(tc, ensure_ascii=False)[:200] if tc else
          "NONE: " + repr(msg.get("content", ""))[:80], flush=True)
    p.terminate()

if __name__ == "__main__":
    main()
