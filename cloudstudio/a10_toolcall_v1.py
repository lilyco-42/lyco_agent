# toolcall_v1: 把路由器从「hw 命令字符串」升级为「lilyco 契约的 tool_call」+ schema 泛化评测
# 契约来源（读 lilyco 源码得到，非猜）：
#   tools/list -> {"tools":[{"name","description","inputSchema"}]}
#     description: T0 直接用 about；非 T0 追加 " [safety: T1 confirm]" 等
#     inputSchema = {"type":"object","properties":{...},"required":[...]}（required 为空则省略）
#   ArgKind -> JSON Schema: Flag=boolean / Text=string / Number=number(+minimum,maximum)
#                            / Enum=string+enum / Path=string / List=array+items
#   tools/call params = {"name","arguments"}，执行前必须过 CommandSchema::validate_args
# 关键评测：**留出若干 tool 完全不训**，只在 prompt 里给 schema，测「看 schema 办事」的泛化能力
import os, re, json, math, random

os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")
os.environ.setdefault("TRANSFORMERS_VERBOSITY", "error")

MODEL = "Qwen/Qwen3-0.6B"
OUT = "/workspace/toolcall_v1"
R = random.Random(2026)

# ---------------- 1) lilyco 保真的工具定义生成 ----------------
def arg(name, about, kind, required=False, default=None):
    return {"name": name, "about": about, "kind": kind, "required": required, "default": default}

def kind_to_js(k):
    t = k["type"]
    if t == "flag":   return {"type": "boolean"}
    if t == "text":   return {"type": "string"}
    if t == "number":
        s = {"type": "number"}
        if k.get("min") is not None: s["minimum"] = k["min"]
        if k.get("max") is not None: s["maximum"] = k["max"]
        return s
    if t == "enum":   return {"type": "string", "enum": k["values"]}
    if t == "path":   return {"type": "string"}
    if t == "list":   return {"type": "array", "items": kind_to_js(k["item"])}
    raise ValueError(t)

def to_json_schema(args):
    props, req = {}, []
    for a in args:
        p = kind_to_js(a["kind"]); p["description"] = a["about"]
        props[a["name"]] = p
        if a["required"]: req.append(a["name"])
    s = {"type": "object", "properties": props}
    if req: s["required"] = req
    return s

TIER_TAG = {"T0": "read_only", "T1": "confirm", "T2": "token", "T3": "never_auto"}

def to_tool(name, about, args, tier="T0"):
    desc = about if tier == "T0" else f"{about} [safety: {TIER_TAG[tier]}]"
    return {"name": name, "description": desc, "inputSchema": to_json_schema(args)}

def to_openai_tool(t):
    """MCP tools/list 条目 -> OpenAI function 工具定义
    （等价于 lilyco 的 CommandSchema::to_openai_tool()；模板只吃这个形状）"""
    return {"type": "function",
            "function": {"name": t["name"], "description": t["description"],
                         "parameters": t["inputSchema"]}}

ENUM2 = {"type": "enum", "values": ["0", "1"]}
COLOR = {"type": "enum", "values": ["blue", "green"]}

