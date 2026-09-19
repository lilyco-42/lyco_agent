# -*- coding: utf-8 -*-
"""fc_train_v3.py — 控变量版：只改「配方」，不动数据来源

对照 v1 (`fc_grpo_corpus.py`)：v1 直接 GRPO + 无负样本 + 评测与训练重叠。
v3 只改三处**同属"配方"**的东西（算一个变量：训练流程），数据仍来自我们自己的语料：
  1. 冷启动 SFT（DeepSeek-R1 教训：纯 RL 会做但不会好好表达）
  2. **assistant-only loss masking**（只训 assistant token，抄 XYZ-Aquila 教程）
  3. 冷启动 SFT 里**教会"不调用"**（负样本给自然语言回复）→ 治 闲聊误调
  4. **held-out 评测**（训练没见过的问法），分开报 工具选择 / abstention 两个指标

⚠️ 刻意不加 HF 数据（xlam-irrelevance / xlam-60k）：那是**另一个变量**，
   按 lyco 信条 5「横向对比必须控住变量」，留到 run B 单独验证。
   启用：USE_HF_IRRELEVANCE=1

用法: python fc_train_v3.py {sft|grpo|eval} [src]
"""
import json
import os
import re
import sys

MODEL_ID = "Qwen/Qwen3-0.6B"
SFT_DIR = "/workspace/qwen3_lyco_v3_sft"
V3_DIR = "/workspace/qwen3_lyco_v3"
TOOLS_JSON = "/workspace/tools_openai.json"
MAX_STEPS = int(os.environ.get("MAX_STEPS", "150"))
SYSTEM = "You are lyco, a helpful assistant. You can call tools."

def load_tools():
    if os.path.exists(TOOLS_JSON):
        return json.load(open(TOOLS_JSON, encoding="utf-8"))
    return [{"type": "function", "function": {"name": "lyv_knowledge", "description": "查询视频知识库",
             "parameters": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}}}]

# ---------------- 数据（训练 / 评测 严格不重叠） ----------------
# 训练: 7 工具 × 5 问法
TOOL_TRAIN = [
    ("怎么新建 rust 项目", "lyv_knowledge", "rust"),
    ("如何创建 rust 工程", "lyv_knowledge", "rust"),
    ("cargo new 之后要做什么", "lyv_knowledge", "cargo"),
    ("怎么运行项目", "lyv_knowledge", "run"),
    ("如何安装依赖", "lyv_knowledge", "install"),
    ("帮我看看这张截图里是什么", "vnn_identify", None),
    ("识别一下这个画面", "vnn_identify", None),
    ("这个界面是什么应用", "vnn_identify", None),
    ("这张图里是什么软件", "vnn_identify", None),
    ("帮我识图", "vnn_identify", None),
    ("帮我把这张图抠图去掉背景", "rembg_remove", None),
    ("这张图片去背", "rembg_remove", None),
    ("把图片背景去掉", "rembg_remove", None),
    ("抠出这张图的主体", "rembg_remove", None),
    ("去掉这张照片的背景", "rembg_remove", None),
    ("生成一个产品落地页", "html_gen", None),
    ("做一个单文件网页", "html_gen", None),
    ("帮我写个 HTML 页面", "html_gen", None),
    ("生成一个介绍页", "html_gen", None),
    ("做个落地页", "html_gen", None),
    ("写一段产品介绍文案", "llm_generate", None),
    ("帮我想个队名", "llm_generate", None),
    ("写一首诗", "llm_generate", None),
    ("帮润色一下这段话", "llm_generate", None),
    ("起个标题", "llm_generate", None),
    ("把这个网页录制成视频", "html_render_video", None),
    ("网页转成 mp4", "html_render_video", None),
    ("把这个页面渲染成视频", "html_render_video", None),
    ("录屏这个网页", "html_render_video", None),
    ("html 转视频", "html_render_video", None),
    ("这个视频多长", "video_info", None),
    ("查看视频分辨率帧率", "video_info", None),
    ("看下这个视频的时长", "video_info", None),
    ("视频的帧率是多少", "video_info", None),
    ("这个 mp4 的信息", "video_info", None),
]

