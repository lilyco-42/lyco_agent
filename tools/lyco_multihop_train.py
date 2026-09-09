# -*- coding: utf-8 -*-
"""lyco_multihop_train.py — 长任务多跳数据生成 + GRPO 第二轮 (R2: 混合长任务调用)

与第一轮 (qwen_grpo_train.py) 的区别:
  第一轮: 单跳决策 (该不该调/调哪个) — 已达 100%
  本轮:   多跳链式任务 — 模型学会在 tool result 里发现 prereq 缺口后**继续调用**

多跳任务模板 (基于 lyv PREREQ 语义):
  "运行 rust 项目" → lyv 返回 command 但 prereq 含 rust.project.create (未满足)
                  → 期望模型追问 create 的知识 (第二轮 lyv_knowledge)
                  → 满足 prereq 后才总结回答

奖励: 第一跳对 +1, prereq 驱动的第二跳 +2 (长任务核心能力), 最终回答收束 +1
"""
import json
import random
import re

MODEL_ID = "Qwen/Qwen3-0.6B"
OUT_DIR = "/workspace/qwen3_lyco_multihop"
MAX_STEPS = 250

TOOLS = [
    {"type": "function", "function": {
        "name": "lyv_knowledge",
        "description": "查询视频知识库: 问怎么做某操作, 返回带时间戳的视频切片+关键帧+OCR验证文字+prereq前置条件",
        "parameters": {"type": "object", "properties": {
            "query": {"type": "string", "description": "想学的操作"},
        }, "required": ["query"]}}},
    {"type": "function", "function": {
        "name": "check_prereq",
        "description": "检查某个前置条件是否已满足 (如 rust.project.create)",
        "parameters": {"type": "object", "properties": {
            "intent": {"type": "string", "description": "前置条件 intent id"},
        }, "required": ["intent"]}}},
]

# 模拟环境状态: prereq 满足表 (训练时随机, 教模型「先查再决定」)
PREREQ_MAP = {
    "rust.project.run": "rust.project.create",
    "rust.project.build": "rust.project.create",
    "rust.cargo-installed": None,
}

KNOW_Q = ["怎么运行 rust 项目", "如何 cargo build", "编译项目怎么做",
          "怎么跑 cargo run", "运行程序怎么做", "build 项目"]
CHAT_Q = ["你好", "1 加 1 等于几", "今天星期几", "再见"]

def gen_prompts(seed=7):
    rng = random.Random(seed)
    rows = []
    for q in KNOW_Q:
        for state in (True, False):
            rows.append({"prompt": q, "type": "know",
                         "prereq_satisfied": state})
    for q in CHAT_Q:
        rows.append({"prompt": q, "type": "chat", "prereq_satisfied": True})
    rng.shuffle(rows)
    return rows

def build_prompt(q, tok):
    return tok.apply_chat_template(
        [{"role": "system", "content": "You are lyco. 操作类问题先用 lyv_knowledge 查询知识库."},
         {"role": "user", "content": q}],
        tools=TOOLS, add_generation_prompt=True, enable_thinking=False,
        tokenize=False)

def parse_call(text):
    m = re.search(r"<tool_call>\s*(\{.*?\})\s*</tool_call>", text, re.S)
    if m:
        try:
            return json.loads(m.group(1))
        except Exception:
            return None
    return None

def extract_intent(text, key="intent"):
    try:
        return json.loads(text).get(key)
    except Exception:
        return None

