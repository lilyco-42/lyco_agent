# -*- coding: utf-8 -*-
"""rewrite_e2e_test.py — 两阶段检索端到端验证 (直查 → LLM 改写 → 二次检索)

通过 lycore serve 的 /ask? 不 — rewrite 在 lycore 库内, serve 还没接。
本测试直接验证改写链路的模型侧: GRPO Q4 模型能否把口语化查询改写成检索关键词。
"""
import json
import urllib.request

BASE = "http://127.0.0.1:8081"
REWRITE_PROMPT = "你是一个检索查询改写器。把用户的口语化问题改写成知识库检索关键词(操作+对象), 只输出关键词本身, 10字以内, 不要回答问题。"

CASES = [
    "我想写个Rust程序第一步干啥",
    "程序怎么让他动起来",
    "怎么新建rust项目",
]


def rewrite(question):
    body = json.dumps({
        "messages": [
            {"role": "system", "content": REWRITE_PROMPT},
            {"role": "user", "content": question},
        ],
        "max_tokens": 60,
        "temperature": 0.7,
        "chat_template_kwargs": {"enable_thinking": False},
    }).encode("utf-8")
    req = urllib.request.Request(f"{BASE}/v1/chat/completions", data=body, method="POST")
    d = json.loads(urllib.request.urlopen(req, timeout=60).read())
    return d["choices"][0]["message"]["content"].strip()


def main():
    for q in CASES:
        r = rewrite(q)
        print(f"{q}  →  {r[:40]!r}")


if __name__ == "__main__":
    main()
