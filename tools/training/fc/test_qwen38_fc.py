# -*- coding: utf-8 -*-
"""test_qwen38_fc.py — Qwen3.8-27B 原生 FC 遵循度测试 (远端执行)"""
import json
import urllib.request

TOOLS = [{
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

CASES = [
    ("怎么新建 rust 项目", "lyv_knowledge", "rust"),
    ("帮我看看这张截图里是什么", None, None),
    ("cargo new 之后要做什么", "lyv_knowledge", "cargo"),
    ("你好呀", None, None),
]


def run_case(question, expect_tool, expect_arg):
    body = json.dumps({
        "messages": [
            {"role": "system", "content": "You are lyco. 操作类问题先用工具查询知识库."},
            {"role": "user", "content": question},
        ],
        "tools": TOOLS,
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
        return (tc is None), f"{'NO_CALL(正确)' if tc is None else '误调用'}"
    if tc:
        fn = tc[0]["function"]
        args = fn.get("arguments", "")
        ok = fn["name"] == expect_tool and (expect_arg is None or expect_arg in str(args))
        return ok, f"{fn['name']} args={str(args)[:60]}"
    return False, "该调不调"


def main():
    hits = 0
    for q, et, ea in CASES:
        ok, detail = run_case(q, et, ea)
        hits += ok
        print(f"[{'PASS' if ok else 'FAIL'}] {q} -> {detail}", flush=True)
    print(f"Qwen3.8-27B 原生 FC 遵循度: {hits}/{len(CASES)}", flush=True)


if __name__ == "__main__":
    main()