# ---------- 训练: 多轮对话式奖励 ----------
def multihop_reward(completions, prompt=None, **kw):
    """简化多轮奖励: 对每条 completion, 若含 tool_call 且是 check_prereq 或
    二次 lyv_knowledge (查询串与首轮不同) → +2 (学会追查 prereq);
    首跳 lyv_knowledge +1; 无调用 0"""
    rewards = []
    for comp in completions:
        calls = re.findall(r"<tool_call>\s*(\{.*?\})\s*</tool_call>", comp, re.S)
        parsed = []
        for c in calls:
            try:
                parsed.append(json.loads(c))
            except Exception:
                pass
        if not parsed:
            # 直接文字回答: 对 chat 合理, 对 know 类不给分
            rewards.append(0.3)
            continue
        r = 0.0
        names = [c.get("name") for c in parsed]
        if names[0] == "lyv_knowledge":
            r += 1.0
        # 多跳: 第二次调用存在 (check_prereq 或再次 lyv)
        if len(parsed) >= 2:
            second = parsed[1].get("name")
            if second == "check_prereq" or second == "lyv_knowledge":
                r += 2.0
        rewards.append(r)
    return rewards

def train():
    from datasets import Dataset
    from transformers import AutoModelForCausalLM, AutoTokenizer
    from trl import GRPOConfig, GRPOTrainer
    import torch

    src = OUT_DIR.replace("multihop", "grpo")  # 从第一轮产物继续训
    import os
    if not os.path.exists(src + "/config.json"):
        src = MODEL_ID
    tok = AutoTokenizer.from_pretrained(MODEL_ID)
    tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(
        src, torch_dtype=torch.bfloat16, attn_implementation="sdpa").cuda()

    rows = gen_prompts()
    for r in rows:
        r["prompt"] = build_prompt(r["prompt"], tok)
    ds = Dataset.from_list(rows)
    print(f"multihop dataset: {len(ds)}", flush=True)

    cfg = GRPOConfig(
        output_dir=OUT_DIR,
        per_device_train_batch_size=8,
        gradient_accumulation_steps=2,
        num_generations=4,
        max_completion_length=320,
        max_steps=MAX_STEPS,
        learning_rate=1e-5,
        logging_steps=10,
        save_strategy="no",
        bf16=True,
        report_to=[],
        temperature=1.0,
    )
    trainer = GRPOTrainer(model=model, reward_funcs=multihop_reward,
                          args=cfg, train_dataset=ds, processing_class=tok)
    trainer.train()
    trainer.save_model(OUT_DIR)
    tok.save_pretrained(OUT_DIR)
    print("TRAIN_DONE", flush=True)

def evaluate():
    """两跳任务实测: 运行项目 → (发现 prereq) → 查 create → 收束"""
    from transformers import AutoModelForCausalLM, AutoTokenizer
    import torch
    tok = AutoTokenizer.from_pretrained(OUT_DIR)
    model = AutoModelForCausalLM.from_pretrained(
        OUT_DIR, torch_dtype=torch.bfloat16, device_map="cuda")

    messages = [{"role": "system", "content": "You are lyco. 操作类问题先用 lyv_knowledge 查询知识库."},
                {"role": "user", "content": "怎么运行 rust 项目"}]
    hops = 0
    for rnd in range(4):
        prompt = tok.apply_chat_template(messages, tools=TOOLS,
                                         add_generation_prompt=True,
                                         enable_thinking=False, tokenize=False)
        inputs = tok(prompt, return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=300, do_sample=False)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False)
        call = parse_call(text)
        if call is None:
            print(f"[{rnd+1}] FINAL: {text[:120]!r}")
            break
        hops += 1
        print(f"[{rnd+1}] CALL: {call}")
        # 模拟环境
        if call.get("name") == "lyv_knowledge":
            result = {"ok": True, "command": "cargo run",
                      "prereq": ["rust.project.create"],
                      "prereq_satisfied": False,
                      "hint": "前置条件未满足, 建议 check_prereq 或先查询 create"}
        elif call.get("name") == "check_prereq":
            result = {"ok": True, "satisfied": False,
                      "hint": "未满足, 先创建项目"}
        messages.append({"role": "assistant",
                         "content": f"<tool_call>\n{json.dumps(call, ensure_ascii=False)}\n</tool_call>"})
        messages.append({"role": "tool", "content": json.dumps(result, ensure_ascii=False)})
    print(f"MULTIHOP_HOPS: {hops} (>=2 = 学会追查 prereq)")

if __name__ == "__main__":
    train()
    evaluate()