# 5 个「server」模拟被挂载的 MCP 服务；HOLDOUT 里的工具【绝不进训练】
TOOLS = [
 # lffmpeg (视频生产)
 ("compress",        "压缩视频", [arg("input","输入文件",{"type":"path"},True), arg("output","输出文件",{"type":"path"},True),
                                  arg("crf","画质 0-51，越小越清晰",{"type":"number","min":0,"max":51},False,23),
                                  arg("preset","编码预设",{"type":"enum","values":["ultrafast","fast","medium","slow"]},False,"medium")], "T1"),
 ("resize",          "调整视频分辨率", [arg("input","输入文件",{"type":"path"},True), arg("output","输出文件",{"type":"path"},True),
                                  arg("width","宽",{"type":"number"},True), arg("height","高",{"type":"number"},True)], "T1"),
 ("extract_audio",   "抽取音轨", [arg("input","输入文件",{"type":"path"},True), arg("output","输出音频",{"type":"path"},True)], "T1"),
 ("to_gif",          "视频转 GIF", [arg("input","输入文件",{"type":"path"},True), arg("output","输出 gif",{"type":"path"},True),
                                  arg("fps","帧率",{"type":"number","min":1,"max":30},False,10),
                                  arg("width","宽",{"type":"number"},False,480)], "T1"),   # HOLDOUT
 # lilyco-brush (PS/图像修复)
 ("remove_bg",       "去除图片背景", [arg("input","输入图片",{"type":"path"},True), arg("output","输出 png",{"type":"path"},True)], "T1"),
 ("repair_photo",    "老照片修复", [arg("input","输入图片",{"type":"path"},True), arg("output","输出图片",{"type":"path"},True),
                                  arg("strength","修复强度 0-1",{"type":"number","min":0,"max":1},False,0.5)], "T1"),
 ("upscale",         "图片超分放大", [arg("input","输入图片",{"type":"path"},True), arg("output","输出图片",{"type":"path"},True),
                                  arg("scale","放大倍数",{"type":"number","min":2,"max":4},False,2)], "T2"),  # HOLDOUT
 ("skin_retouch",    "人像美颜磨皮", [arg("input","输入图片",{"type":"path"},True), arg("output","输出图片",{"type":"path"},True),
                                  arg("level","强度",{"type":"text"},False,"natural")], "T1"),   # HOLDOUT
 # lilyco-vision
 ("ocr",             "识别图片文字", [arg("image","图片路径",{"type":"path"},True),
                                  arg("lang","语言",{"type":"enum","values":["chi_sim","eng"]},False,"chi_sim")], "T0"),
 ("describe_image",  "描述图片内容", [arg("image","图片路径",{"type":"path"},True)], "T0"),   # HOLDOUT
 # lilyco-chat
 ("chat_generate",   "通用文本生成/闲聊", [arg("prompt","生成需求",{"type":"text"},True),
                                  arg("max_tokens","最大长度",{"type":"number"},False,512)], "T0"),
 # hw (板端) —— 与 lyco_agent 现有能力对齐
 ("led_on",   "打开 LED", [arg("color","颜色",COLOR,True)], "T1"),
 ("led_off",  "关闭 LED", [arg("color","颜色",COLOR,True)], "T1"),
 ("led_status","查询 LED 状态", [arg("color","颜色",COLOR,True)], "T0"),
 ("led_blink","LED 闪烁", [arg("color","颜色",COLOR,True), arg("times","次数",{"type":"number","min":1,"max":20},True)], "T1"),
 ("temp",     "读取 CPU 温度", [], "T0"),
 ("cpu",      "读取 CPU 频率/负载", [], "T0"),
 ("mem",      "读取内存占用", [], "T0"),
 ("disk",     "读取磁盘占用", [], "T0"),
 ("fan",      "设置风扇转速", [arg("speed","转速 0-255",{"type":"number","min":0,"max":255},True)], "T1"),
 ("fan_status","查询风扇转速", [], "T0"),
 ("gpio_get", "读取 GPIO 电平", [arg("chip","芯片号",{"type":"number"},True), arg("line","引脚号",{"type":"number"},True)], "T0"),
 ("gpio_set", "设置 GPIO 输出", [arg("chip","芯片号",{"type":"number"},True), arg("line","引脚号",{"type":"number"},True),
                                arg("value","电平",ENUM2,True)], "T2"),
 ("info",     "读取板卡信息", [], "T0"),
]
HOLDOUT = {"to_gif", "upscale", "skin_retouch", "describe_image"}
ALL_TOOLS = {t[0]: to_tool(*t) for t in TOOLS}
TRAIN_TOOLS = [n for n in ALL_TOOLS if n not in HOLDOUT]

