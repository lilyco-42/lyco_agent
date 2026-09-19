# -*- coding: utf-8 -*-
"""rewrite_grpo_v5.py — 改写 GRPO v5: 分布外变体扩充 + 标签对齐知识包 (合并版)

合并两个方向 (2026-09-11):
  a) 分布外变体扩充 + 混合动词模板 (原 v5 草案, 04:43)
  b) 标签对齐知识包真实 intent (v4 评测发现, docs/rewrite-grpo-v4-eval-2026-09-11.md):
     - chdir intent 实际是 cd.hello (无 fs.chdir)
     - git.commit / git.push 是两个独立 intent, commit 语义样本期望 git.commit
     - npm 知识包无内容 → node.pkg.install 移出训练/评测集
T4 适配: fp16 (T4 无 bf16), dtype 参数名 (transformers 5.x), save_steps=50 + checkpoint 续训
"""
import glob
import os
import random
import sys

sys.path.insert(0, "/workspace")
import lyv

MODEL_ID = "Qwen/Qwen3-0.6B"
BASE_MODEL = "/workspace/qwen3_lyco_rewrite_v4"
OUT_DIR = "/workspace/qwen3_lyco_rewrite_v5"
MAX_STEPS = 300
PACK = "/workspace/lyv_tmp/pack_merged"

# 合并样本集: 分布外变体 (原 v5) + 修正标签核心集 (v4 25 组去 npm)
ORAL_PATTERNS = [
    # ── create 类 (含 v4 分布外失败模式)
    ("我想写个Rust程序第一步干啥", "新建 rust 项目", "rust.project.create"),
    ("我想搞个Rust代码工程", "新建 rust 项目", "rust.project.create"),
    ("怎么开始一个开发项目", "新建 rust 项目", "rust.project.create"),
    ("Rust程序怎么创建", "新建 rust 项目", "rust.project.create"),
    ("开一个代码工程", "cargo new 工程", "rust.project.create"),
    ("项目文件怎么生成", "cargo new 项目", "rust.project.create"),
    ("Rust项目从零开始怎么搞", "新建 rust 项目", "rust.project.create"),
    ("项目文件建在哪了", "新建 rust 项目", "rust.project.create"),
    ("初始化一个代码项目", "cargo init 项目", "rust.project.create"),
    # ── run 类
    ("程序怎么让他动起来", "运行 项目", "rust.project.run"),
    ("程序怎么让他跑起来", "运行 项目", "rust.project.run"),
    ("Rust代码怎么执行", "运行 项目", "rust.project.run"),
    ("编译好的东西怎么跑", "运行 项目", "rust.project.run"),
    ("代码跑不起来怎么回事", "运行 项目", "rust.project.run"),
    ("跑个hello world看看", "cargo run hello world", "rust.project.run"),
    ("运行程序的命令", "cargo run 程序", "rust.project.run"),
    # ── chdir 类 — 期望修正: 包内 intent = cd.hello
    ("怎么进入那个文件夹", "cd 项目目录", "cd.hello"),
    ("切到项目目录里", "cd 项目目录", "cd.hello"),
    ("到代码目录下去", "cd 目录", "cd.hello"),
    ("跳转到工程文件夹", "cd 工程目录", "cd.hello"),
    ("切换到工程目录", "cd 工程目录", "cd.hello"),
    # ── install 类 (py; npm 移除 — 包无内容)
    ("依赖包怎么下载", "install 依赖", "py.pkg.install"),
    ("第三方库装一下", "install 第三方库", "py.pkg.install"),
    ("第三方包怎么添加", "install 依赖", "py.pkg.install"),
    # ── commit/push 类 — commit 与 push 语义边界 (包内两个独立 intent)
    ("代码提交到仓库", "git commit 提交", "git.push"),
    ("改动推到远程", "git push 推送", "git.push"),
    ("上传代码到github", "git push github", "git.push"),
    ("远端仓库同步", "git push 同步", "git.push"),
    ("保存我的修改", "git commit 修改", "git.commit"),
    ("先把改动提交了先别推送", "git commit 暂存提交", "git.commit"),
    ("本地存个提交记录", "git commit 记录", "git.commit"),
    ("代码改动怎么入库", "git commit 入库", "git.commit"),   # 修正: 入库=commit 非 push
    # ── build 类
    ("项目怎么构建出来", "build 项目", "rust.project.build"),
    ("编译一下代码", "cargo build 代码", "rust.project.build"),
    ("打个发布版本", "cargo build release", "rust.project.build"),
    ("代码怎么编译成二进制", "cargo build 代码", "rust.project.build"),
    ("发布版本怎么打", "cargo build release", "rust.project.build"),
]
ORAL_TEMPLATES = [
    "{q}", "{q}呢", "我想{q}", "{q}要怎么做", "{q}求教", "{q}啊",
    "第一步{q}", "怎么开始{q}", "{q}具体步骤", "{q}该从哪开始",
]


