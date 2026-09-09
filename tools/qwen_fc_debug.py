# -*- coding: utf-8 -*-
"""qwen_fc_debug.py — 打印 Qwen3-0.6B 对工具调用 prompt 的原始输出"""
import json
import torch
from transformers import AutoModelForCausalLM, AutoTokenizer

TOOLS = [
    {"type": "function", "function": {
        "name": "lyv_knowledge",
        "description": "查询视频知识库: 问怎么做某操作, 返回带时间戳的视频切片+关键帧+OCR验证文字",
        "parameters": {"type": "object", "properties": {
            "query": {"type": "string", "description": "想学的操作"},
            "pack": {"type": "string", "description": "知识包路径"},
        }, "required": ["query"]}}},
    {"type": "function", "function": {
        "name": "vnn_identify",
        "description": "OCR失败时启用内部识图神经网络对图片打分描述",
        "parameters": {"type": "object", "properties": {
            "image": {"type": "string", "description": "图片路径"},
        }, "required": ["image"]}}},
]

def main():
    tok = AutoTokenizer.from_pretrained("Qwen/Qwen3-0.6B")
    model = AutoModelForCausalLM.from_pretrained(
        "Qwen/Qwen3-0.6B", torch_dtype=torch.float16, device_map="cuda")
    tools_json = json.dumps(TOOLS, ensure_ascii=False)
    q = "怎么新建 rust 项目"
    prompt = ("<|im_start|>system\nYou are lyco. # Tools\n" + tools_json +
              "<|im_end|>\n<|im_start|>user\n" + q + "<|im_end|>\n<|im_start|>assistant\n")
    inputs = tok(prompt, return_tensors="pt").to("cuda")
    out = model.generate(**inputs, max_new_tokens=200, do_sample=False)
    text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False)
    open("/workspace/fc_raw.txt", "w", encoding="utf-8").write(text)
    print("SAVED, len", len(text))

if __name__ == "__main__":
    main()