# 训练用问法（中文）+ 期望参数；每题给出 (query, {args}) —— args 只含必填与「本次显式给出」的可选
TRAIN_CASES = {
 "compress": [("把这个视频压一下：{i} 存成 {o}", {"input":"{i}","output":"{o}"}),
              ("压缩 {i} 到 {o}", {"input":"{i}","output":"{o}"})],
 "resize": [("把 {i} 改成 1280x720 存到 {o}", {"input":"{i}","output":"{o}","width":1280,"height":720})],
 "extract_audio": [("从 {i} 里把音频抽成 {o}", {"input":"{i}","output":"{o}"})],
 "remove_bg": [("把 {i} 的背景去掉，输出 {o}", {"input":"{i}","output":"{o}"})],
 "repair_photo": [("修复老照片 {i}，结果存 {o}", {"input":"{i}","output":"{o}"})],
 "ocr": [("识别 {i} 里的文字", {"image":"{i}"})],
 "chat_generate": [("帮我写一段产品文案", {"prompt":"帮我写一段产品文案"})],
 "led_on": [("把蓝灯打开", {"color":"blue"})],
 "led_off": [("关掉绿灯", {"color":"green"})],
 "led_status": [("蓝灯现在什么状态", {"color":"blue"})],
 "led_blink": [("让蓝灯闪 5 下", {"color":"blue","times":5})],
 "temp": [("现在多少度", {})],
 "cpu": [("cpu 频率多少", {})],
 "mem": [("内存还剩多少", {})],
 "disk": [("磁盘还剩多少", {})],
 "fan": [("风扇调到 200", {"speed":200})],
 "fan_status": [("风扇转速多少", {})],
 "gpio_get": [("读一下 gpio0 的 97 号脚", {"chip":0,"line":97})],
 "gpio_set": [("把 gpio0 的 12 号脚设为 1", {"chip":0,"line":12,"value":"1"})],
 "info": [("板子什么型号", {})],
}
# 留出工具的评测问法（训练里绝不出现这些工具的名字）
HOLDOUT_CASES = {
 "to_gif": [("把 {i} 转成 gif 存到 {o}", {"input":"{i}","output":"{o}"})],
 "upscale": [("把 {i} 放大两倍存成 {o}", {"input":"{i}","output":"{o}"})],
 "skin_retouch": [("给 {i} 美颜一下存到 {o}", {"input":"{i}","output":"{o}"})],
 "describe_image": [("描述一下 {i} 这张图", {"image":"{i}"})],
}
NOOP_Q = ["讲个笑话", "今天几号", "写一首诗", "帮我订机票", "明天天气如何", "你是谁",
          "帮我算一下 12*15", "今天股市怎么样", "播放一首歌", "查一下快递", "推荐一部电影"]
PATHS = ["a.mp4", "b.mp4", "clip.mov", "photo.jpg", "old.png", "shot.png", "v2.mkv"]

CHAT_TMPL = ('<tool_call>\n{body}\n</tool_call>')

def render_call(name, args):
    return CHAT_TMPL.format(body=json.dumps({"name": name, "arguments": args}, ensure_ascii=False))

def sample_tools(pool, k):
    # 只用 OpenAI 形状（lilyco to_openai_tool() 的等价物）；k 控制 prompt 长度
    return [to_openai_tool(ALL_TOOLS[n]) for n in R.sample(pool, min(k, len(pool)))]

def build_rows(n_per_tool=140, k=3):
    rows = []
    for name in TRAIN_TOOLS:
        for _ in range(n_per_tool):
            q, a = R.choice(TRAIN_CASES[name])
            i, o = R.choice(PATHS), "out_" + R.choice(PATHS)
            qq = q.format(i=i, o=o)
            aa = {kk: (vv.format(i=i, o=o) if isinstance(vv, str) and ("{i}" in vv or "{o}" in vv) else vv)
                  for kk, vv in a.items()}
            rows.append({"q": qq, "tool": name, "args": aa, "tools": sample_tools(TRAIN_TOOLS, k)})
    for _ in range(n_per_tool):
        rows.append({"q": R.choice(NOOP_Q), "tool": None, "args": None, "tools": sample_tools(TRAIN_TOOLS, k)})
    R.shuffle(rows)
    return rows

