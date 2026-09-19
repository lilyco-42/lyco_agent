# -*- coding: utf-8 -*-
"""fc_train_v4.py — 泛化版 FC 训练（用户方向纠正: 大量 CLI 训练 + 不要内置工具）

与 v3 的根本区别:
  v3: 手写 65 条 + **硬编码 10 工具** → 只认自家工具 (泛化差)
  v4: **大规模真实语料** + **工具来自每条数据自己**(任意工具集) → 泛化

数据源 (均已在 A10 实测可拉, 见 docs/pivot-generalization-2026-09-16.md):
  hermes-function-calling-v1  → 任意工具的 FC 监督 (核心)
  xlam-irrelevance-7.5k       → 该不调
  Nemotron-Terminal-Corpus    → 真实 CLI 轨迹 → 映射为 shell_exec 调用 (CLI 训练)
  我们自己的域样本(小比例)     → 保住既有域行为不回退

评测: held-out = **训练未见的 hermes 工具类别** (泛化) + 我们的域套件 (不回退)

用法: python fc_train_v4.py {sft|grpo|eval} [src]     (env: N_HERMES/N_IRREL/N_TERM/N_DOMAIN)
"""
import json
import os
import re
import sys

MODEL_ID = "Qwen/Qwen3-0.6B"
SFT_DIR = "/workspace/qwen3_lyco_v4_sft"
V4_DIR = "/workspace/qwen3_lyco_v4"
TOOLS_JSON = "/workspace/tools_openai.json"
SYSTEM = "You are lyco, a helpful assistant. You can call tools."
N_HERMES = int(os.environ.get("N_HERMES", "1500"))
N_IRREL = int(os.environ.get("N_IRREL", "1500"))
N_TERM = int(os.environ.get("N_TERM", "1200"))
N_DOMAIN = int(os.environ.get("N_DOMAIN", "0"))
MAX_STEPS = int(os.environ.get("MAX_STEPS", "200"))

CALL_RE = re.compile(r"<tool_call>\s*(\{.*?\})\s*</tool_call>", re.S)

def parse_call(text):
    m = CALL_RE.search(text or "")
    if m:
        try:
            return json.loads(m.group(1))
        except Exception:
            return None
    return None

# ---------------- 语料加载 ----------------
def load_hermes(n):
    from datasets import load_dataset
    ds = load_dataset("NousResearch/hermes-function-calling-v1", "func_calling_singleturn", split="train")
    out = []
    for r in ds.select(range(min(n, len(ds)))):
        tools = r.get("tools") or []
        conv = r.get("conversations") or []
        user = next((c.get("value") for c in conv if c.get("from") in ("human", "user")), None)
        asst = next((c.get("value") for c in conv if c.get("from") in ("gpt", "assistant")), None)
        if user and asst and tools:
            call = parse_call(asst)
            out.append({"query": user, "tools": tools, "target": asst.strip(),
                        "expected_tool": (call or {}).get("name")})
    return out

def load_irrelevance(n):
    from datasets import load_dataset
    ds = load_dataset("MadeAgents/xlam-irrelevance-7.5k", split="train")
    out = []
    for r in ds.select(range(min(n, len(ds)))):
        if (r.get("answers") or []):
            continue  # 只要"该不调"的
        out.append({"query": r.get("query", ""), "tools": r.get("tools") or [],
                    "target": "抱歉，没有合适的工具可以处理这个请求。", "expected_tool": None})
    return out

def load_terminal(n):
    """真实 CLI 轨迹 → shell_exec 调用 (近似映射, 见文档)"""
    from datasets import load_dataset
    try:
        ds = load_dataset("nvidia/Nemotron-Terminal-Corpus", "skill_based_mixed", split="train")
    except Exception as e:
        print(f"[term] 跳过: {e}", flush=True)
        return []
    tools = [{"type": "function", "function": {
        "name": "shell_exec", "description": "执行 shell 命令",
        "parameters": {"type": "object", "properties": {"command": {"type": "string"}},
                       "required": ["command"]}}}]
    out = []
    for r in ds.select(range(min(n, len(ds)))):
        conv = r.get("conversations") or []
        task = None
        cmds = []
        for c in conv:
            role = c.get("role") or c.get("from")
            val = c.get("content") or c.get("value") or ""
            if role in ("user", "system") and task is None and len(val) < 600:
                task = val
            if role in ("assistant", "gpt") and val:
                for line in val.splitlines():
                    line = line.strip()
                    if line and not line.startswith(("#", "```")) and len(line) < 200:
                        cmds.append(line)
                        break
        if task and cmds:
            target = f'<tool_call>{json.dumps({"name": "shell_exec", "arguments": {"command": cmds[0]}}, ensure_ascii=False)}</tool_call>'
            out.append({"query": task, "tools": tools, "target": target, "expected_tool": "shell_exec"})
    return out

