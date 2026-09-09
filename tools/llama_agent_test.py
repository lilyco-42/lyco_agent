# -*- coding: utf-8 -*-
"""llama_agent_test.py — llama.cpp GRPO 模型 + lyv 真实检索的 agent loop (远端)"""
import json
import urllib.request

import lyv

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


def chat(messages):
    body = json.dumps({
        "messages": messages,
        "tools": TOOLS,
        "max_tokens": 250,
        "chat_template_kwargs": {"enable_thinking": False},
    }).encode("utf-8")
    req = urllib.request.Request(
        "http://127.0.0.1:8081/v1/chat/completions", data=body, method="POST")
    d = json.loads(urllib.request.urlopen(req, timeout=110).read())
    msg = d["choices"][0]["message"]
    tc = msg.get("tool_calls")
    if tc:
        fn = tc[0]["function"]
        args = fn["arguments"]
        if isinstance(args, str):
            args = json.loads(args)
        return ("call", fn["name"], args)
    return ("answer", msg.get("content", ""), None)


def main():
    messages = [
        {"role": "system", "content": "You are lyco. 操作类问题先用工具查询知识库."},
        {"role": "user", "content": "怎么新建 rust 项目"},
    ]
    for round_i in range(1, 4):
        kind, name_or_text, args = chat(messages)
        if kind == "answer":
            print(f"[{round_i}] FINAL: {name_or_text[:80]}")
            break
        print(f"[{round_i}] CALL: {name_or_text} {args}")
        result = lyv.lookup("lyv_tmp/pack_final", args.get("query", ""))
        if result:
            payload = {"ok": True, "text": result["text"],
                       "clip": f"{result['t0']}-{result['t1']}s",
                       "keyframe": result["frame"]}
        else:
            payload = {"ok": False, "error": "NO_HIT: 还没学会"}
        print(f"[{round_i}] RESULT: {json.dumps(payload, ensure_ascii=False)[:110]}")
        messages.append({"role": "assistant", "content": json.dumps(
            {"tool_call": {"name": name_or_text, "arguments": args}}, ensure_ascii=False)})
        messages.append({"role": "tool", "content": json.dumps(payload, ensure_ascii=False)})


if __name__ == "__main__":
    main()
