# -*- coding: utf-8 -*-
"""q38_session.py — 单 kernel 会话全流程: 起 Qwen3.8-27B + 等加载 + FC 测试
(kernel 收割周期内完成全部工作, 规避 server 被杀)"""
import json
import subprocess
import urllib.request

def wait_loaded():
    for i in range(120):
        try:
            log = open("/workspace/llama38.log", errors="replace").read()
            if "model loaded" in log.split("llama_server:")[-1] or "model loaded" in log:
                return True
        except Exception:
            pass
        import time
        time.sleep(2)
    return False

def fc_case(question, expect_tool, expect_arg):
    tools = [{
        "type": "function",
        "function": {
            "name": "lyv_knowledge",
            "description": "查询视频知识库: 问怎么做某操作, 返回带时间戳的视频切片+关键帧+OCR验证文字",
            "parameters": {
                "type": "object",
                "properties": {"query": {"type": "string", "description": "想学的操作"}},
                "required": ["query"],
            },
        },
    }]
    body = json.dumps({
        "messages": [
            {"role": "system", "content": "You are lyco. 操作类问题先用工具查询知识库."},
            {"role": "user", "content": question},
        ],
        "tools": tools,
        "max_tokens": 200,
        "temperature": 0.7,
        "chat_template_kwargs": {"enable_thinking": False},
    }).encode("utf-8")
    req = urllib.request.Request(
        "http://127.0.0.1:8081/v1/chat/completions", data=body, method="POST")
    d = json.loads(urllib.request.urlopen(req, timeout=110).read())
    msg = d["choices"][0]["message"]
    tc = msg.get("tool_calls")
    if expect_tool is None:
        return tc is None, "NO_CALL(正确)" if tc is None else "误调用"
    if tc:
        fn = tc[0]["function"]
        ok = fn["name"] == expect_tool and (expect_arg is None or expect_arg in str(fn.get("arguments", "")))
        return ok, f"{fn['name']} {str(fn.get('arguments',''))[:50]}"
    return False, "该调不调"

def main():
    # 1. 起 server
    subprocess.run(
        ["bash", "-c",
         "cd /workspace && pkill -f llama-server 2>/dev/null; sleep 1; "
         "setsid ./llama-b10883/llama-server -m qwen38_27b_q4.gguf --port 8081 --jinja -c 8192 -t 28 "
         "> llama38.log 2>&1 &"],
        timeout=15)
    # 2. 等 model loaded
    import time
    loaded = False
    for _ in range(90):
        try:
            if "model loaded" in open("/workspace/llama38.log", errors="replace").read():
                loaded = True
                break
        except Exception:
            pass
        time.sleep(2)
    if not loaded:
        print("MODEL_LOAD_TIMEOUT")
        return
    # 3. FC 测试 (同 kernel 会话内)
    cases = [
        ("怎么新建 rust 项目", "lyv_knowledge", "rust"),
        ("帮我看看这张截图里是什么", None, None),
        ("cargo new 之后要做什么", "lyv_knowledge", "cargo"),
        ("你好呀", None, None),
    ]
    hits = 0
    for q, et, ea in cases:
        try:
            ok, detail = fc_case(q, et, ea)
        except Exception as e:
            ok, detail = False, f"ERR {str(e)[:40]}"
        hits += ok
        print(f"[{'PASS' if ok else 'FAIL'}] {q} -> {detail}", flush=True)
    print(f"Qwen3.8-27B 原生 FC: {hits}/{len(cases)}", flush=True)

if __name__ == "__main__":
    main()
