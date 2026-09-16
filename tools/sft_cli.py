# -*- coding: utf-8 -*-
"""sft_cli.py — 训练「意图 → CLI 路由器」(省 API 开销的本地小模型)

数据: cli_datagen.py 生成的 train/eval.jsonl (真实 hw CLI 语料)
模型: Qwen3-0.6B (本地推理零 API 开销)
评测: eval.jsonl 上贪心解码 → 与金标命令**精确匹配**; 该不调样本要求不输出命令

用法: python sft_cli.py [data_dir]   (默认 ./cgidata)
"""
import json
import os
import sys

MODEL_ID = "Qwen/Qwen3-0.6B"
OUT = "/workspace/qwen3_router_v1"
DATA = sys.argv[1] if len(sys.argv) > 1 else "/workspace/cgidata"
SYSTEM = ("你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。"
          "只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。")


def load(path):
    out = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                out.append(json.loads(line)["messages"])
    return out


def encode(tok, msgs):
    """assistant-only loss: 只对命令部分算 loss"""
    full = tok.apply_chat_template(msgs, tokenize=False, add_generation_prompt=False,
                                   enable_thinking=False)
    prompt = tok.apply_chat_template(msgs[:-1], tokenize=False,
                                     add_generation_prompt=True, enable_thinking=False)
    ids = tok(full, truncation=True, max_length=256)["input_ids"]
    pids = tok(prompt, truncation=True, max_length=256)["input_ids"]
    n = min(len(pids), len(ids))
    labels = [-100] * n + ids[n:]
    if len(labels) != len(ids):
        labels = list(ids)
    return {"input_ids": ids, "labels": labels}


class Collator:
    def __init__(self, pad_id):
        self.pad_id = pad_id

    def __call__(self, feats):
        import torch
        L = max(len(f["input_ids"]) for f in feats)
        ids, att, lab = [], [], []
        for f in feats:
            i, l = f["input_ids"], f["labels"]
            pad = L - len(i)
            ids.append(i + [self.pad_id] * pad)
            att.append([1] * len(i) + [0] * pad)
            lab.append(l + [-100] * pad)
        return {"input_ids": torch.tensor(ids), "attention_mask": torch.tensor(att),
                "labels": torch.tensor(lab)}


def main():
    import torch
    from transformers import (AutoModelForCausalLM, AutoTokenizer, Trainer,
                              TrainingArguments)

    tok = AutoTokenizer.from_pretrained(MODEL_ID)
    tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(
        MODEL_ID, torch_dtype=torch.bfloat16, attn_implementation="sdpa").cuda()

    train = [encode(tok, m) for m in load(os.path.join(DATA, "train.jsonl"))]
    print(f"train={len(train)}", flush=True)

    args = TrainingArguments(output_dir=OUT, num_train_epochs=3,
                             per_device_train_batch_size=8, learning_rate=2e-5,
                             logging_steps=25, save_strategy="no", bf16=True,
                             report_to=[])
    Trainer(model=model, args=args, train_dataset=train,
            data_collator=Collator(tok.pad_token_id)).train()
    model.save_pretrained(OUT)
    tok.save_pretrained(OUT)
    print("SFT_DONE", flush=True)

    # ---- 评测: 精确匹配 + 该不调 ----
    import re
    evals = load(os.path.join(DATA, "eval.jsonl"))
    ok_cmd, ok_noop, total_cmd, total_noop = 0, 0, 0, 0
    for msgs in evals:
        q = msgs[0]["content"]
        gold = msgs[1]["content"]
        prompt = tok.apply_chat_template(
            [{"role": "system", "content": SYSTEM},
             {"role": "user", "content": q}],
            tokenize=False, add_generation_prompt=True, enable_thinking=False)
        ids = tok(prompt, return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**ids, max_new_tokens=64, do_sample=False,
                             pad_token_id=tok.eos_token_id)
        pred = tok.decode(out[0][ids["input_ids"].shape[1]:],
                          skip_special_tokens=True).strip()
        pred = re.sub(r"\s+", " ", pred)
        gold_n = re.sub(r"\s+", " ", gold)
        if "(无需调用硬件命令)" in gold:
            total_noop += 1
            ok_noop += "(无需调用硬件命令)" in pred or not pred.startswith("hw")
        else:
            total_cmd += 1
            ok_cmd += pred == gold_n
            if pred != gold_n:
                print(f"  MISS: {q!r} → {pred!r} (gold {gold_n!r})", flush=True)
    acc_cmd = ok_cmd / max(total_cmd, 1)
    rej = ok_noop / max(total_noop, 1)
    print(f"EVAL: 命令精确匹配 {ok_cmd}/{total_cmd} = {acc_cmd:.0%} | "
          f"该不调拒绝 {ok_noop}/{total_noop} = {rej:.0%}", flush=True)
    print("EVAL_DONE", flush=True)


if __name__ == "__main__":
    main()
