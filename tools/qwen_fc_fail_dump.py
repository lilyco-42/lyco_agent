# -*- coding: utf-8 -*-
"""qwen_fc_fail_dump.py — 打印失败 case 的原始输出"""
import json
import torch
from transformers import AutoModelForCausalLM, AutoTokenizer
import sys
sys.path.insert(0, "/workspace")
import importlib.util
spec = importlib.util.spec_from_file_location("qft", "/workspace/qwen_fc_test.py")
qft = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qft)

tok = AutoTokenizer.from_pretrained("Qwen/Qwen3-0.6B")
model = AutoModelForCausalLM.from_pretrained(
    "Qwen/Qwen3-0.6B", torch_dtype=torch.float16, device_map="cuda")

for q in ["帮我看看这张截图里是什么", "cargo new 之后要做什么"]:
    prompt = qft.build_prompt(q, tok)
    inputs = tok(prompt, return_tensors="pt", add_special_tokens=False).to("cuda")
    out = model.generate(**inputs, max_new_tokens=400, do_sample=False)
    text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False)
    print("===", q)
    print(text[:280])
    print()
