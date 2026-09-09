# -*- coding: utf-8 -*-
"""qwen_fc_test.py — Qwen3-0.6B 原生 function calling 遵循度测试 (R2 路线第一步)

用 lyco tools_openai.json 的 lyv_knowledge/vnn_identify 工具定义,
测 Qwen3-0.6B-Instruct 能否:
  1. 需要知识库时正确发 lyv_knowledge 调用 (query 参数填对)
  2. 识图场景正确发 vnn_identify 调用
  3. 闲聊时不乱发工具
"""
import json
import re

TOOLS = [
    {"type": "function", "function": {
        "name": "lyv_knowledge",
        "description": "查询视频知识库: 问怎么做某操作, 返回带时间戳的视频切片+关键帧+OCR验证文字",
        "parameters": {"type": "object", "properties": {
            "query": {"type": "string", "description": "想学的操作, 如 新建rust项目"},
            "pack": {"type": "string", "description": "知识包路径, 如 demo_pack"},
        }, "required": ["query"]}}},
    {"type": "function", "function": {
        "name": "vnn_identify",
        "description": "OCR失败时启用内部识图神经网络, 特征激活式对图片打分描述",
        "parameters": {"type": "object", "properties": {
            "image": {"type": "string", "description": "图片路径"},
        }, "required": ["image"]}}},
]

CASES = [
    # (用户输入, 期望调用的工具, 期望参数子串或 None)
    ("怎么新建 rust 项目", "lyv_knowledge", "rust"),
    ("帮我看看这张截图里是什么", "vnn_identify", None),
    ("cargo new 之后要做什么", "lyv_knowledge", "cargo"),
    ("你好呀", None, None),           # 闲聊不应发工具
    ("今天天气怎么样", None, None),   # 知识库外的闲聊
]

def build_prompt(q, tok):
    # Qwen3: enable_thinking=False 走非思考模式(工具调用不需要 think), 否则 200 token 全耗在 <think> 里
    # tokenize=False: 拿到的是字符串模板(我们手动 tokenize); Qwen3 新版 transformers 返回 str
    return tok.apply_chat_template(
        [{"role": "system", "content": "You are lyco, a helpful assistant. You can call tools."},
         {"role": "user", "content": q}],
        tools=TOOLS, add_generation_prompt=True, enable_thinking=False,
        tokenize=False)

def parse_call(text):
    m = re.search(r"<tool_call>\s*(\{.*?\})\s*</tool_call>", text, re.S)
    if m:
        try:
            return json.loads(m.group(1))
        except Exception:
            return None
    # 容错: 裸 JSON
    m = re.search(r'\{\s*"name"\s*:\s*"(\w+)"[^}]*\}', text)
    if m:
        return {"name": m.group(1)}
    return None

def main():
    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer
    model_id = "Qwen/Qwen3-0.6B"
    tok = AutoTokenizer.from_pretrained(model_id)
    model = AutoModelForCausalLM.from_pretrained(
        model_id, torch_dtype=torch.float16, device_map="cuda")
    print("loaded", model_id, flush=True)

    hits, total = 0, 0
    for q, expect_tool, expect_arg in CASES:
        total += 1
        prompt = build_prompt(q, tok)
        inputs = tok(prompt, return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=220, do_sample=False,
                             temperature=None, top_p=None, top_k=None)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False)
        call = parse_call(text)
        ok = False
        detail = ""
        if expect_tool is None:
            ok = call is None
            detail = "无调用(正确)" if ok else f"误调用 {call}"
        else:
            if call and call.get("name") == expect_tool:
                args = json.dumps(call.get("arguments", {}), ensure_ascii=False)
                ok = expect_arg is None or expect_arg in args
                detail = f"调用 {call.get('name')} args={args}"
            else:
                detail = f"FAIL got={call}"
        hits += ok
        print(f"[{'PASS' if ok else 'FAIL'}] {q} -> {detail}", flush=True)

    print(f"\nFC 遵循度: {hits}/{total} = {hits/total:.0%}")
    with open("/workspace/fc_test_result.json", "w") as f:
        json.dump({"model": "Qwen3-0.6B", "hits": hits, "total": total,
                   "rate": hits / total}, f)

if __name__ == "__main__":
    main()
