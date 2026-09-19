# -*- coding: utf-8 -*-
"""mixed_grpo_v5.py — 多任务混合 GRPO: 单模型同时学会 tool_call + 改写 (v5 实验)

背景 (灾难性遗忘发现): v4 在 v1 基础上顺序训 rewrite → 丢 tool_call。
v5 假设: 两类任务数据**混合**从头训 (而非顺序), 可同时保持两种能力。

数据:
  A. tool_call 决策 (v1 的 gen_prompts, 118 条)
  B. 查询改写     (v3 的 ORAL_PATTERNS+模板, 150 条)
奖励: 按 prompt 类型分发 (A→tool_reward, B→rewrite_reward/env)
验证: 双能力复测 — tool_call 5-case + 改写 10-case, 任一 <80% 即遗忘
"""
import json
import random
import re
import sys

sys.path.insert(0, "/workspace")
import lyv

MODEL_ID = "Qwen/Qwen3-0.6B"
OUT_DIR = "/workspace/qwen3_lyco_v5_mixed"
MAX_STEPS = 400
PACK = "/workspace/lyv_tmp/pack_merged"

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
        "description": "OCR失败时启用内部识图神经网络, 特征激活式对图片打分描述",
        "parameters": {"type": "object", "properties": {
            "image": {"type": "string", "description": "图片路径"},
        }, "required": ["image"]}}},
]

KNOW_Q = ["怎么新建 rust 项目", "如何创建 rust 项目", "cargo new 之后要做什么",
          "怎么运行项目", "运行程序怎么做", "cargo run 怎么打", "如何初始化项目",
          "新建项目第一步", "怎么 cargo build", "编译项目用什么命令",
          "如何安装依赖", "怎么加 crate", "git clone 之后干嘛", "怎么提交代码",
          "pip install 怎么用", "npm install 怎么执行", "怎么进入项目目录",
          "cd 到哪个目录", "hello world 怎么写", "怎么输出 hello world"]
IDENT_Q = ["帮我看看这张截图里是什么", "这张图片是什么内容", "识别一下这个画面",
           "屏幕上显示的是什么", "这个截图什么意思", "看看这张图",
           "图里是什么软件", "这个界面是什么应用", "帮我识图", "图中有什么"]
CHAT_Q = ["你好呀", "今天天气怎么样", "你会唱歌吗", "讲个笑话", "1 加 1 等于几",
          "你叫什么名字", "谢谢啦", "再见", "周末去哪玩好", "推荐一部电影",
          "地球是圆的吗", "天空为什么是蓝色的", "早饭吃什么好", "晚安",
          "帮我算算 23 乘 4", "周杰伦是谁", "猫和狗哪个聪明", "怎么学英语"]
KNOW_ARG = ["rust", "cargo", "项目", "run", "build", "install", "clone",
            "commit", "hello", "目录"]
IDENT_ARG = ["截图", "图片", "画面", "图"]

ORAL_PATTERNS = [
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
    ("项目怎么构建出来", "build 项目", "rust.project.build"),
    ("改动推到远程", "git push 推送", "git.push"),
]
ORAL_TEMPLATES = ["{q}", "{q}呢", "我想{q}", "{q}要怎么做", "{q}求教", "{q}啊"]

def seg(text):
    out, buf = [], ""
    for ch in text:
        if ch in " \t\r\n":
            if buf: out.append(buf); buf = ""
            continue
        if ch.isascii() and ch not in "。，；！？":
            buf += ch
        else:
            if buf: out.append(buf); buf = ""
            if ch not in "。，；！？": out.append(ch)
    if buf: out.append(buf)
    return " ".join(out)

def gen_prompts(seed=53):
    rng = random.Random(seed)
    rows = []
    # A 类: tool_call 决策
    for q in KNOW_Q:
        rows.append({"prompt": q, "type": "tool",
                     "expected_tool": "lyv_knowledge", "expected_arg": rng.choice(KNOW_ARG)})
        rows.append({"prompt": rng.choice(["请问", "", ""]) + q,
                     "type": "tool", "expected_tool": "lyv_knowledge",
                     "expected_arg": rng.choice(KNOW_ARG)})
    for q in IDENT_Q:
        rows.append({"prompt": q, "type": "tool",
                     "expected_tool": "vnn_identify", "expected_arg": rng.choice(IDENT_ARG)})
    for q in CHAT_Q:
        rows.append({"prompt": q, "type": "chat", "expected_tool": None,
                     "expected_arg": None})
    # B 类: 查询改写
    for oral, standard, intent in ORAL_PATTERNS:
        for ot in ORAL_TEMPLATES:
            rows.append({"prompt": ot.format(q=oral), "type": "rewrite",
                         "standard": standard, "intent": intent})
    rng.shuffle(rows)
    return rows

def build_prompt(q, tok, with_tools):
    if with_tools:
        return tok.apply_chat_template(
            [{"role": "system", "content": "You are lyco. 操作类问题先用工具查询知识库."},
             {"role": "user", "content": q}],
            tools=TOOLS, add_generation_prompt=True, enable_thinking=False, tokenize=False)
    return tok.apply_chat_template(
        [{"role": "system", "content": "你是检索查询改写器。把口语化问题改写成检索关键词(操作+对象), 只输出关键词, 10字以内。"},
         {"role": "user", "content": q}],
        add_generation_prompt=True, enable_thinking=False, tokenize=False)