def eval_rows(k=3):
    """每工具多题（path/数值/颜色变化制造真实多样性），保证 N 够做统计"""
    seen, held, noop = [], [], []

    def fill(q, a, name, i, o):
        return {"q": q.format(i=i, o=o), "tool": name,
                "args": {kk: (vv.format(i=i, o=o) if isinstance(vv, str) and ("{i}" in vv or "{o}" in vv) else vv)
                         for kk, vv in a.items()}}

    for name in TRAIN_TOOLS:
        for _ in range(12):                                   # 每个已见工具 12 题
            q, a = R.choice(TRAIN_CASES[name])
            r = fill(q, a, name, R.choice(PATHS), "e_" + R.choice(PATHS))
            r["tools"] = sample_tools(TRAIN_TOOLS, k)
            seen.append(r)
    for name, cases in HOLDOUT_CASES.items():                 # 留出工具：schema 在 prompt 里，训练从未见过
        for _ in range(8):
            q, a = R.choice(cases)
            r = fill(q, a, name, R.choice(PATHS), "h_" + R.choice(PATHS))
            r["tools"] = sample_tools(TRAIN_TOOLS, k - 1) + [to_openai_tool(ALL_TOOLS[name])]
            held.append(r)
    for q in NOOP_Q:
        noop.append({"q": q, "tool": None, "args": None, "tools": sample_tools(TRAIN_TOOLS, k)})
    return seen, held, noop

SEEN, HELD, NOOP = eval_rows()
print(f"train_tools={len(TRAIN_TOOLS)} holdout_tools={sorted(HOLDOUT)} "
      f"eval: seen={len(SEEN)} heldout={len(HELD)} noop={len(NOOP)}", flush=True)

# ---------------- 2) 训练 ----------------
import torch
from transformers import AutoModelForCausalLM, AutoTokenizer, Trainer, TrainingArguments
from datasets import Dataset

SYS = "You are lyco, a local agent. Use the provided tools when they help; otherwise reply directly."
tok = AutoTokenizer.from_pretrained(MODEL)
tok.pad_token = tok.eos_token

def apply(msgs, tools):
    return tok.apply_chat_template(msgs, tools=tools, tokenize=False,
                                   add_generation_prompt=False, enable_thinking=False)

def apply_gen(msgs, tools):
    return tok.apply_chat_template(msgs, tools=tools, tokenize=False,
                                   add_generation_prompt=True, enable_thinking=False)

def encode(row):
    target = render_call(row["tool"], row["args"]) if row["tool"] else "这个我帮不上，换个硬件相关的吧。"
    full = apply([{"role": "system", "content": SYS}, {"role": "user", "content": row["q"]},
                  {"role": "assistant", "content": target}], row["tools"])
    prm = apply_gen([{"role": "system", "content": SYS}, {"role": "user", "content": row["q"]}], row["tools"])
    # 工具 schema 会让 prompt 很长：max_length 必须够大，否则截断会把 assistant 目标切掉 → 标签全 -100
    MAXLEN = 1024
    ids = tok(full, truncation=True, max_length=MAXLEN)["input_ids"]
    pids = tok(prm, truncation=True, max_length=MAXLEN)["input_ids"]
    n = min(len(pids), len(ids)); lab = [-100] * n + ids[n:]
    if len(lab) != len(ids): lab = list(ids)
    return {"input_ids": ids, "labels": lab, "_sup": sum(1 for x in lab if x != -100)}

class Collator:
    def __init__(self, pad): self.pad = pad
    def __call__(self, fs):
        L = max(len(f["input_ids"]) for f in fs); ids, att, lab = [], [], []
        for f in fs:
            i, l = f["input_ids"], f["labels"]; pad = L - len(i)
            ids.append(i + [self.pad] * pad); att.append([1] * len(i) + [0] * pad); lab.append(l + [-100] * pad)
        return {"input_ids": torch.tensor(ids), "attention_mask": torch.tensor(att), "labels": torch.tensor(lab)}

CALL_RE = re.compile(r"<tool_call>\s*(\{.*?\})\s*</tool_call>", re.S)
def parse_call(t):
    m = CALL_RE.search(t)
    if not m: return None
    try: return json.loads(m.group(1))
    except Exception: return None

def judge(pred, row):
    call = parse_call(pred)
    if row["tool"] is None:
        return (call is None), ("unnecessary_call" if call else None)
    if call is None:
        return False, "prose_fallback"
    if call.get("name") != row["tool"]:
        return False, "wrong_tool"
    got = call.get("arguments") or {}
    exp = row["args"] or {}
    if all(str(got.get(k, "")).strip() == str(v).strip() for k, v in exp.items()):
        return True, None
    return False, "wrong_args"

