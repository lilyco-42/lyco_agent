# -*- coding: utf-8 -*-
"""rewrite_grpo_v2.py — 奖励函数语义化: 用真实检索器 lyv.lookup 当环境奖励

v1 问题: 词面重叠奖励只奖励"用了同样的词", 对"换词表达"泛化不足 (67%)
v2 方案: 环境 = lyv.sqlite 检索器。奖励 = 改写结果喂进真实检索器:
    命中且 intent == 期望   → +1.0   (检索器替代人工标注)
    命中但 intent 错        → +0.2   (召回不错位, 部分分)
    未命中                  →  0.0
这是 FEE 论文「环境反馈 > 人工奖励工程」的直接实现: 环境本身就是判定器。
"""
import json
import random
import sys

sys.path.insert(0, "/workspace")
import lyv  # 真实检索器 = 环境

MODEL_ID = "Qwen/Qwen3-0.6B"
BASE_MODEL = "/workspace/qwen3_lyco_rewrite"
OUT_DIR = "/workspace/qwen3_lyco_rewrite_v2"
MAX_STEPS = 300
PACK = "/workspace/lyv_tmp/pack_final"

ORAL_PATTERNS = [
    # (口语查询, 改写目标检索词, 期望 intent)
    ("我想写个Rust程序第一步干啥", "新建 rust 项目", "rust.project.create"),
    ("程序怎么让他动起来", "运行 项目", "rust.project.run"),
    ("Rust项目从零开始怎么搞", "新建 rust 项目", "rust.project.create"),
    ("写好的代码怎么跑", "运行 项目", "rust.project.run"),
    ("怎么进入那个文件夹", "cd 项目目录", "fs.chdir"),
    ("跑个hello world看看", "cargo run hello world", "rust.project.run"),
    ("项目文件建在哪了", "新建 rust 项目", "rust.project.create"),
    ("代码跑不起来怎么回事", "运行 项目", "rust.project.run"),
    ("依赖包怎么下载", "install 依赖", "py.pkg.install"),
    ("代码提交到仓库", "git commit 提交", "git.push"),
]
ORAL_TEMPLATES = ["{q}", "{q}呢", "我想{q}", "{q}要怎么做", "{q}求教", "{q}啊"]

def gen_prompts(seed=23):
    rng = random.Random(seed)
    rows = []
    for oral, standard, intent in ORAL_PATTERNS:
        for ot in ORAL_TEMPLATES:
            rows.append({"prompt": ot.format(q=oral), "standard": standard,
                         "intent": intent})
    rng.shuffle(rows)
    return rows

def build_prompt(q, tok):
    return tok.apply_chat_template(
        [{"role": "system", "content": "你是检索查询改写器。把口语化问题改写成知识库检索关键词(操作+对象), 只输出关键词, 10字以内。"},
         {"role": "user", "content": q}],
        add_generation_prompt=True, enable_thinking=False, tokenize=False)

def env_reward(completions, intent=None, **kw):
    """环境奖励: 改写结果 → 真实检索器 → intent 匹配判定"""
    import re as _re
    intents = intent if isinstance(intent, list) else [intent] * len(completions)
    rewards = []
    for comp, want in zip(completions, intents):
        text = comp.strip()
        # 清理生成残留
        text = text.replace("<|im_end|>", "").strip().strip('"').strip("'")
        if not text or text.startswith("<think>"):
            rewards.append(0.0)
            continue
        result = lyv.lookup(PACK, text)
        if result is None:
            rewards.append(0.0)
        elif result["intent"] == want:
            rewards.append(1.0)
        else:
            rewards.append(0.2)  # 召回了但错位
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
    print(f"v2 dataset: {len(ds)} (env=lyv.lookup)", flush=True)

    cfg = GRPOConfig(
        output_dir=OUT_DIR,
        per_device_train_batch_size=8,
        gradient_accumulation_steps=2,
        num_generations=4,
        max_completion_length=48,
        max_steps=MAX_STEPS,
        learning_rate=1e-5,
        logging_steps=10,
        save_strategy="no",
        bf16=True,
        report_to=[],
        temperature=1.0,
    )
    trainer = GRPOTrainer(model=model, reward_funcs=env_reward,
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
    for oral, standard, intent in ORAL_PATTERNS:
        total += 1
        prompt = build_prompt(oral, tok)
        inputs = tok(prompt, return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=32, do_sample=False)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False).strip()
        result = lyv.lookup(PACK, text)
        ok = result is not None and result["intent"] == intent
        hits += ok
        print(f"[{'PASS' if ok else 'FAIL'}] {oral[:18]} -> {text[:26]!r} -> {result['intent'] if result else 'NO_HIT'}", flush=True)
    print(f"环境奖励评测: {hits}/{total} = {hits/total:.0%} (v1 词面奖励 4/6)", flush=True)

if __name__ == "__main__":
    train()
    evaluate()
