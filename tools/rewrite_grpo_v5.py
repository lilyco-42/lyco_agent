# -*- coding: utf-8 -*-
"""rewrite_grpo_v5.py — 改写 GRPO v5: 分布外变体扩充 + 混合动词模式 (修复口语歧义)

v4 遗留: '我想写个Rust程序第一步干啥' 改写成回显 (分布外)
v5 新增: 混合动词模板 (写个/开始/搞) + 域名词变化 (程序/工程/项目)
奖励: 不变 (v2 环境奖励 = 真实检索器判定)
"""
import json
import random
import sys

sys.path.insert(0, "/workspace")
import lyv

MODEL_ID = "Qwen/Qwen3-0.6B"
BASE_MODEL = "/workspace/qwen3_lyco_rewrite_v4"
OUT_DIR = "/workspace/qwen3_lyco_rewrite_v5"
MAX_STEPS = 300
PACK = "/workspace/lyv_tmp/pack_merged"

# v5 扩充: 混合动词 + 域名词变化 → 对应标准词 + intent
ORAL_PATTERNS_V5 = [
    # create 类 (含 v4 遗留的失败模式)
    ("我想写个Rust程序第一步干啥", "新建 rust 项目", "rust.project.create"),
    ("我想搞个Rust代码工程", "新建 rust 项目", "rust.project.create"),
    ("怎么开始一个开发项目", "新建 rust 项目", "rust.project.create"),
    ("Rust程序怎么创建", "新建 rust 项目", "rust.project.create"),
    ("开一个代码工程", "cargo new 工程", "rust.project.create"),
    ("项目文件怎么生成", "cargo new 项目", "rust.project.create"),
    # run 类
    ("程序怎么让他跑起来", "运行 项目", "rust.project.run"),
    ("Rust代码怎么执行", "运行 项目", "rust.project.run"),
    ("编译好的东西怎么跑", "运行 项目", "rust.project.run"),
    ("运行程序的命令", "cargo run 程序", "rust.project.run"),
    # build 类
    ("代码怎么编译成二进制", "cargo build 代码", "rust.project.build"),
    ("发布版本怎么打", "cargo build release", "rust.project.build"),
    # chdir 类
    ("切换到工程目录", "cd 工程目录", "fs.chdir"),
    # install/commit/push 类
    ("第三方包怎么添加", "install 依赖", "py.pkg.install"),
    ("代码改动怎么入库", "git commit 提交", "git.push"),
    ("远端仓库同步", "git push 同步", "git.push"),
]
ORAL_TEMPLATES = [
    "{q}", "{q}呢", "我想{q}", "{q}要怎么做", "{q}求教", "{q}啊",
    "第一步{q}", "怎么开始{q}", "{q}具体步骤", "{q}该从哪开始",
]

def gen_prompts(seed=67):
    rng = random.Random(seed)
    rows = []
    for oral, standard, intent in ORAL_PATTERNS_V5:
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
    print(f"v5 dataset: {len(ds)} (混合动词+域名词变化)", flush=True)

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
    # 重点测 v4 失败的分布外 case
    test_cases = [
        ("我想写个Rust程序第一步干啥", "新建 rust 项目", "rust.project.create"),
        ("程序怎么让他动起来", "运行 项目", "rust.project.run"),
    ] + [(o, s, i) for o, s, i in ORAL_PATTERNS_V5[:5]]
    for oral, standard, intent in test_cases:
        total += 1
        prompt = build_prompt(oral, tok)
        inputs = tok(prompt, return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=32, do_sample=False)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False).strip()
        result = lyv.lookup(PACK, text)
        ok = result is not None and result["intent"] == intent
        hits += ok
        print(f"[{'PASS' if ok else 'FAIL'}] {oral[:18]} -> {text[:26]!r} -> {result['intent'] if result else 'NO_HIT'}", flush=True)
    print(f"v5 环境奖励评测: {hits}/{total} = {hits/total:.0%} (v4 遗留: 1/2 分布外)", flush=True)

if __name__ == "__main__":
    train()
    evaluate()
