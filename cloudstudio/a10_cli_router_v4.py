# A10/V100 CloudStudio: CLI router v3.
# v2 教训: 英文 shell 占 80% 会冲淡中文硬件任务并把拒绝率打到 20% -> v4 纯中文 + 说法族扩充,
# 并新增「弱线索困难集」(不出现温度/内存等明显关键词) 真测语义泛化。全部在内核里跑。
import os, json, re, time, random

os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")
os.environ.setdefault("TRANSFORMERS_VERBOSITY", "error")

R = random.Random(2026)
HW_SYSTEM = ("你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。"
             "只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。")
NOOP_OUT = "(无需调用硬件命令)"
V1 = "/workspace/qwen3_router_v1"
OUT4 = "/workspace/qwen3_router_v4"

print("=== CLI router v4 (pure zh, richer paraphrase families) ===", flush=True)

OBJ = {"blue": ["蓝灯", "蓝色指示灯", "blue led", "台灯", "lamp", "床头灯"],
       "green": ["绿灯", "电源灯", "green led"]}

# ---- 训练用说法族 (尽量覆盖: 祈使/疑问/口语/中英夹杂/带语气词/省略)
T_TRAIN = {
    "led_on": ["帮我打开{obj}", "把{obj}打开", "{obj}开一下", "开{obj}", "让{obj}亮起来",
               "{obj}点亮", "麻烦开{obj}", "把{obj}调成开的", "{obj}打开下", "open the {obj}",
               "{obj}给我开开", "能不能把{obj}开了", "{obj}打开呗", "把那个{obj}开一下",
               "{obj}帮开一下", "劳烦打开{obj}"],
    "led_off": ["帮我关掉{obj}", "把{obj}关了", "{obj}关一下", "关{obj}", "turn off the {obj}",
                "{obj}熄灭", "别让{obj}亮着", "{obj}帮我关掉", "把{obj}灭了", "{obj}关掉吧",
                "关一下{obj}呗", "让{obj}别亮了", "{obj}给我关了"],
    "led_status": ["{obj}现在什么状态", "看看{obj}开没开", "{obj}亮着吗", "is the {obj} on",
                   "查一下{obj}状态", "{obj}是开的还是关的", "现在{obj}是亮是灭",
                   "{obj}状态如何", "告诉我{obj}现在的情况"],
    "led_blink": ["让{obj}闪{n}下", "{obj}闪烁{n}次", "{obj}闪一闪, {n}次", "blink {obj} {n} times",
                  "{obj}给我闪{n}回", "{obj}连续闪{n}下", "把{obj}设成闪{n}次",
                  "{obj}帮我闪{n}下呗"],
    "temp": ["多少度", "cpu 温度", "现在热不热", "看下温度", "板子温度多少", "热吗现在",
             "cpu 多少度", "处理器温度", "cpu 热不热", "温度多少了", "给个温度",
             "现在温度几度", "热不热啊", "温度"],
    "cpu": ["cpu 频率", "现在多少主频", "负载怎么样", "cpu 状态", "cpu 现在跑多少",
            "主频多少", "cpu 占用高吗", "看看 cpu", "处理器现在忙不忙", "cpu 负载"],
    "mem": ["内存还剩多少", "内存占用", "还有多少内存", "看下内存用掉多少", "内存情况",
            "还剩多少 ram", "memory 还剩多少", "查一下内存", "内存用了多少"],
    "disk": ["磁盘还剩多少", "存储满了没", "看下磁盘", "硬盘还剩多少空间", "disk space",
             "还剩多少存储", "磁盘占用多少", "存储还剩多少"],
    "fan": ["风扇调到{v}", "风扇转速{v}", "把风扇设为{v}", "风扇开到{v}", "风扇速度改成{v}",
            "把风扇转速设置成{v}", "fan speed {v}", "风扇给{v}"],
    "fan_status": ["风扇转多快", "看看风扇", "风扇状态", "风扇现在什么速度", "查风扇转速",
                   "风扇开着吗", "fan rpm now", "风扇转速多少", "看看风扇转不转"],
    "gpio_get": ["读一下 gpio{cn} 的 {l} 号脚", "gpio line {l} 什么电平", "读 gpio chip{cn} line{l}",
                 "gpio{cn} 第{l}脚读一下", "看看 gpio{cn} line{l} 的值", "gpio line {l} 是高的还是低的"],
    "gpio_set": ["把 gpio{cn} 的 {l} 号脚拉{v}", "gpio line {l} 设为 {v}", "gpio{cn} line{l} 输出 {v}",
                 "gpio{cn} 第{l}脚设成{v}", "让 gpio line{l} 变成 {v}", "设 gpio{cn} line{l} 为 {v}"],
    "info": ["板子什么型号", "系统信息", "什么板子", "看一下板子信息", "board info",
             "这是啥板子", "查一下硬件信息", "告诉我板子型号"],
}

