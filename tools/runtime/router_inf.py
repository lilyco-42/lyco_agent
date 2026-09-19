# -*- coding: utf-8 -*-
"""router_inf.py — 路由器真推理演示 (v5 权重, 零 API 开销)"""
import torch
from transformers import AutoModelForCausalLM, AutoTokenizer

tok = AutoTokenizer.from_pretrained("/workspace/qwen3_router_v1")
model = AutoModelForCausalLM.from_pretrained(
    "/workspace/qwen3_router_v1", torch_dtype=torch.bfloat16, device_map="cuda")
SYS = ("你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。"
       "只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。")

for q in ["帮我打开台灯", "把蓝灯关了", "cpu 多少度", "风扇调到 180",
          "讲个笑话", "看看绿灯", "让蓝灯闪 3 下"]:
    prompt = tok.apply_chat_template(
        [{"role": "system", "content": SYS}, {"role": "user", "content": q}],
        tokenize=False, add_generation_prompt=True, enable_thinking=False)
    ids = tok(prompt, return_tensors="pt", add_special_tokens=False).to("cuda")
    out = model.generate(**ids, max_new_tokens=48, do_sample=False,
                         pad_token_id=tok.eos_token_id)
    cmd = tok.decode(out[0][ids["input_ids"].shape[1]:],
                     skip_special_tokens=True).strip()
    print(f"  {q!r} -> {cmd!r}", flush=True)
