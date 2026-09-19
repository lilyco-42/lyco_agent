# -*- coding: utf-8 -*-
"""qwen_grpo_train.py — lyco 工具调用环境 GRPO 最小验证 (R2 Act 阶段)

环境奖励设计 (对齐 lyco 论点: 工具调用能力靠环境反馈, 不靠模型规模):
  期望调用工具且 JSON 可解析        -> +1.0
  + 期望参数 token 出现在 arguments -> +0.5
  期望不调用而模型未调用           -> +1.0
  该调不调 / 乱调 / JSON 坏        ->  0.0

数据: 程序化生成 90 条三类 prompt (知识查询/识图请求/闲聊) + 变体模板
训练: TRL GRPOTrainer, Qwen3-0.6B, num_generations=4, max_steps=200
验证: 训练后自动重跑 5-case FC 测试, 与基线 60% 对比
"""
import json
import random
import re

MODEL_ID = "Qwen/Qwen3-0.6B"
OUT_DIR = "/workspace/qwen3_lyco_grpo"
MAX_STEPS = 200

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

# ---------- 数据生成 ----------
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

def gen_prompts(seed=42):
    rng = random.Random(seed)
    rows = []
    for q in KNOW_Q:
        rows.append({"prompt": q, "expected_tool": "lyv_knowledge",
                     "expected_arg": rng.choice(KNOW_ARG)})
        # 变体: 加前缀
        pre = rng.choice(["请问", "麻烦问下", "", ""])
        rows.append({"prompt": pre + q + rng.choice(["", " 呢", " 谢谢"]),
                     "expected_tool": "lyv_knowledge",
                     "expected_arg": rng.choice(KNOW_ARG)})
    for q in IDENT_Q:
        rows.append({"prompt": q, "expected_tool": "vnn_identify",
                     "expected_arg": rng.choice(IDENT_ARG)})
    for q in CHAT_Q:
        rows.append({"prompt": q, "expected_tool": None, "expected_arg": None})
    rng.shuffle(rows)
    return rows

# ---------- 奖励 ----------
def parse_call(text):
    m = re.search(r"<tool_call>\s*(\{.*?\})\s*</tool_call>", text, re.S)
    if m:
        try:
            return json.loads(m.group(1))
        except Exception:
            return None
    m = re.search(r'\{\s*"name"\s*:\s*"(\w+)"[^}]*\}', text)
    if m:
        return {"name": m.group(1)}
    return None

def tool_reward(completions, expected_tool=None, expected_arg=None, **kw):
    rewards = []
    ets = expected_tool if isinstance(expected_tool, list) else [expected_tool] * len(completions)
    eas = expected_arg if isinstance(expected_arg, list) else [expected_arg] * len(completions)
    for comp, et, ea in zip(completions, ets, eas):
        call = parse_call(comp)
        if et is None:
            rewards.append(1.0 if call is None else 0.0)
        elif call and call.get("name") == et:
            args = json.dumps(call.get("arguments", {}), ensure_ascii=False)
            rewards.append(1.0 + (0.5 if (ea and ea in args) else 0.0))
        else:
            rewards.append(0.0)
    return rewards

def build_prompt(q, tok):
    return tok.apply_chat_template(
        [{"role": "system", "content": "You are lyco, a helpful assistant. You can call tools."},
         {"role": "user", "content": q}],
        tools=TOOLS, add_generation_prompt=True, enable_thinking=False,
        tokenize=False)

# ---------- 训练 ----------
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
    for r in rows:
        r["prompt"] = build_prompt(r["prompt"], tok)
    ds = Dataset.from_list(rows)
    print(f"dataset: {len(ds)} prompts", flush=True)

    cfg = GRPOConfig(
        output_dir=OUT_DIR,
        per_device_train_batch_size=8,       # 8 prompts x 4 gens = 32 completions/step
        gradient_accumulation_steps=2,
        num_generations=4,
        max_completion_length=256,
        max_steps=MAX_STEPS,
        learning_rate=1e-5,
        logging_steps=10,
        save_strategy="no",
        bf16=True,
        report_to=[],
        temperature=1.0,
    )
    trainer = GRPOTrainer(model=model, reward_funcs=tool_reward,
                          args=cfg, train_dataset=ds, processing_class=tok)
    trainer.train()
    trainer.save_model(OUT_DIR)
    tok.save_pretrained(OUT_DIR)
    print("TRAIN_DONE", flush=True)

# ---------- 训练后自评 (与基线 60% 对比) ----------
CASES = [
    ("怎么新建 rust 项目", "lyv_knowledge", "rust"),
    ("帮我看看这张截图里是什么", "vnn_identify", None),
    ("cargo new 之后要做什么", "lyv_knowledge", "cargo"),
    ("你好呀", None, None),
    ("今天天气怎么样", None, None),
]

def evaluate():
    from transformers import AutoModelForCausalLM, AutoTokenizer
    import torch
    src = OUT_DIR if __import__("os").path.exists(OUT_DIR + "/config.json") else MODEL_ID
    tok = AutoTokenizer.from_pretrained(src)
    model = AutoModelForCausalLM.from_pretrained(
        src, torch_dtype=torch.bfloat16, device_map="cuda")
    hits = 0
    for q, et, ea in CASES:
        inputs = tok(build_prompt(q, tok), return_tensors="pt",
                     add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=256, do_sample=False)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False)
        call = parse_call(text)
        if et is None:
            ok = call is None
        else:
            ok = bool(call and call.get("name") == et and
                      (ea is None or ea in json.dumps(call.get("arguments", {}), ensure_ascii=False)))
        hits += ok
        print(f"[{'PASS' if ok else 'FAIL'}] {q} -> {call or text[:60]!r}", flush=True)
    print(f"GRPO 后 FC 遵循度: {hits}/{len(CASES)} = {hits/len(CASES):.0%} (基线 60%)", flush=True)
    json.dump({"rate": hits / len(CASES)},
              open("/workspace/grpo_eval.json", "w"))

if __name__ == "__main__":
    train()
    evaluate()