# ---- 留出集A: 未见说法 (有一定线索)
T_HELD = {
    "led_on": ["劳驾把{obj}接通", "{obj}麻烦给打开", "帮我把那个{obj}点着"],
    "led_off": ["麻烦把{obj}断电", "{obj}我不想让它亮着", "把{obj}熄了吧"],
    "led_status": ["{obj}是开着的吗", "告诉我{obj}当前状态"],
    "led_blink": ["{obj}给我闪{n}下哈", "让{obj}眨{n}次眼"],
    "temp": ["摸着烫手吗", "核心温度报一下"],
    "cpu": ["处理器现在跑多少频率", "主频给个数"],
    "mem": ["还剩多少运行内存", "查看剩余运存"],
    "disk": ["硬盘爆了没有", "剩余存储查一下"],
    "fan": ["风机转速给到{v}", "把风扇调成{v}转"],
    "fan_status": ["风扇现在多大风速", "查一下散热风扇速度"],
    "gpio_get": ["看看 gpio{cn} 第{l}脚是高是低", "gpio chip{cn} 的 line{l} 读个值"],
    "gpio_set": ["gpio{cn} 的 {l} 脚给我拉到{v}", "让 gpio 第{l}号线变成{v}"],
    "info": ["这是块什么开发板", "查一下板卡信息"],
}

# ---- 留出集B(困难): 弱线索, 不出现 温度/内存/磁盘/cpu/gpio 等明显词, 逼模型学语义
T_HARD = {
    "temp": {"tpl": ["板子烫不烫", "芯片现在凉快吗"], "kw": {}},
    "cpu": {"tpl": ["处理器现在跑得快吗", "算力吃紧不"], "kw": {}},
    "mem": {"tpl": ["运行内存还够用吗", "还剩多少地方能放临时数据"], "kw": {}},
    "disk": {"tpl": ["还剩多少地方存东西", "空间快占完了吧"], "kw": {}},
    "fan": {"tpl": ["散热那个给我拧到{v}", "让风扇呼呼转到{v}"], "kw": {}},
    "fan_status": {"tpl": ["散热那个现在转得猛吗", "风扇响不响"], "kw": {}},
    "info": {"tpl": ["告诉我这板子的来头", "这玩意儿是啥设备"], "kw": {}},
    "led_on": {"tpl": ["把那盏{obj}给我弄亮", "{obj}该亮起来了"], "kw": {}},
    "led_off": {"tpl": ["{obj}别亮了", "那盏{obj}给我掐了"], "kw": {}},
    "led_status": {"tpl": ["{obj}现在是亮的不", "那{obj}有没有在亮"], "kw": {}},
    "led_blink": {"tpl": ["让那{obj}眨{n}下眼", "{obj}哆嗦{n}次"], "kw": {}},
}


def make_cmd(cap, kw):
    if cap == "led_on":     return f"hw led {kw['color']} on"
    if cap == "led_off":    return f"hw led {kw['color']} off"
    if cap == "led_status": return f"hw led {kw['color']} status"
    if cap == "led_blink":  return f"hw led {kw['color']} blink {kw['n']}"
    if cap == "temp":       return "hw temp"
    if cap == "cpu":        return "hw cpu"
    if cap == "mem":        return "hw mem"
    if cap == "disk":       return "hw disk"
    if cap == "fan":        return f"hw fan {kw['v']}"
    if cap == "fan_status": return "hw fan"
    if cap == "gpio_get":   return f"hw gpio get {kw['cn']} {kw['l']}"
    if cap == "gpio_set":   return f"hw gpio set {kw['cn']} {kw['l']} {kw['v']}"
    if cap == "info":       return "hw info"
    raise ValueError(cap)


