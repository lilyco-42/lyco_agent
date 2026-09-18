# -*- coding: utf-8 -*-
"""fc_train_v2.py — 论文驱动的 FC 模型优化 (v1 → v2)

v1 (`fc_grpo_corpus.py`) 缺陷: 直接 GRPO、无冷启动 SFT、闲聊样本仅 4/34 → "你好呀" 误调 lyv_knowledge。

本脚本按论文改三处 (均为可测的单变量改进):
  1. **DeepSeek-R1 (Nature 2026-09-02)**: "纯 RL 会做但不会好好表达" → 加**冷启动 SFT**
     阶段 (格式/行为先固定), 再进 GRPO。
  2. **Hammer (ICLR 2025)**: 注入 **irrelevant functions** (干扰工具), 逼模型学会"在多个
     工具中选对", 而非被函数名表层特征带偏。
  3. **数据再平衡 (ToolGrad 的教训: 多样性>条数)**: 补齐**闲聊负样本**与其正确"不调用"回复,
     直接修 误调。

用法: python fc_train_v2.py {sft|grpo|eval} [src]
  eval BASE                       → 评测底座 Qwen3-0.6B
  eval /workspace/xxx             → 评测某产物
  sft                             → 冷启动 SFT → /workspace/qwen3_lyco_fc_sft
  grpo                            → 从 SFT 续 GRPO → /workspace/qwen3_lyco_fc_v2
"""
import json
import os
import re
import sys

MODEL_ID = "Qwen/Qwen3-0.6B"
SFT_DIR = "/workspace/qwen3_lyco_fc_sft"
V2_DIR = "/workspace/qwen3_lyco_fc_v2"
CORPUS = "/workspace/sft_corpus.jsonl"
TOOLS_JSON = "/workspace/tools_openai.json"
MAX_STEPS = int(os.environ.get("MAX_STEPS", "120"))
SYSTEM = "You are lyco, a helpful assistant. You can call tools."

# 干扰工具 (Hammer 式 irrelevant functions): 训练期混入, 逼模型按语义而非表面特征选择
DISTRACTORS = [
    {"type": "function", "function": {"name": "legacy_sync", "description": "已废弃的旧版同步接口, 不推荐使用",
     "parameters": {"type": "object", "properties": {"target": {"type": "string"}}, "required": ["target"]}}},
    {"type": "function", "function": {"name": "internal_audit", "description": "内部审计日志导出 (运维专用)",
     "parameters": {"type": "object", "properties": {"since": {"type": "string"}}, "required": ["since"]}}},
    {"type": "function", "function": {"name": "deprecated_ocr", "description": "旧版 OCR, 已被 vnn_identify 取代",
     "parameters": {"type": "object", "properties": {"file": {"type": "string"}}, "required": ["file"]}}},
    {"type": "function", "function": {"name": "cron_admin", "description": "管理 crontab 条目 (系统运维)",
     "parameters": {"type": "object", "properties": {"expr": {"type": "string"}}, "required": ["expr"]}}},
    {"type": "function", "function": {"name": "mail_send", "description": "发送电子邮件",
     "parameters": {"type": "object", "properties": {"to": {"type": "string"}}, "required": ["to"]}}},
]

def load_tools():
    if os.path.exists(TOOLS_JSON):
        return json.load(open(TOOLS_JSON, encoding="utf-8"))
    return [
        {"type": "function", "function": {"name": "lyv_knowledge", "description": "查询视频知识库",
         "parameters": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}}},
        {"type": "function", "function": {"name": "vnn_identify", "description": "识别图片内容",
         "parameters": {"type": "object", "properties": {"image": {"type": "string"}}, "required": ["image"]}}},
    ]

def parse_call(text):
    m = re.search(r"<tool_call>\s*(\{.*?\})\s*</tool_call>", text, re.S)
    if m:
        try:
            return json.loads(m.group(1))
        except Exception:
            return None
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

# ---------- 数据 ----------
# (query, tool, arg, 正确回复) —— 回复即 answer-first 的 "answer"
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
]

# 闲聊负样本 + 正确"不调用"回复 (修 v1 误调; R1 式冷启动 SFT 的关键养料)
CHAT = [
    ("你好呀", "你好呀！有什么可以帮你的吗？"),
    ("今天天气怎么样", "我暂时没法联网查天气，你可以看看手机自带的天气。"),
    ("讲个笑话", "程序员最讨厌的两件事：写文档，和别人不写文档。"),
    ("谢谢啦", "不客气，随时找我。"),
    ("1 加 1 等于几", "等于 2。"),
    ("你叫什么名字", "我是 lyco，你的本地助手。"),
    ("周末去哪玩好", "看你喜欢什么类型，想安静就去公园，想热闹就去逛街。"),
    ("晚安", "晚安，好好休息。"),
    ("周杰伦是谁", "他是华语流行歌手，代表作很多。"),
    ("你会唱歌吗", "我不会唱，但可以帮你写歌词。"),
    ("今天几号", "我这边看不到系统日期，你可以看下手机。"),
    ("帮我算算 23 乘 4", "23 乘 4 等于 92。"),
]

def load_corpus_rows(path):
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
        rows.append((q, call.get("name"), None))
    return rows

def build_rows():
    """→ [(q, tool, arg)]; tool=None 表示该不调用"""
    rows = list(load_corpus_rows(CORPUS))
    rows += PROG
    rows += [(q, None, None) for q, _ in CHAT]
    return rows