def gen_prompts(seed=67):
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


def latest_checkpoint(out_dir):
    cks = sorted(glob.glob(os.path.join(out_dir, "checkpoint-*")))
    return cks[-1] if cks else None


def load_model_train(path):
    """训练用: fp32 权重 + fp16 AMP (fp16 权重 + fp16=True 会触发 unscale 报错)"""
    from transformers import AutoModelForCausalLM
    import torch
    try:
        return AutoModelForCausalLM.from_pretrained(
            path, dtype=torch.float32, attn_implementation="sdpa").cuda()
    except TypeError:
        return AutoModelForCausalLM.from_pretrained(
            path, torch_dtype=torch.float32, attn_implementation="sdpa").cuda()


def train():
    from datasets import Dataset
    from transformers import AutoTokenizer
    from trl import GRPOConfig, GRPOTrainer

    tok = AutoTokenizer.from_pretrained(MODEL_ID)
    tok.pad_token = tok.eos_token
    model = load_model_train(BASE_MODEL)

    rows = gen_prompts()
    for r in rows:
        r["prompt"] = build_prompt(r["prompt"], tok)
    ds = Dataset.from_list(rows)
    print(f"v5 dataset: {len(ds)} (变体扩充 + 标签对齐包, npm 移除)", flush=True)

    cfg = GRPOConfig(
        output_dir=OUT_DIR,
        per_device_train_batch_size=8,
        gradient_accumulation_steps=2,
        num_generations=4,
        max_completion_length=48,
        max_steps=MAX_STEPS,
        learning_rate=1e-5,
        logging_steps=10,
        save_strategy="steps",
        save_steps=50,
        save_total_limit=2,
        bf16=False,
        fp16=True,
        report_to=[],
        temperature=1.0,
    )
    trainer = GRPOTrainer(model=model, reward_funcs=env_reward,
                          args=cfg, train_dataset=ds, processing_class=tok)
    ckpt = latest_checkpoint(OUT_DIR)
    if ckpt:
        print(f"resume from {ckpt}", flush=True)
        trainer.train(resume_from_checkpoint=ckpt)
    else:
        trainer.train()
    trainer.save_model(OUT_DIR)
    tok.save_pretrained(OUT_DIR)
    print("TRAIN_DONE", flush=True)


def evaluate():
    from transformers import AutoModelForCausalLM, AutoTokenizer
    import torch
    tok = AutoTokenizer.from_pretrained(OUT_DIR)
    try:
        model = AutoModelForCausalLM.from_pretrained(OUT_DIR, dtype=torch.float16, device_map="cuda")
    except TypeError:
        model = AutoModelForCausalLM.from_pretrained(OUT_DIR, torch_dtype=torch.float16, device_map="cuda")
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
    print(f"环境奖励评测: {hits}/{total} = {hits/total:.0%} (v4 同口径修正后 80%)", flush=True)


if __name__ == "__main__":
    if "--eval-only" in sys.argv:
        evaluate()
    else:
        train()
        evaluate()