def slots_for(cap, held):
    if cap.startswith("led"):
        color = R.choice(list(OBJ))
        kw = {"color": color, "obj": R.choice(OBJ[color]), "n": 0}
        if cap == "led_blink":
            kw["n"] = R.randint(8, 12) if held else R.randint(1, 7)
        return kw
    if cap == "fan":
        return {"v": R.randint(250, 255) if held else R.randint(30, 240)}
    if cap == "gpio_get":
        return {"cn": R.choice([0, 1]), "l": R.randint(301, 380) if held else R.randint(0, 300)}
    if cap == "gpio_set":
        return {"cn": R.choice([0, 1]),
                "l": R.randint(301, 380) if held else R.randint(0, 300),
                "v": R.choice([0, 1])}
    return {}


def gen(cap, tpl_list, held):
    tpl = R.choice(tpl_list)
    kw = slots_for(cap, held)
    return [{"role": "user", "content": tpl.format(**kw)},
            {"role": "assistant", "content": make_cmd(cap, kw)}]


# 拒绝类: 明显无关 + 「看着像硬件其实不支持」的硬负例
NOOP_TRAIN = ["讲个笑话", "今天几号", "你喜欢什么音乐", "写一首诗", "帮我算 1+1", "唱首歌",
              "写个爬虫", "推荐一部电影", "1 加 1 等于几", "你是谁", "讲个鬼故事",
              "帮我订机票", "明天天气如何", "hello", "翻译一句话", "帮我写封邮件",
              "今天心情不好", "什么是黑洞", "帮我算一下 12*15", "把这段文字翻译成英文",
              "今天股市怎么样", "帮我写个排序算法", "打开浏览器", "给我讲讲量子力学",
              "播放一首歌", "设置一个闹钟", "帮我订外卖", "查一下快递",
              # 硬负例 (像硬件但本 CLI 不支持)
              "把屏幕亮度调高", "帮我连上 wifi", "重启一下路由器", "给摄像头调个焦距",
              "把音量调大点", "蓝牙打开一下"]
NOOP_HELD = ["给我讲个冷笑话", "圆周率前三位是多少", "明天适合穿什么", "帮我起个英文名字",
             "推荐一部科幻片", "把显示器亮度调暗", "帮我配对蓝牙耳机"]

CAPS = list(T_TRAIN.keys())
hw_train, held_a, held_b, seen = [], [], [], set()

# v3 缺陷: 无槽位能力 (temp/cpu/mem/disk/info/fan_status) 只有十几条模板, 去重后唯一样本过少,
# 而 heldB 弱线索题主要考这些能力 -> 用口语后缀/语气词做说法倍增 (标签不变)。
# 注意: 后缀必须【同时】加到拒绝类上, 否则模型会学到"带语气词=要出命令"的伪相关。
SUF = ["", "", "", "谢谢", "吧", "呢", "?", "。", "啊", "呀"]


def is_zh(s):
    return any("\u4e00" <= ch <= "\u9fff" for ch in s)


def aug(q):
    return q + R.choice(SUF) if is_zh(q) else q


for cap in CAPS:
    for _ in range(900):
        kw = slots_for(cap, False)
        tpl = R.choice(T_TRAIN[cap])
        q = aug(tpl.format(**kw))
        if q not in seen:
            seen.add(q)
            hw_train.append([{"role": "user", "content": q},
                             {"role": "assistant", "content": make_cmd(cap, kw)}])
for t in NOOP_TRAIN:
    for _ in range(15):      # v3 用 40 偏高 -> 过度拒答, 压到 15
        q = aug(t)
        if q not in seen:
            seen.add(q)
            hw_train.append([{"role": "user", "content": q},
                             {"role": "assistant", "content": NOOP_OUT}])

for cap in CAPS:
    for _ in range(40):
        p = gen(cap, T_HELD[cap], True)
        if p[0]["content"] not in seen:
            held_a.append(p)
for cap, spec in T_HARD.items():
    for _ in range(30):
        p = gen(cap, spec["tpl"], True)
        if p[0]["content"] not in seen:
            held_b.append(p)
for t in NOOP_HELD:
    held_a.append([{"role": "user", "content": t}, {"role": "assistant", "content": NOOP_OUT}])
    held_b.append([{"role": "user", "content": t}, {"role": "assistant", "content": NOOP_OUT}])

R.shuffle(hw_train); R.shuffle(held_a); R.shuffle(held_b)
leak = sum(1 for p in held_a + held_b if any(p[0]["content"] == q[0]["content"] for q in hw_train))
print(f"train={len(hw_train)}  heldA(未见说法)={len(held_a)}  heldB(弱线索困难)={len(held_b)}", flush=True)
print(f"[check] leak={leak} (must be 0)", flush=True)