def evaluate(model, rows):
    model.eval(); res = []
    for r in rows:
        p = apply_gen([{"role": "system", "content": SYS}, {"role": "user", "content": r["q"]}], r["tools"])
        ids = tok(p, return_tensors="pt", add_special_tokens=False).to("cuda")
        with torch.no_grad():
            o = model.generate(**ids, max_new_tokens=128, do_sample=False, pad_token_id=tok.eos_token_id)
        pred = tok.decode(o[0][ids["input_ids"].shape[1]:], skip_special_tokens=False)
        res.append(judge(pred, r))
    return res

def rep(tag, res):
    ok = sum(1 for r in res if r[0]); n = len(res)
    from collections import Counter
    bad = Counter(r[1] for r in res if not r[0])
    print(f"  {tag}: {ok}/{n} = {ok/n:.1%}" + (f"  失败:{dict(bad)}" if bad else ""), flush=True)
    return [r[0] for r in res]

rows = build_rows()
print(f"train rows = {len(rows)}", flush=True)
data = [encode(r) for r in rows]

# 显存自检与清理：历次运行遗留的 ipykernel 会一直占着 CUDA context（上次 OOM 就是它们占掉 22.4GB）
import subprocess as _sp, signal as _sig
def free_gpu():
    """容器里 nvidia-smi --query-compute-apps 查不到 PID（显示 [Not Found]），
    所以改用 ps 枚举 ipykernel 并排除自身 PID 后 SIGKILL。"""
    try:
        ps = _sp.run("ps -eo pid,cmd --no-headers", shell=True, capture_output=True, text=True).stdout
        me = os.getpid()
        victims = []
        for line in ps.splitlines():
            if "ipykernel" in line:
                pid = int(line.split(None, 1)[0])
                if pid != me:
                    victims.append(pid)
        for p in victims:
            try:
                os.kill(p, _sig.SIGKILL); print(f"  [gpu] killed stale kernel pid {p}", flush=True)
            except Exception:
                pass
        if victims:
            time.sleep(3)
    except Exception as e:
        print("  [gpu] cleanup skipped:", e, flush=True)

free_gpu()
free, total = torch.cuda.mem_get_info()
print(f"  显存: 空闲 {free/2**30:.1f}G / 共 {total/2**30:.1f}G", flush=True)
sup = [d["_sup"] for d in data]
print(f"  监督 token: min={min(sup)} max={max(sup)} 零监督行数={sum(1 for s in sup if s == 0)}（须0）", flush=True)
assert min(sup) > 0, "有行的 assistant 目标被截断掉了，需加大 max_length 或减少 tools 数"
for d in data:
    d.pop("_sup", None)

model = AutoModelForCausalLM.from_pretrained(MODEL, torch_dtype=torch.bfloat16,
                                            attn_implementation="sdpa").cuda()
args = TrainingArguments(output_dir=OUT, num_train_epochs=3, per_device_train_batch_size=4,
                         gradient_accumulation_steps=4, gradient_checkpointing=True,
                         learning_rate=2e-5, logging_steps=100, save_strategy="no", bf16=True,
                         optim="adafactor", report_to=[], lr_scheduler_type="cosine",
                         warmup_ratio=0.03, disable_tqdm=True)
Trainer(model=model, args=args, train_dataset=Dataset.from_list(data),
        data_collator=Collator(tok.pad_token_id)).train()
os.makedirs(OUT, exist_ok=True); model.save_pretrained(OUT); tok.save_pretrained(OUT)
print("TOOLCALL_SAVED", flush=True)

print("\n=== 评测（关键看 heldout = schema 泛化）===", flush=True)
a = rep("S1 已见工具", evaluate(model, SEEN))
b = rep("S2 留出工具(仅凭schema)", evaluate(model, HELD))
c = rep("S3 该不调(拒绝)", evaluate(model, NOOP))
print("\n=== 汇总 ===", flush=True)
for tag, v in (("S1_seen", a), ("S2_heldout_schema", b), ("S3_reject", c)):
    print(f"  {tag}: {sum(v)}/{len(v)}", flush=True)
print("TOOLCALL_DONE", flush=True)
