# -*- coding: utf-8 -*-
"""rewrite_grpo_v3.py — 样本平衡迭代 (v2 诊断: install/commit 类样本不足)

改动: ORAL_PATTERNS 扩到 25 组, 每类意图 ≥4 个口语变体, 6 模板 = ~150 prompts
奖励: 不变 (v2 环境奖励已验证正确 — 问题在数据不在奖励)
"""
import json
import random
import sys

sys.path.insert(0, "/workspace")
import lyv

MODEL_ID = "Qwen/Qwen3-0.6B"
BASE_MODEL = "/workspace/qwen3_lyco_rewrite_v2"
OUT_DIR = "/workspace/qwen3_lyco_rewrite_v4"
MAX_STEPS = 300
PACK = "/workspace/lyv_tmp/pack_merged"

# 25 组: (口语, 标准检索词, 期望 intent) — 每类意图 ≥3 组
ORAL_PATTERNS = [
    # create 类 x5
    ("我想写个Rust程序第一步干啥", "新建 rust 项目", "rust.project.create"),
    ("Rust项目从零开始怎么搞", "新建 rust 项目", "rust.project.create"),
    ("项目文件建在哪了", "新建 rust 项目", "rust.project.create"),
    ("开个新工程练练手", "cargo new 工程", "rust.project.create"),
    ("初始化一个代码项目", "cargo init 项目", "rust.project.create"),
    # run 类 x5
    ("程序怎么让他动起来", "运行 项目", "rust.project.run"),
    ("写好的代码怎么跑", "运行 项目", "rust.project.run"),
    ("代码跑不起来怎么回事", "运行 项目", "rust.project.run"),
    ("跑个hello world看看", "cargo run hello world", "rust.project.run"),
    ("执行程序用什么命令", "cargo run 程序", "rust.project.run"),
    # chdir 类 x4
    ("怎么进入那个文件夹", "cd 项目目录", "fs.chdir"),
    ("切到项目目录里", "cd 项目目录", "fs.chdir"),
    ("到代码目录下去", "cd 目录", "fs.chdir"),
    ("跳转到工程文件夹", "cd 工程目录", "fs.chdir"),
    # install 类 x4
    ("依赖包怎么下载", "install 依赖", "py.pkg.install"),
    ("第三方库装一下", "install 第三方库", "py.pkg.install"),
    ("npm的包咋安装", "npm install 包", "node.pkg.install"),
    ("缺个模块装哪个命令", "npm install 模块", "node.pkg.install"),
    # commit/push 类 x4
    ("代码提交到仓库", "git commit 提交", "git.push"),
    ("改动推到远程", "git push 推送", "git.push"),
    ("保存我的修改", "git commit 修改", "git.push"),
    ("上传代码到github", "git push github", "git.push"),
    # build 类 x3
    ("项目怎么构建出来", "build 项目", "rust.project.build"),
    ("编译一下代码", "cargo build 代码", "rust.project.build"),
    ("打个发布版本", "cargo build release", "rust.project.build"),
]
ORAL_TEMPLATES = ["{q}", "{q}呢", "我想{q}", "{q}要怎么做", "{q}求教", "{q}啊"]

def gen_prompts(seed=37):
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
    intents = intent if isinstance(intent, list) else [intent] * len(completions)
    rewards = []
    for comp, want in zip(completions, intents):
        text = comp.strip().replace("<|im_end|>", "").strip().strip('"').strip("'")
        if not text or text.startswith("<think>"):
            rewards.append(0.0)
            continue
        result = lyv.lookup(PACK, text)
        if result is None:
            rewards.append(0.0)
        elif result["intent"] == want:
            rewards.append(1.0)
        else:
            rewards.append(0.2)
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
    print(f"v4 dataset: {len(ds)} (env=pack_merged)", flush=True)

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
    by_intent = {}
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
        by_intent.setdefault(intent, [0, 0])
        by_intent[intent][1] += 1
        by_intent[intent][0] += ok
        if not ok:
            print(f"[FAIL] {oral[:18]} -> {text[:26]!r} -> {result['intent'] if result else 'NO_HIT'}", flush=True)
    for intent, (h, t) in sorted(by_intent.items()):
        print(f"  {intent}: {h}/{t}")
    print(f"环境奖励评测: {hits}/{total} = {hits/total:.0%} (v2: 7/10)", flush=True)

if __name__ == "__main__":
    train()
    evaluate()