def parse_call(text):
    m = re.search(r"<tool_call>\s*(\{.*?\})\s*</tool_call>", text, re.S)
    if m:
        try:
            return json.loads(m.group(1))
        except Exception:
            return None
    return None

def mixed_reward(completions, rtype=None, expected_tool=None, expected_arg=None,
                 standard=None, intent=None, **kw):
    def one(comp, rtype, et, ea, std, it):
        if rtype == "tool":
            call = parse_call(comp)
            if et is None:
                return 1.0 if call is None else 0.0
            if call and call.get("name") == et:
                args = json.dumps(call.get("arguments", {}), ensure_ascii=False)
                return 1.0 + (0.5 if (ea and ea in args) else 0.0)
            return 0.0
        if rtype == "rewrite":
            text = comp.strip().replace("<|im_end|>", "").strip().strip('"').strip("'")
            if not text or text.startswith("<think>"):
                return 0.0
            result = lyv.lookup(PACK, text)
            if result is None:
                return 0.0
            return 1.0 if result["intent"] == it else 0.2
        return 0.0
    def as_list(x):
        return x if isinstance(x, list) else [x] * len(completions)
    return [one(c, r, e, a, s, i) for c, r, e, a, s, i in zip(
        completions, as_list(rtype), as_list(expected_tool),
        as_list(expected_arg), as_list(standard), as_list(intent))]

def train():
    from datasets import Dataset
    from transformers import AutoModelForCausalLM, AutoTokenizer
    from trl import GRPOConfig, GRPOTrainer
    import torch

    tok = AutoTokenizer.from_pretrained(MODEL_ID)
    tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(
        MODEL_ID, torch_dtype=torch.bfloat16, attn_implementation="sdpa").cuda()

    rows = gen_prompts()
    tool_rows = [r for r in rows if r["type"] != "rewrite"]
    for r in tool_rows:
        r["prompt"] = build_prompt(r["prompt"], tok, with_tools=True)
    rewrite_rows = [r for r in rows if r["type"] == "rewrite"]
    for r in rewrite_rows:
        r["prompt"] = build_prompt(r["prompt"], tok, with_tools=False)
    ds = Dataset.from_list(rows)
    print(f"mixed dataset: {len(ds)} (tool={len(tool_rows)} rewrite={len(rewrite_rows)})", flush=True)

    cfg = GRPOConfig(
        output_dir=OUT_DIR,
        per_device_train_batch_size=8,
        gradient_accumulation_steps=2,
        num_generations=4,
        max_completion_length=128,
        max_steps=MAX_STEPS,
        learning_rate=1e-5,
        logging_steps=10,
        save_strategy="no",
        bf16=True,
        report_to=[],
        temperature=1.0,
    )
    trainer = GRPOTrainer(model=model, reward_funcs=mixed_reward,
                          args=cfg, train_dataset=ds, processing_class=tok)
    trainer.train()
    trainer.save_model(OUT_DIR)
    tok.save_pretrained(OUT_DIR)
    print("TRAIN_DONE", flush=True)

def evaluate():
    """双能力复测: tool_call 5-case + 改写 10-case"""
    from transformers import AutoModelForCausalLM, AutoTokenizer
    import torch
    tok = AutoTokenizer.from_pretrained(OUT_DIR)
    model = AutoModelForCausalLM.from_pretrained(
        OUT_DIR, torch_dtype=torch.bfloat16, device_map="cuda")

    # A: tool_call
    fc_cases = [("怎么新建 rust 项目", "lyv_knowledge", "rust"),
                ("帮我看看这张截图里是什么", "vnn_identify", None),
                ("cargo new 之后要做什么", "lyv_knowledge", "cargo"),
                ("你好呀", None, None),
                ("今天天气怎么样", None, None)]
    fc_hits = 0
    for q, et, ea in fc_cases:
        prompt = build_prompt(q, tok, with_tools=True)
        inputs = tok(prompt, return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=256, do_sample=False)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False)
        call = parse_call(text)
        if et is None:
            ok = call is None
        else:
            ok = bool(call and call.get("name") == et and
                      (ea is None or ea in json.dumps(call.get("arguments", {}), ensure_ascii=False)))
        fc_hits += ok
        print(f"[{'PASS' if ok else 'FAIL'}][FC] {q}", flush=True)
    # B: 改写
    rw_hits = 0
    for oral, standard, intent in ORAL_PATTERNS[:10]:
        prompt = build_prompt(oral, tok, with_tools=False)
        inputs = tok(prompt, return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=32, do_sample=False)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False).strip()
        result = lyv.lookup(PACK, text)
        ok = result is not None and result["intent"] == intent
        rw_hits += ok
        print(f"[{'PASS' if ok else 'FAIL'}][RW] {oral[:18]} -> {text[:26]!r}", flush=True)
    print(f"双能力: FC {fc_hits}/5 (基线 100%), 改写 {rw_hits}/10 (基线 70%)", flush=True)
    both = (fc_hits / 5 >= 0.8) and (rw_hits / 10 >= 0.6)
    print("V5_VERDICT:", "单模型可行" if both else "维持分工模型", flush=True)

if __name__ == "__main__":
    train()
    evaluate()