# 训练: 负样本（该不调）—— 闲聊 + "像但要不到工具"的请求
NEG_TRAIN = [
    ("你好呀", "你好呀！有什么可以帮你的吗？"),
    ("今天天气怎么样", "我暂时没法联网查天气，你可以看下手机自带天气。"),
    ("讲个笑话", "程序员最讨厌两件事：写文档，和别人不写文档。"),
    ("谢谢啦", "不客气，随时找我。"),
    ("1 加 1 等于几", "等于 2。"),
    ("你叫什么名字", "我是 lyco，你的本地助手。"),
    ("周末去哪玩好", "看你喜欢热闹还是安静。"),
    ("晚安", "晚安，好好休息。"),
    ("周杰伦是谁", "他是华语流行歌手。"),
    ("你会唱歌吗", "我不会唱，但可以帮你写歌词。"),
    ("帮我订一张去北京的机票", "我没有订票能力，帮不上这个忙。"),
    ("今天股市怎么样", "我这边查不到行情数据。"),
    ("帮我给这张图加个滤镜", "我没有加滤镜的工具，只有抠图和识图。"),
    ("把这段视频上传到 B 站", "我没有上传能力。"),
    ("帮我发一封邮件给老板", "我没有发邮件的能力。"),
    ("为什么天是蓝的", "因为大气对阳光的瑞利散射，蓝光散射更强。"),
    ("给我推荐一部电影", "你喜欢什么类型？科幻可以看《星际穿越》。"),
    ("北京到上海多远", "直线距离大约 1000 多公里。"),
    ("帮我算算 23 乘 4", "23 乘 4 等于 92。"),
    ("怎么才能早睡", "固定作息、睡前少看手机，会比较有用。"),
    ("你好，你是谁", "我是 lyco，一个本地助手。"),
    ("今天几号", "我这边看不到系统日期。"),
    ("帮我写个请假条", "这个我可以帮你写，你想请几天？"),
    ("随便聊聊吧", "好啊，你想聊点什么？"),
    ("帮我翻译一下 hello world", "翻译成中文是「你好，世界」。"),
    ("这个游戏好玩吗", "看你喜欢什么类型，我可以帮你分析下。"),
    ("帮我建个微信群", "我没有创建群聊的能力。"),
    ("给我讲讲相对论", "简单说：时间和空间会随速度与引力变化。"),
    ("帮我删掉这个文件", "我没有删文件的能力，请在系统里操作。"),
    ("今天心情不好", "怎么了？愿意说说的话我听着。"),
]

# 评测: held-out（问法与训练**不重叠**）
EVAL_TOOL = [
    ("怎么初始化一个 rust 项目", "lyv_knowledge", "rust"),
    ("依赖怎么装", "lyv_knowledge", "install"),
    ("这张截图识别一下内容", "vnn_identify", None),
    ("抠图，去掉背景", "rembg_remove", None),
    ("生成一个单页网站", "html_gen", None),
    ("帮我写段宣传语", "llm_generate", None),
    ("把网页转成视频文件", "html_render_video", None),
    ("视频的分辨率是多少", "video_info", None),
]
EVAL_NEG = [
    ("哈喽", None, None),
    ("天气不错", None, None),
    ("谢谢", None, None),
    ("给我讲个段子", None, None),
    ("帮我买个东西", None, None),
    ("今天美元汇率多少", None, None),
    ("帮我预约理发", None, None),
    ("我睡不着", None, None),
]

def _synth_args(tool, q):
    if tool == "lyv_knowledge":
        return {"query": q}
    if tool in ("llm_generate", "html_gen"):
        return {"prompt": q}
    return {}

def reply_for(tool, q, neg_reply=None):
    if tool:
        return f'<tool_call>{json.dumps({"name": tool, "arguments": _synth_args(tool, q)}, ensure_ascii=False)}</tool_call>'
    return neg_reply or "好的。"

def _load_hf_irrelevance(limit=300):
    """可选：从 HF 拉 xlam-irrelevance-7.5k（run B 才开）。失败则静默跳过。"""
    try:
        from datasets import load_dataset
        ds = load_dataset("MadeAgents/xlam-irrelevance-7.5k", split="train")
        out = []
        for r in ds.select(range(min(limit, len(ds)))):
            q = r.get("query") or r.get("question") or ""
            if q and len(q) < 60:
                out.append((q, "没有可调用的工具。"))
        print(f"[hf] irrelevance 载入 {len(out)} 条", flush=True)
        return out
    except Exception as e:
        print(f"[hf] 跳过 irrelevance: {e}", flush=True)
        return []

# ---------------- 奖励 ----------------
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

def train_rows():
    rows = [(q, t, a, None) for q, t, a in TOOL_TRAIN]
    rows += [(q, None, None, r) for q, r in NEG_TRAIN]
    if os.environ.get("USE_HF_IRRELEVANCE") == "1":
        rows += [(q, None, None, r) for q, r in _load_hf_irrelevance()]
    return rows

# ---------------- 冷启动 SFT (assistant-only loss masking) ----------------
class PadCollator:
    def __init__(self, pad_id, max_len=768):
        self.pad_id = pad_id
        self.max_len = max_len

    def __call__(self, feats):
        import torch
        L = min(max(len(f["input_ids"]) for f in feats), self.max_len)
        ids, att, lab = [], [], []
        for f in feats:
            i = f["input_ids"][:L]
            l = f["labels"][:L]
            pad = L - len(i)
            ids.append(i + [self.pad_id] * pad)
            att.append([1] * len(i) + [0] * pad)
            lab.append(l + [-100] * pad)
        return {"input_ids": torch.tensor(ids), "attention_mask": torch.tensor(att),
                "labels": torch.tensor(lab)}

