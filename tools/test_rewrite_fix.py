# -*- coding: utf-8 -*-
"""test_rewrite_fix.py — rewrite v4 模型修复口语化意图歧义验证

bench v2 唯一残留: '我想写个Rust程序第一步干啥' → 词典判 run (错, 应 create)
方案: rewrite 模型改写 → '新建 rust 项目' → lookup 命中 create
"""
import json
import subprocess
import sys
import urllib.request

sys.path.insert(0, "/workspace")
import lyv

BASE = "http://127.0.0.1:8082"
PACK = "/workspace/lyv_tmp/pack_merged"


def rewrite_with_v4(question):
    """用 rewrite v4 专训模型 (端口 8082) 改写"""
    body = json.dumps({
        "messages": [
            {"role": "system", "content": "你是检索查询改写器。把口语化问题改写成检索关键词(操作+对象), 只输出关键词, 10字以内。"},
            {"role": "user", "content": question},
        ],
        "max_tokens": 32,
        "temperature": 0.7,
        "chat_template_kwargs": {"enable_thinking": False},
    }).encode("utf-8")
    req = urllib.request.Request(f"{BASE}/v1/chat/completions", data=body, method="POST")
    d = json.loads(urllib.request.urlopen(req, timeout=60).read())
    return d["choices"][0]["message"]["content"].strip()


def main():
    # 1. 起 rewrite v4 server (端口 8082)
    p = subprocess.Popen(
        ["bash", "-c",
         "cd /workspace && setsid ./llama-src/build-cuda/bin/llama-server "
         "-m qwen3_lyco_rewrite_v4_q4km.gguf --port 8082 --jinja -c 2048 -ngl 99 "
         "> llama_rw4.log 2>&1"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    # 但 27B Q4 gguf 已删, rewrite v4 是 HF 格式不是 GGUF...
    # 检查: qwen3_lyco_rewrite_v4 目录在 (HF 格式), llama.cpp 加载不了
    # → 回退: 用 transformers 直接推理 (慢但可行, A10 GPU)
    print("llama.cpp 不支持 HF 格式, 改用 transformers 推理")
    from transformers import AutoModelForCausalLM, AutoTokenizer
    import torch
    tok = AutoTokenizer.from_pretrained("/workspace/qwen3_lyco_rewrite_v4")
    model = AutoModelForCausalLM.from_pretrained(
        "/workspace/qwen3_lyco_rewrite_v4", torch_dtype=torch.bfloat16, device_map="cuda")
    p.terminate()

    def rewrite(question):
        prompt = f"你是检索查询改写器。把口语化问题改写成检索关键词(操作+对象), 只输出关键词, 10字以内。\n\n{question}"
        inputs = tok(prompt, return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=32, do_sample=False)
        return tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False).strip()

    # 2. 验证 bench v2 残留 case
    cases = [
        ("我想写个Rust程序第一步干啥", "rust.project.create"),
        ("程序怎么让他动起来", "rust.project.run"),
    ]
    for q, want in cases:
        rewritten = rewrite(q)
        result = lyv.lookup(PACK, rewritten)
        got = result["intent"] if result else "NO_HIT"
        ok = got == want
        print(f"[{'PASS' if ok else 'FAIL'}] {q[:20]} → 改写:{rewritten[:25]!r} → {got}")


if __name__ == "__main__":
    main()