def load_domain(n):
    if n <= 0:
        return []
    here = os.path.dirname(os.path.abspath(__file__))
    rows = []
    try:
        sys.path.insert(0, here)
        import fc_train_v3 as v3  # 复用域样本 (含 10 工具)
        tools = json.load(open(TOOLS_JSON, encoding="utf-8")) if os.path.exists(TOOLS_JSON) else v3.load_tools()
        for q, t, a, nr in v3.train_rows()[:n]:
            target = v3.reply_for(t, q, nr)
            rows.append({"query": q, "tools": tools, "target": target, "expected_tool": t})
    except Exception as e:
        print(f"[domain] 跳过: {e}", flush=True)
    return rows

def build_all():
    rows = []
    for name, fn, n in [("hermes", load_hermes, N_HERMES), ("irrel", load_irrelevance, N_IRREL),
                        ("terminal", load_terminal, N_TERM), ("domain", load_domain, N_DOMAIN)]:
        try:
            part = fn(n)
            print(f"[data] {name}: {len(part)}", flush=True)
            rows += part
        except Exception as e:
            print(f"[data] {name} 失败: {str(e)[:160]}", flush=True)
    return rows

# ---------------- SFT (assistant-only masking, 每条数据自带 tools) ----------------
class PadCollator:
    def __init__(self, pad_id, max_len=1024):
        self.pad_id, self.max_len = pad_id, max_len

    def __call__(self, feats):
        import torch
        L = min(max(len(f["input_ids"]) for f in feats), self.max_len)
        ids, att, lab = [], [], []
        for f in feats:
            i, l = f["input_ids"][:L], f["labels"][:L]
            pad = L - len(i)
            ids.append(i + [self.pad_id] * pad)
            att.append([1] * len(i) + [0] * pad)
            lab.append(l + [-100] * pad)
        return {"input_ids": torch.tensor(ids), "attention_mask": torch.tensor(att),
                "labels": torch.tensor(lab)}

def _encode(tok, row):
    msgs = [{"role": "system", "content": SYSTEM}, {"role": "user", "content": row["query"]}]
    tools = row["tools"] or None
    full = tok.apply_chat_template(msgs + [{"role": "assistant", "content": row["target"]}],
                                   tools=tools, tokenize=False, enable_thinking=False)
    prompt = tok.apply_chat_template(msgs, tools=tools, add_generation_prompt=True,
                                     tokenize=False, enable_thinking=False)
    ids = tok(full, truncation=True, max_length=1024)["input_ids"]
    pids = tok(prompt)["input_ids"]
    n = min(len(pids), len(ids))
    labels = [-100] * n + ids[n:]
    if len(labels) != len(ids):
        labels = list(ids)
    return {"input_ids": ids, "labels": labels}

def sft():
    from transformers import AutoModelForCausalLM, AutoTokenizer, Trainer, TrainingArguments
    import torch

    tok = AutoTokenizer.from_pretrained(MODEL_ID)
    tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(
        MODEL_ID, torch_dtype=torch.bfloat16, attn_implementation="sdpa").cuda()

    rows = build_all()
    print(f"SFT 总样本: {len(rows)}", flush=True)
    feats = [_encode(tok, r) for r in rows]
    args = TrainingArguments(output_dir=SFT_DIR, num_train_epochs=1,
                             per_device_train_batch_size=8, learning_rate=1e-5,
                             logging_steps=20, save_strategy="no", bf16=True, report_to=[])
    Trainer(model=model, args=args, train_dataset=feats,
            data_collator=PadCollator(tok.pad_token_id)).train()
    model.save_pretrained(SFT_DIR)
    tok.save_pretrained(SFT_DIR)
    print("SFT_DONE", flush=True)

def build_prompt(q, tok, tools):
    return tok.apply_chat_template(
        [{"role": "system", "content": SYSTEM}, {"role": "user", "content": q}],
        tools=tools or None, add_generation_prompt=True, enable_thinking=False, tokenize=False)

def tool_reward(completions, expected_tool=None, **kw):
    ets = expected_tool if isinstance(expected_tool, list) else [expected_tool] * len(completions)
    out = []
    for comp, et in zip(completions, ets):
        call = parse_call(comp)
        if et is None:
            out.append(1.0 if call is None else 0.0)
        elif call and call.get("name") == et:
            out.append(1.0)
        elif call is None:
            out.append(0.2)          # 该调却不调: 低分但不零 (梯度更平滑)
        else:
            out.append(0.0)
    return out

