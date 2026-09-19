# -*- coding: utf-8 -*-
"""fc_grpo_corpus.py — 用 answer-first 语料做 FC GRPO 训练 (P1③ 训练侧)

与本仓既有 `qwen_grpo_train.py` (程序化数据, 2 工具) 的区别:
  1. **7 工具全量 schema**: 从 `/workspace/tools_openai.json` 加载 (由 `lycore tools --out`
     导出 = CHAT_TOOLS 单一真源), 不再手工维护 2 工具副本。
  2. **语料 = datagen 真实语料 ∪ 程序化 prompt**: 读 `lycore datagen` 产出的
     `/workspace/sft_corpus.jsonl` (answer-first: query + 规范 tool_call), 再补既有
     程序化 prompt 保证体量 (真实轨迹样本目前偏少)。
  3. 训练后 8-case 自评 (覆盖 7 工具 + 闲聊), 与基线对比。

环境 (CloudStudio A10 实测): python 3.11 + torch 2.10 + transformers 5.1 + trl 1.13。
坑 (沿用既有结论): 必须 enable_thinking=False (否则 token 全耗在 <think>);
  apply_chat_template 用 tokenize=False, 生成时 add_special_tokens=False。
"""
import json
import os
import random
import re

MODEL_ID = "Qwen/Qwen3-0.6B"
OUT_DIR = "/workspace/qwen3_lyco_fc_corpus"
CORPUS = "/workspace/sft_corpus.jsonl"
TOOLS_JSON = "/workspace/tools_openai.json"
MAX_STEPS = int(os.environ.get("MAX_STEPS", "120"))

SYSTEM = "You are lyco, a helpful assistant. You can call tools."

# ---------- 工具 schema (单一真源: lycore tools --out) ----------
def load_tools():
    if os.path.exists(TOOLS_JSON):
        return json.load(open(TOOLS_JSON, encoding="utf-8"))
    # 兜底: 最小 2 工具 (与 qwen_grpo_train.py 对齐)
    return [
        {"type": "function", "function": {"name": "lyv_knowledge", "description": "查询视频知识库",
         "parameters": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}}},
        {"type": "function", "function": {"name": "vnn_identify", "description": "识别图片内容",
         "parameters": {"type": "object", "properties": {"image": {"type": "string"}}, "required": ["image"]}}},
    ]

# ---------- 奖励 (与 qwen_grpo_train.py 同口径) ----------
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

def build_prompt(q, tok, tools):
    return tok.apply_chat_template(
        [{"role": "system", "content": SYSTEM}, {"role": "user", "content": q}],
        tools=tools, add_generation_prompt=True, enable_thinking=False, tokenize=False)

# ---------- 语料 ----------
def load_corpus_rows(path):
    """读 datagen 产出的 answer-first 语料 → GRPO rows"""
    rows = []
    if not os.path.exists(path):
        return rows
    for line in open(path, encoding="utf-8"):
        line = line.strip()
        if not line:
            continue
        v = json.loads(line)
        q = v["messages"][0]["content"]
        call = parse_call(v["messages"][1]["content"]) or {}
        tool = call.get("name")
        rows.append({"prompt": q, "expected_tool": tool, "expected_arg": None})
    return rows

# 程序化补充 (覆盖 7 工具, 保证体量)
PROG = [
    ("怎么新建 rust 项目", "lyv_knowledge", "rust"),
    ("cargo new 之后要做什么", "lyv_knowledge", "cargo"),
    ("怎么运行项目", "lyv_knowledge", "run"),
    ("如何安装依赖", "lyv_knowledge", "install"),
    ("git clone 之后干嘛", "lyv_knowledge", "clone"),
    ("帮我看看这张截图里是什么", "vnn_identify", None),
    ("识别一下这个画面", "vnn_identify", None),
    ("这个界面是什么应用", "vnn_identify", None),
    ("帮我把这张图抠图去掉背景", "rembg_remove", None),
    ("这张图片去背", "rembg_remove", None),
    ("生成一个产品落地页", "html_gen", None),
    ("做一个单文件网页", "html_gen", None),
    ("写一段产品介绍文案", "llm_generate", None),
    ("帮我想个队名", "llm_generate", None),
    ("把这个网页录制成视频", "html_render_video", None),
    ("网页转成 mp4", "html_render_video", None),
    ("这个视频多长", "video_info", None),
    ("查看视频分辨率帧率", "video_info", None),
    ("你好呀", None, None),
    ("今天天气怎么样", None, None),
    ("讲个笑话", None, None),
    ("谢谢啦", None, None),
]

def gen_rows(corpus_rows):
    rows = list(corpus_rows)
    for q, et, ea in PROG:
        rows.append({"prompt": q, "expected_tool": et, "expected_arg": ea})
    random.Random(42).shuffle(rows)
    return rows

# ---------- 训练 ----------
def train():
    from datasets import Dataset
    from transformers import AutoModelForCausalLM, AutoTokenizer
    from trl import GRPOConfig, GRPOTrainer
    import torch

    tools = load_tools()
    print(f"tools: {len(tools)}", flush=True)
    tok = AutoTokenizer.from_pretrained(MODEL_ID)
    tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(
        MODEL_ID, torch_dtype=torch.bfloat16, attn_implementation="sdpa").cuda()

    corpus_rows = load_corpus_rows(CORPUS)
    print(f"corpus rows (datagen): {len(corpus_rows)}", flush=True)
    rows = gen_rows(corpus_rows)
    for r in rows:
        r["prompt"] = build_prompt(r["prompt"], tok, tools)
    ds = Dataset.from_list(rows)
    print(f"dataset: {len(ds)} prompts", flush=True)

    cfg = GRPOConfig(
        output_dir=OUT_DIR,
        per_device_train_batch_size=8,
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

# ---------- 自评 (8-case, 覆盖 7 工具 + 闲聊) ----------
CASES = [
    ("怎么新建 rust 项目", "lyv_knowledge", "rust"),
    ("帮我看看这张截图里是什么", "vnn_identify", None),
    ("帮我把这张图抠图去掉背景", "rembg_remove", None),
    ("生成一个产品落地页", "html_gen", None),
    ("写一段产品介绍文案", "llm_generate", None),
    ("把这个网页录制成视频", "html_render_video", None),
    ("这个视频多长", "video_info", None),
    ("你好呀", None, None),
]

def evaluate():
    from transformers import AutoModelForCausalLM, AutoTokenizer
    import torch
    tools = load_tools()
    src = os.environ.get("EVAL_SRC") or (OUT_DIR if os.path.exists(OUT_DIR + "/config.json") else MODEL_ID)
    print(f"eval src: {src}", flush=True)
    tok = AutoTokenizer.from_pretrained(src)
    model = AutoModelForCausalLM.from_pretrained(
        src, torch_dtype=torch.bfloat16, device_map="cuda")
    hits = 0
    for q, et, ea in CASES:
        inputs = tok(build_prompt(q, tok, tools), return_tensors="pt",
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
    print(f"GRPO(语料版) 后 FC 遵循度: {hits}/{len(CASES)} = {hits/len(CASES):.0%}", flush=True)
    json.dump({"rate": hits / len(CASES)}, open("/workspace/fc_corpus_eval.json", "w"))

if __name__ == "__main__":
    if os.environ.get("EVAL_ONLY") == "1":
        evaluate()  # 基线: OUT_DIR 不存在时自动回退 MODEL_ID
    else:
        train()
        evaluate()