# ---------------- model ----------------
import torch
from transformers import AutoModelForCausalLM, AutoTokenizer, Trainer, TrainingArguments

MODEL_ID = "Qwen/Qwen3-0.6B"
tok = AutoTokenizer.from_pretrained(MODEL_ID)
tok.pad_token = tok.eos_token


def build_prompt(system, user):
    return tok.apply_chat_template(
        [{"role": "system", "content": system}, {"role": "user", "content": user}],
        tokenize=False, add_generation_prompt=True, enable_thinking=False)


def encode(system, msgs):
    m = [{"role": "system", "content": system}] + msgs
    full = tok.apply_chat_template(m, tokenize=False, add_generation_prompt=False, enable_thinking=False)
    prompt = tok.apply_chat_template(m[:-1], tokenize=False, add_generation_prompt=True, enable_thinking=False)
    ids = tok(full, truncation=True, max_length=256)["input_ids"]
    pids = tok(prompt, truncation=True, max_length=256)["input_ids"]
    n = min(len(pids), len(ids))
    labels = [-100] * n + ids[n:]
    if len(labels) != len(ids):
        labels = list(ids)
    return {"input_ids": ids, "labels": labels}


class Collator:
    def __init__(self, pad_id): self.pad_id = pad_id
    def __call__(self, feats):
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


def evaluate(model, cases):
    model.eval()
    ok_cmd = tot_cmd = ok_rej = tot_rej = 0
    for msgs in cases:
        q, gold = msgs[0]["content"], msgs[1]["content"]
        ids = tok(build_prompt(HW_SYSTEM, q), return_tensors="pt", add_special_tokens=False).to("cuda")
        with torch.no_grad():
            out = model.generate(**ids, max_new_tokens=48, do_sample=False, pad_token_id=tok.eos_token_id)
        pred = re.sub(r"\s+", " ", tok.decode(out[0][ids["input_ids"].shape[1]:],
                                              skip_special_tokens=True).strip())
        gold_n = re.sub(r"\s+", " ", gold)
        if NOOP_OUT in gold_n:
            tot_rej += 1
            ok_rej += (NOOP_OUT in pred) or (not pred.startswith("hw"))
        else:
            tot_cmd += 1
            ok_cmd += (pred == gold_n)
    return (ok_cmd / max(tot_cmd, 1), ok_rej / max(tot_rej, 1), tot_cmd, tot_rej)


# ---- v1 基线 (两个集都测, 同一 run 内可比)
if os.path.isdir(V1):
    try:
        v1 = AutoModelForCausalLM.from_pretrained(V1).to(torch.bfloat16).cuda()
        for name, cases in (("heldA", held_a), ("heldB", held_b)):
            a, r, tc, tr = evaluate(v1, cases)
            print(f"v1 {name}: cmd={a:.1%} ({int(a*tc)}/{tc})  reject={r:.1%} ({int(r*tr)}/{tr})", flush=True)
        del v1
        torch.cuda.empty_cache()
    except Exception:
        import traceback; traceback.print_exc()
        torch.cuda.empty_cache()

# ---- 训练 v4 (纯中文, repo 配方: lr 2e-5 / 4 epoch)
rows = [encode(HW_SYSTEM, m) for m in hw_train]
print(f"\ntrain rows={len(rows)}", flush=True)
model = AutoModelForCausalLM.from_pretrained(MODEL_ID, torch_dtype=torch.bfloat16,
                                             attn_implementation="sdpa").cuda()
args = TrainingArguments(output_dir=OUT4, num_train_epochs=4, per_device_train_batch_size=16,
                         learning_rate=2e-5, logging_steps=200, save_strategy="no", bf16=True,
                         report_to=[], lr_scheduler_type="cosine", warmup_ratio=0.03,
                         disable_tqdm=True)
Trainer(model=model, args=args, train_dataset=rows,
        data_collator=Collator(tok.pad_token_id)).train()
os.makedirs(OUT4, exist_ok=True)
model.save_pretrained(OUT4)
tok.save_pretrained(OUT4)
print("V4_SAVED", flush=True)

for name, cases in (("heldA", held_a), ("heldB", held_b)):
    a, r, tc, tr = evaluate(model, cases)
    print(f"v4 {name}: cmd={a:.1%} ({int(a*tc)}/{tc})  reject={r:.1%} ({int(r*tr)}/{tr})", flush=True)
print("V4_DONE", flush=True)