def grpo():
    from datasets import Dataset
    from transformers import AutoModelForCausalLM, AutoTokenizer
    from trl import GRPOConfig, GRPOTrainer
    import torch

    src = SFT_DIR if os.path.exists(SFT_DIR + "/config.json") else MODEL_ID
    tok = AutoTokenizer.from_pretrained(src)
    tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(
        src, torch_dtype=torch.bfloat16, attn_implementation="sdpa").cuda()

    rows = build_all()
    ds = Dataset.from_list([{"prompt": build_prompt(r["query"], tok, r["tools"]),
                             "expected_tool": r["expected_tool"]} for r in rows])
    print(f"GRPO dataset: {len(ds)} (src={src})", flush=True)
    cfg = GRPOConfig(output_dir=V4_DIR, per_device_train_batch_size=4,
                     gradient_accumulation_steps=4, num_generations=4,
                     max_completion_length=256, max_steps=MAX_STEPS, learning_rate=1e-5,
                     logging_steps=10, save_strategy="no", bf16=True, report_to=[], temperature=1.0)
    GRPOTrainer(model=model, reward_funcs=tool_reward, args=cfg,
                train_dataset=ds, processing_class=tok).train()
    model.save_pretrained(V4_DIR)
    tok.save_pretrained(V4_DIR)
    print("GRPO_DONE", flush=True)

# ---------------- 评测: 泛化(未见工具) + 域回归 ----------------
def evaluate(src):
    from transformers import AutoModelForCausalLM, AutoTokenizer
    import torch
    print(f"eval src: {src}", flush=True)
    tok = AutoTokenizer.from_pretrained(src)
    model = AutoModelForCausalLM.from_pretrained(src, torch_dtype=torch.bfloat16, device_map="cuda")

    # (A) 泛化: 取 hermes **后段**(未参与训练) 的工具
    gen = load_hermes(N_HERMES + 300)[N_HERMES:]
    ok = 0
    for r in gen[:100]:
        inputs = tok(build_prompt(r["query"], tok, r["tools"]), return_tensors="pt",
                     add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=128, do_sample=False)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False)
        call = parse_call(text)
        hit = bool(call and call.get("name") == r["expected_tool"])
        ok += hit
        print(f"[{'PASS' if hit else 'FAIL'}] {r['query'][:40]} -> {call.get('name') if call else None} (期望 {r['expected_tool']})", flush=True)
    print(f"泛化(未见工具) 工具选择: {ok}/{min(100,len(gen))} = {ok/max(1,min(100,len(gen))):.0%}", flush=True)

    # (B) 域回归: 我们的 10 工具套件
    tj = json.load(open(TOOLS_JSON, encoding="utf-8")) if os.path.exists(TOOLS_JSON) else None
    dom = [("怎么启动 paper 服务器", "lyv_knowledge"), ("帮我看看这张截图", "vnn_identify"),
           ("帮我把图抠图去背景", "rembg_remove"), ("生成一个产品落地页", "html_gen"),
           ("写一段产品介绍文案", "llm_generate"), ("把这个网页录成视频", "html_render_video"),
           ("这个视频多长", "video_info"), ("帮我把这个服务启动起来", "shell_exec"),
           ("帮我写个启动脚本到文件", "file_write"), ("每天早上8点自动启动服务器", "schedule"),
           ("你好呀", None), ("今天天气怎么样", None)]
    dok = 0
    for q, et in dom:
        inputs = tok(build_prompt(q, tok, tj), return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=128, do_sample=False)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False)
        call = parse_call(text)
        hit = (call is None) if et is None else bool(call and call.get("name") == et)
        dok += hit
        print(f"[{'PASS' if hit else 'FAIL'}] {q} -> {call.get('name') if call else None}", flush=True)
    print(f"域回归(10工具): {dok}/{len(dom)} = {dok/len(dom):.0%}", flush=True)

if __name__ == "__main__":
    st = sys.argv[1] if len(sys.argv) > 1 else "all"
    if st == "sft":
        sft()
    elif st == "grpo":
        grpo()
    elif st == "eval":
        s = sys.argv[2] if len(sys.argv) > 2 else MODEL_ID
        evaluate(MODEL_ID if s == "BASE" else s)
    else:
        sft(); grpo(); evaluate(V4_DIR)