def _encode_masked(tok, tools, q, tool, neg_reply):
    msgs = [{"role": "system", "content": SYSTEM}, {"role": "user", "content": q}]
    assistant = reply_for(tool, q, neg_reply)
    full = tok.apply_chat_template(msgs + [{"role": "assistant", "content": assistant}],
                                   tools=tools, tokenize=False, enable_thinking=False)
    prompt = tok.apply_chat_template(msgs, tools=tools, add_generation_prompt=True,
                                     tokenize=False, enable_thinking=False)
    ids = tok(full)["input_ids"]
    pids = tok(prompt)["input_ids"]
    n = len(pids)
    # assistant-only: prompt 段 label = -100
    labels = [-100] * n + ids[n:]
    if len(labels) != len(ids):
        labels = list(ids)
    return {"input_ids": ids, "labels": labels}

def sft():
    from transformers import AutoModelForCausalLM, AutoTokenizer, Trainer, TrainingArguments
    import torch

    tools = load_tools()
    tok = AutoTokenizer.from_pretrained(MODEL_ID)
    tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(
        MODEL_ID, torch_dtype=torch.bfloat16, attn_implementation="sdpa").cuda()

    rows = train_rows()
    feats = [_encode_masked(tok, tools, q, t, r) for q, t, a, r in rows]
    n_tool = sum(1 for _, t, _, _ in rows if t)
    print(f"SFT 样本: {len(feats)} (工具 {n_tool} / 负样本 {len(feats)-n_tool}), assistant-only masking", flush=True)

    args = TrainingArguments(output_dir=SFT_DIR, num_train_epochs=3,
                             per_device_train_batch_size=4, learning_rate=1e-5,
                             logging_steps=5, save_strategy="no", bf16=True, report_to=[])
    Trainer(model=model, args=args, train_dataset=feats,
            data_collator=PadCollator(tok.pad_token_id)).train()
    model.save_pretrained(SFT_DIR)
    tok.save_pretrained(SFT_DIR)
    print("SFT_DONE", flush=True)

# ---------------- GRPO ----------------
def grpo():
    from datasets import Dataset
    from transformers import AutoModelForCausalLM, AutoTokenizer
    from trl import GRPOConfig, GRPOTrainer
    import torch

    tools = load_tools()
    src = SFT_DIR if os.path.exists(SFT_DIR + "/config.json") else MODEL_ID
    tok = AutoTokenizer.from_pretrained(src)
    tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(
        src, torch_dtype=torch.bfloat16, attn_implementation="sdpa").cuda()

    rows = [{"prompt": build_prompt(q, tok, tools), "expected_tool": t, "expected_arg": a}
            for q, t, a, _ in train_rows()]
    print(f"GRPO dataset: {len(rows)} prompts (src={src})", flush=True)
    cfg = GRPOConfig(output_dir=V3_DIR, per_device_train_batch_size=8,
                     gradient_accumulation_steps=2, num_generations=4,
                     max_completion_length=256, max_steps=MAX_STEPS, learning_rate=1e-5,
                     logging_steps=10, save_strategy="no", bf16=True, report_to=[], temperature=1.0)
    GRPOTrainer(model=model, reward_funcs=tool_reward, args=cfg,
                train_dataset=Dataset.from_list(rows), processing_class=tok).train()
    model.save_pretrained(V3_DIR)
    tok.save_pretrained(V3_DIR)
    print("GRPO_DONE", flush=True)

# ---------------- 评测 (held-out, 分指标) ----------------
def evaluate(src):
    from transformers import AutoModelForCausalLM, AutoTokenizer
    import torch
    tools = load_tools()
    print(f"eval src: {src}", flush=True)
    tok = AutoTokenizer.from_pretrained(src)
    model = AutoModelForCausalLM.from_pretrained(src, torch_dtype=torch.bfloat16, device_map="cuda")

    def run(cases):
        rows = []
        for q, et, ea in cases:
            inputs = tok(build_prompt(q, tok, tools), return_tensors="pt",
                         add_special_tokens=False).to("cuda")
            out = model.generate(**inputs, max_new_tokens=256, do_sample=False)
            text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False)
            call = parse_call(text)
            ok = (call is None) if et is None else bool(
                call and call.get("name") == et and
                (ea is None or ea in json.dumps(call.get("arguments", {}), ensure_ascii=False)))
            rows.append((q, et, ok, call))
        return rows

    tool_rows = run(EVAL_TOOL)
    neg_rows = run(EVAL_NEG)
    for q, et, ok, call in tool_rows + neg_rows:
        print(f"[{'PASS' if ok else 'FAIL'}] {q} -> {call or '(no call)'}", flush=True)
    ta = sum(1 for r in tool_rows if r[2])
    na = sum(1 for r in neg_rows if r[2])
    print(f"工具选择准确率 : {ta}/{len(tool_rows)} = {ta/len(tool_rows):.0%}", flush=True)
    print(f"abstention 准确率(不误调): {na}/{len(neg_rows)} = {na/len(neg_rows):.0%}", flush=True)
    print(f"总体: {(ta+na)}/{len(tool_rows)+len(neg_rows)} = {(ta+na)/(len(tool_rows)+len(neg_rows)):.0%}", flush=True)

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
        sft(); grpo(); evaluate(V3_DIR)