def reply_for(tool, q):
    """answer-first 的规范回复: 该调 → tool_call; 不该调 → 自然语言"""
    if tool:
        return f'<tool_call>{json.dumps({"name": tool, "arguments": _synth_args(tool, q)}, ensure_ascii=False)}</tool_call>'
    for qq, r in CHAT:
        if qq == q:
            return r
    return "好的。"

def _synth_args(tool, q):
    if tool == "lyv_knowledge":
        return {"query": q}
    if tool in ("llm_generate", "html_gen"):
        return {"prompt": q}
    return {}

# ---------- 冷启动 SFT ----------
def sft():
    from datasets import Dataset
    from transformers import (AutoModelForCausalLM, AutoTokenizer, Trainer,
                              TrainingArguments, DataCollatorForLanguageModeling)
    import torch

    tools = load_tools() + DISTRACTORS          # 训练期混入干扰工具 (Hammer)
    tok = AutoTokenizer.from_pretrained(MODEL_ID)
    tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(
        MODEL_ID, torch_dtype=torch.bfloat16, attn_implementation="sdpa").cuda()

    texts = []
    for q, tool, _ in build_rows():
        msgs = [{"role": "system", "content": SYSTEM},
                {"role": "user", "content": q},
                {"role": "assistant", "content": reply_for(tool, q)}]
        texts.append(tok.apply_chat_template(msgs, tools=tools, tokenize=False, enable_thinking=False))
    print(f"SFT 样本: {len(texts)}", flush=True)

    ds = Dataset.from_list([{"text": t} for t in texts]).map(
        lambda b: tok(b["text"], truncation=True, max_length=1024), batched=True, remove_columns=["text"])
    args = TrainingArguments(
        output_dir=SFT_DIR, num_train_epochs=3, per_device_train_batch_size=4,
        learning_rate=1e-5, logging_steps=5, save_strategy="no", bf16=True, report_to=[])
    Trainer(model=model, args=args, train_dataset=ds,
            data_collator=DataCollatorForLanguageModeling(tok, mlm=False)).train()
    model.save_pretrained(SFT_DIR)
    tok.save_pretrained(SFT_DIR)
    print("SFT_DONE", flush=True)

# ---------- GRPO (从 SFT 续) ----------
def grpo():
    from datasets import Dataset
    from transformers import AutoModelForCausalLM, AutoTokenizer
    from trl import GRPOConfig, GRPOTrainer
    import torch

    tools = load_tools() + DISTRACTORS
    src = SFT_DIR if os.path.exists(SFT_DIR + "/config.json") else MODEL_ID
    tok = AutoTokenizer.from_pretrained(src)
    tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(
        src, torch_dtype=torch.bfloat16, attn_implementation="sdpa").cuda()

    rows = [{"prompt": build_prompt(q, tok, tools), "expected_tool": t, "expected_arg": a}
            for q, t, a in build_rows()]
    print(f"GRPO dataset: {len(rows)} prompts (src={src})", flush=True)

    cfg = GRPOConfig(
        output_dir=V2_DIR, per_device_train_batch_size=8, gradient_accumulation_steps=2,
        num_generations=4, max_completion_length=256, max_steps=MAX_STEPS,
        learning_rate=1e-5, logging_steps=10, save_strategy="no", bf16=True,
        report_to=[], temperature=1.0)
    GRPOTrainer(model=model, reward_funcs=tool_reward, args=cfg,
                train_dataset=Dataset.from_list(rows), processing_class=tok).train()
    model.save_pretrained(V2_DIR)
    tok.save_pretrained(V2_DIR)
    print("GRPO_DONE", flush=True)

# ---------- 评测 (12-case: 7 工具 + 5 闲聊) ----------
CASES = [
    ("怎么新建 rust 项目", "lyv_knowledge", "rust"),
    ("帮我看看这张截图里是什么", "vnn_identify", None),
    ("帮我把这张图抠图去掉背景", "rembg_remove", None),
    ("生成一个产品落地页", "html_gen", None),
    ("写一段产品介绍文案", "llm_generate", None),
    ("把这个网页录制成视频", "html_render_video", None),
    ("这个视频多长", "video_info", None),
    ("你好呀", None, None),
    ("今天天气怎么样", None, None),
    ("讲个笑话", None, None),
    ("1 加 1 等于几", None, None),
    ("谢谢啦", None, None),
]

def evaluate(src):
    from transformers import AutoModelForCausalLM, AutoTokenizer
    import torch
    tools = load_tools()  # 评测用生产 schema (7 工具, 不含干扰)
    print(f"eval src: {src}", flush=True)
    tok = AutoTokenizer.from_pretrained(src)
    model = AutoModelForCausalLM.from_pretrained(src, torch_dtype=torch.bfloat16, device_map="cuda")
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
        print(f"[{'PASS' if ok else 'FAIL'}] {q} -> {call or text[:50]!r}", flush=True)
    print(f"FC 遵循度: {hits}/{len(CASES)} = {hits/len(CASES):.0%}  (src={src})", flush=True)

if __name__ == "__main__":
    stage = sys.argv[1] if len(sys.argv) > 1 else "all"
    if stage == "sft":
        sft()
    elif stage == "grpo":
        grpo()
    elif stage == "eval":
        src = sys.argv[2] if len(sys.argv) > 2 else MODEL_ID
        evaluate(MODEL_ID if src == "BASE" else src)
    else:
        sft(); grpo(); evaluate(V2_DIR)
