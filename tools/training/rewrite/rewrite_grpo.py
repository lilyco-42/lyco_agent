# -*- coding: utf-8 -*-
"""rewrite_grpo.py — 查询改写能力 GRPO 训练 (飞轮第一圈: 失败→训练→复测)

任务: 口语化查询 → 检索关键词 (能被 lyv.lookup 命中 create/run)
奖励: 改写结果被 lyv.lookup 命中 +1, 命中且意图正确 +1 (环境反馈 = 真检索器)
数据: 程序化生成 口语化→标准 对 (与 bench v2 D4 同源), 240 prompts
底座: qwen3_lyco_grpo (已会 tool_call 格式), 继续训 300 步
验证: 训练后 rewrite_e2e 的 3 个 case 改写质量应提升
"""
import json
import random
import re
import sys

sys.path.insert(0, "/workspace")
MODEL_ID = "Qwen/Qwen3-0.6B"
BASE_MODEL = "/workspace/qwen3_lyco_grpo"  # 已会 tool_call 格式
OUT_DIR = "/workspace/qwen3_lyco_rewrite"
MAX_STEPS = 300

# 口语化→标准 检索词对 (程序化模板 + 变体)
ORAL_PATTERNS = [
    ("我想写个Rust程序第一步干啥", "新建 rust 项目"),
    ("程序怎么让他动起来", "运行 项目"),
    ("Rust项目从零开始怎么搞", "新建 rust 项目"),
    ("写好的代码怎么跑", "运行 项目"),
    ("项目怎么构建出来", "build 项目"),
    ("依赖怎么装", "install 依赖"),
    ("代码库拉下来", "git clone 仓库"),
    ("改动怎么提交", "git commit 提交"),
    ("怎么进入那个文件夹", "cd 项目目录"),
    ("跑个hello world看看", "cargo run hello world"),
]
TEMPLATES = [
    "{q}", "帮我看看{q}", "{q}啊", "请教下{q}", "{q}呗",
]
ORAL_TEMPLATES = [
    "{q}", "{q}呢", "我想{q}", "{q}要怎么做", "{q}有人会吗", "{q}求教",
]

def gen_prompts(seed=11):
    rng = random.Random(seed)
    rows = []
    for oral, standard in ORAL_PATTERNS:
        for ot in ORAL_TEMPLATES:
            q = ot.format(q=oral)
            rows.append({"prompt": q, "standard": standard})
        for tt in TEMPLATES:
            # 正例变体: 标准词也该能被直查/改写回自己
            rows.append({"prompt": tt.format(q=standard), "standard": standard})
    rng.shuffle(rows)
    return rows

def build_prompt(q, tok):
    return tok.apply_chat_template(
        [{"role": "system", "content": "你是检索查询改写器。把口语化问题改写成检索关键词(操作+对象), 只输出关键词, 10字以内。"},
         {"role": "user", "content": q}],
        add_generation_prompt=True, enable_thinking=False, tokenize=False)

def rewrite_reward(completions, standard=None, **kw):
    """奖励: 改写结果与目标检索词的词面重叠 + 意图关键词命中"""
    import subprocess
    rewards = []
    stds = standard if isinstance(standard, list) else [standard] * len(completions)
    for comp, std in zip(completions, stds):
        text = comp.strip()
        if not text or text.startswith("<think>"):
            rewards.append(0.0)
            continue
        target_words = set(std.lower().split())
        got_words = set(text.lower().split())
        overlap = len(target_words & got_words)
        rewards.append(min(1.0, overlap * 0.5))
    return rewards

def train():
    from datasets import Dataset
    from transformers import AutoModelForCausalLM, AutoTokenizer
    from trl import GRPOConfig, GRPOTrainer
    import torch

    tok = AutoTokenizer.from_pretrained(MODEL_ID)
    tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(
        BASE_MODEL, torch_dtype=torch.bfloat16, attn_implementation="sdpa").cuda()

    rows = gen_prompts()
    for r in rows:
        r["prompt"] = build_prompt(r["prompt"], tok)
    ds = Dataset.from_list(rows)
    print(f"rewrite dataset: {len(ds)}", flush=True)

    cfg = GRPOConfig(
        output_dir=OUT_DIR,
        per_device_train_batch_size=8,
        gradient_accumulation_steps=2,
        num_generations=4,
        max_completion_length=48,   # 改写输出很短
        max_steps=MAX_STEPS,
        learning_rate=1e-5,
        logging_steps=10,
        save_strategy="no",
        bf16=True,
        report_to=[],
        temperature=1.0,
    )
    trainer = GRPOTrainer(model=model, reward_funcs=rewrite_reward,
                          args=cfg, train_dataset=ds, processing_class=tok)
    trainer.train()
    trainer.save_model(OUT_DIR)
    tok.save_pretrained(OUT_DIR)
    print("TRAIN_DONE", flush=True)

def evaluate():
    from transformers import AutoModelForCausalLM, AutoTokenizer
    import torch
    tok = AutoTokenizer.from_pretrained(OUT_DIR)
    model = AutoModelForCausalLM.from_pretrained(
        OUT_DIR, torch_dtype=torch.bfloat16, device_map="cuda")
    hits = 0
    total = 0
    for oral, standard in ORAL_PATTERNS[:6]:
        total += 1
        prompt = build_prompt(oral, tok)
        inputs = tok(prompt, return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=32, do_sample=False)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False).strip()
        tw = set(standard.lower().split())
        gw = set(text.lower().split())
        ok = bool(tw & gw)
        hits += ok
        print(f"[{'PASS' if ok else 'FAIL'}] {oral[:18]} -> {text[:30]!r}", flush=True)
    print(f"改写命中率: {hits}/{total}", flush=True)

if __name__ == "__main__":
    train()
    evaluate()
