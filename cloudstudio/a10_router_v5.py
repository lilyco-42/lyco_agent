# v5：依据 DeepSeek-V4.1-Flash 技术报告的三条做法做实验
#  ① 可编程 verifier 做 reward（对齐报告 (problem, environment, verification system) 三元组）
#  ② 失败回放：把观测到的失败模式做成定向 RL prompt（新增信息，而非同义改写）
#  ③ 模型合并重启 RL（报告 5.1.2：model merging reinitializes successive RL runs）
# 初始化用 v4（续训），基线在同 run 内评测，保证可比。
import os, json, re, random, itertools

os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")
os.environ.setdefault("TRANSFORMERS_VERBOSITY", "error")

R = random.Random(2026)
HW_SYS = ("你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。"
          "只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。")
NOOP_OUT = "(无需调用硬件命令)"
V3, V4 = "/workspace/qwen3_router_v3", "/workspace/qwen3_router_v4"
MERGED, V5 = "/workspace/qwen3_router_merged", "/workspace/qwen3_router_v5"
MODEL_ID = "Qwen/Qwen3-0.6B"

OBJ = {"blue": ["蓝灯", "蓝色指示灯", "blue led", "台灯"], "green": ["绿灯", "电源灯", "green led"]}

T_TRAIN = {
 "led_on": ["帮我打开{obj}", "把{obj}打开", "{obj}开一下", "开{obj}", "让{obj}亮起来", "{obj}点亮",
            "麻烦开{obj}", "把{obj}调成开的", "{obj}打开下", "open the {obj}", "{obj}给我开开", "能不能把{obj}开了"],
 "led_off": ["帮我关掉{obj}", "把{obj}关了", "{obj}关一下", "关{obj}", "turn off the {obj}", "{obj}熄灭",
             "别让{obj}亮着", "{obj}帮我关掉", "把{obj}灭了", "关一下{obj}呗"],
 "led_status": ["{obj}现在什么状态", "看看{obj}开没开", "{obj}亮着吗", "is the {obj} on", "查一下{obj}状态", "{obj}是开的还是关的"],
 "led_blink": ["让{obj}闪{n}下", "{obj}闪烁{n}次", "{obj}闪一闪, {n}次", "blink {obj} {n} times", "{obj}给我闪{n}回"],
 "temp": ["多少度", "cpu 温度", "现在热不热", "看下温度", "板子温度多少", "热吗现在", "cpu 多少度",
          "处理器温度", "cpu 热不热", "温度多少了", "给个温度", "现在温度几度"],
 "cpu": ["cpu 频率", "现在多少主频", "负载怎么样", "cpu 状态", "cpu 现在跑多少", "主频多少", "cpu 占用高吗", "看看 cpu"],
 "mem": ["内存还剩多少", "内存占用", "还有多少内存", "看下内存用掉多少", "内存情况", "还剩多少 ram", "查一下内存"],
 "disk": ["磁盘还剩多少", "存储满了没", "看下磁盘", "硬盘还剩多少空间", "disk space", "还剩多少存储", "磁盘占用多少"],
 "fan": ["风扇调到{v}", "风扇转速{v}", "把风扇设为{v}", "风扇开到{v}", "风扇速度改成{v}", "fan speed {v}", "风扇给{v}"],
 "fan_status": ["风扇转多快", "看看风扇", "风扇状态", "风扇现在什么速度", "查风扇转速", "风扇开着吗", "fan rpm now", "风扇转速多少"],
 "gpio_get": ["读一下 gpio{cn} 的 {l} 号脚", "gpio line {l} 什么电平", "读 gpio chip{cn} line{l}", "gpio{cn} 第{l}脚读一下"],
 "gpio_set": ["把 gpio{cn} 的 {l} 号脚拉{v}", "gpio line {l} 设为 {v}", "gpio{cn} line{l} 输出 {v}", "gpio{cn} 第{l}脚设成{v}"],
 "info": ["板子什么型号", "系统信息", "什么板子", "看一下板子信息", "board info", "这是啥板子", "查一下硬件信息"],
}
T_HELD = {
 "led_on": ["劳驾把{obj}接通", "{obj}麻烦给打开", "帮我把那个{obj}点着"],
 "led_off": ["麻烦把{obj}断电", "{obj}我不想让它亮着", "把{obj}熄了吧"],
 "led_status": ["{obj}是开着的吗", "告诉我{obj}当前状态"],
 "led_blink": ["{obj}给我闪{n}下哈", "让{obj}眨{n}次眼"],
 "temp": ["摸着烫手吗", "核心温度报一下"], "cpu": ["处理器现在跑多少频率", "主频给个数"],
 "mem": ["还剩多少运行内存", "查看剩余运存"], "disk": ["硬盘爆了没有", "剩余存储查一下"],
 "fan": ["风机转速给到{v}", "把风扇调成{v}转"], "fan_status": ["风扇现在多大风速", "查一下散热风扇速度"],
 "gpio_get": ["看看 gpio{cn} 第{l}脚是高是低", "gpio chip{cn} 的 line{l} 读个值"],
 "gpio_set": ["gpio{cn} 的 {l} 脚给我拉到{v}", "让 gpio 第{l}号线变成{v}"], "info": ["这是块什么开发板", "查一下板卡信息"],
}
T_HARD = {   # heldB 弱线索（评测用，绝不进训练）
 "temp": ["板子烫不烫", "芯片现在凉快吗"], "cpu": ["处理器现在跑得快吗", "算力吃紧不"],
 "mem": ["运行内存还够用吗", "还剩多少地方能放临时数据"], "disk": ["还剩多少地方存东西", "空间快占完了吧"],
 "fan": ["散热那个给我拧到{v}", "让风扇呼呼转到{v}"], "fan_status": ["散热那个现在转得猛吗", "风扇响不响"],
 "info": ["告诉我这板子的来头", "这玩意儿是啥设备"],
 "led_on": ["把那盏{obj}给我弄亮", "{obj}该亮起来了"], "led_off": ["{obj}别亮了", "那盏{obj}给我掐了"],
 "led_status": ["{obj}现在是亮的不", "那{obj}有没有在亮"], "led_blink": ["让那{obj}眨{n}下眼", "{obj}哆嗦{n}次"],
}
# 失败回放：观测到的失败模式 —— 与 heldB 措辞不同，专补"弱线索/口语化"这一类
T_FAIL = {
 "temp": ["机子烫手不", "烤不烤得慌"],
 "cpu": ["处理器累不累", "现在算得快不"],
 "mem": ["还有地方塞数据没", "暂存区挤不挤"],
 "disk": ["能存的东西还多吗", "装不下了吧"],
 "fan_status": ["那个转的东西还在转吗", "有风吗现在"],
 "info": ["给我介绍下这台设备", "这是啥型号的"],
 "led_on": ["蓝的那个给我搞亮点", "把灯打开下"],
 "led_off": ["把灯关了吧", "灯不用了"],
 "led_status": ["灯现在亮着没", "灯开着还是关着"],
 "led_blink": ["让灯闪{n}下", "灯闪{n}次看看"],
 "gpio_get": ["读下 {l} 脚", "第 {l} 脚现在是啥状态"],
 "gpio_set": ["把 {l} 脚拉到{v}", "第 {l} 脚输出{v}"],
}
NOOP_TRAIN = ["讲个笑话", "今天几号", "写一首诗", "帮我算 1+1", "唱首歌", "写个爬虫", "推荐一部电影",
              "你是谁", "帮我订机票", "明天天气如何", "帮我写封邮件", "什么是黑洞", "帮我算一下 12*15",
              "把这段文字翻译成英文", "今天股市怎么样", "打开浏览器", "播放一首歌", "查一下快递",
              "把屏幕亮度调高", "帮我连上 wifi", "重启一下路由器", "给摄像头调个焦距", "把音量调大点", "蓝牙打开一下"]
NOOP_HELD = ["给我讲个冷笑话", "圆周率前三位是多少", "明天适合穿什么", "帮我起个英文名字", "推荐一部科幻片",
             "把显示器亮度调暗", "帮我配对蓝牙耳机"]

CAPS = list(T_TRAIN.keys())


def make_cmd(cap, kw):
    return {"led_on": f"hw led {kw['color']} on", "led_off": f"hw led {kw['color']} off",
            "led_status": f"hw led {kw['color']} status", "led_blink": f"hw led {kw['color']} blink {kw['n']}",
            "temp": "hw temp", "cpu": "hw cpu", "mem": "hw mem", "disk": "hw disk",
            "fan": f"hw fan {kw['v']}", "fan_status": "hw fan",
            "gpio_get": f"hw gpio get {kw['cn']} {kw['l']}",
            "gpio_set": f"hw gpio set {kw['cn']} {kw['l']} {kw['v']}", "info": "hw info"}[cap]


def slots_for(cap, held):
    if cap.startswith("led"):
        c = R.choice(list(OBJ)); kw = {"color": c, "obj": R.choice(OBJ[c]), "n": 0}
        if cap == "led_blink":
            kw["n"] = R.randint(8, 12) if held else R.randint(1, 7)
        return kw
    if cap == "fan": return {"v": R.randint(250, 255) if held else R.randint(30, 240)}
    if cap == "gpio_get": return {"cn": R.choice([0, 1]), "l": R.randint(301, 380) if held else R.randint(0, 300)}
    if cap == "gpio_set": return {"cn": R.choice([0, 1]), "l": R.randint(301, 380) if held else R.randint(0, 300), "v": R.choice([0, 1])}
    return {}


def gen(tpls, cap, held):
    kw = slots_for(cap, held)
    return [{"role": "user", "content": R.choice(tpls).format(**kw)},
            {"role": "assistant", "content": make_cmd(cap, kw)}]


# ---------- 数据集：训练 / 两档留出 ----------
train, heldA, heldB, seen = [], [], [], set()


def add_train(p, n=1):
    k = p[0]["content"]
    if k not in seen:
        seen.add(k)
        for _ in range(n):
            train.append(p)


for cap in CAPS:
    for _ in range(220): add_train(gen(T_TRAIN[cap], cap, False))
    for _ in range(220): add_train(gen(T_FAIL[cap], cap, False))      # 失败回放
for t in NOOP_TRAIN: add_train([{"role": "user", "content": t}, {"role": "assistant", "content": NOOP_OUT}], 3)
for cap in CAPS:
    for _ in range(40):
        p = gen(T_HELD[cap], cap, True)
        if p[0]["content"] not in seen: heldA.append(p)
for cap, tpls in T_HARD.items():
    for _ in range(30):
        p = gen(tpls, cap, True)
        if p[0]["content"] not in seen: heldB.append(p)
for t in NOOP_HELD:
    heldA.append([{"role": "user", "content": t}, {"role": "assistant", "content": NOOP_OUT}])
    heldB.append([{"role": "user", "content": t}, {"role": "assistant", "content": NOOP_OUT}])
R.shuffle(train); R.shuffle(heldA); R.shuffle(heldB)
leak = sum(1 for p in heldA + heldB if p[0]["content"] in seen)
print(f"train={len(train)} heldA={len(heldA)} heldB={len(heldB)} leak={leak}(须0)", flush=True)
assert leak == 0

import torch
from transformers import AutoModelForCausalLM, AutoTokenizer, Trainer, TrainingArguments

tok = AutoTokenizer.from_pretrained(MODEL_ID)
tok.pad_token = tok.eos_token


def build_prompt(user):
    return tok.apply_chat_template([{"role": "system", "content": HW_SYS}, {"role": "user", "content": user}],
                                   tokenize=False, add_generation_prompt=True, enable_thinking=False)


def encode(msgs):
    m = [{"role": "system", "content": HW_SYS}] + msgs
    full = tok.apply_chat_template(m, tokenize=False, add_generation_prompt=False, enable_thinking=False)
    prm = tok.apply_chat_template(m[:-1], tokenize=False, add_generation_prompt=True, enable_thinking=False)
    ids = tok(full, truncation=True, max_length=192)["input_ids"]
    pids = tok(prm, truncation=True, max_length=192)["input_ids"]
    n = min(len(pids), len(ids))
    lab = [-100] * n + ids[n:]
    if len(lab) != len(ids): lab = list(ids)
    return {"input_ids": ids, "labels": lab}


class Collator:
    def __init__(self, pad): self.pad = pad
    def __call__(self, fs):
        L = max(len(f["input_ids"]) for f in fs)
        ids, att, lab = [], [], []
        for f in fs:
            i, l = f["input_ids"], f["labels"]; pad = L - len(i)
            ids.append(i + [self.pad] * pad); att.append([1] * len(i) + [0] * pad); lab.append(l + [-100] * pad)
        return {"input_ids": torch.tensor(ids), "attention_mask": torch.tensor(att), "labels": torch.tensor(lab)}


# ---------- ⑥ verifier：可编程校验（对应报告的三元组里的 verification system） ----------
CMD_RE = re.compile(r"^hw (led (blue|green|blue led|green led) (on|off|status|blink \d+)|temp|cpu|mem|disk|fan( \d+)?|gpio (get|set) \d+ \d+( \d+)?|info)$")


def verify(pred, gold):
    """返回 0~1 奖励；分级给分，比精确匹配更适合 RL。"""
    p = re.sub(r"\s+", " ", pred.strip())
    p = p.split("\n")[0].strip()
    g = re.sub(r"\s+", " ", gold.strip())
    if NOOP_OUT in g:
        if NOOP_OUT in p: return 1.0
        return 0.0 if p.startswith("hw") else 0.3
    if p == g: return 1.0
    gc = g.split(); pc = p.split()
    if CMD_RE.match(p):
        if len(pc) >= 2 and len(gc) >= 2 and pc[:2] == gc[:2]:
            return 0.6                      # 能力对、槽位错
        return 0.2                          # 是合法 hw 命令但能力错
    if p.startswith("hw"): return 0.1       # 形态像但语法不合法
    return 0.0                              # 答了废话 / 该调没调


def evaluate(model, cases):
    model.eval(); okc = totc = okr = totr = 0
    for msgs in cases:
        q, gold = msgs[0]["content"], msgs[1]["content"]
        ids = tok(build_prompt(q), return_tensors="pt", add_special_tokens=False).to("cuda")
        with torch.no_grad():
            out = model.generate(**ids, max_new_tokens=32, do_sample=False, pad_token_id=tok.eos_token_id)
        pred = tok.decode(out[0][ids["input_ids"].shape[1]:], skip_special_tokens=True)
        s = verify(pred, gold)
        if NOOP_OUT in gold:
            totr += 1; okr += (s >= 0.9)
        else:
            totc += 1; okc += (s >= 0.9)
    return okc / max(totc, 1), okr / max(totr, 1), totc, totr


def load(p):
    m = AutoModelForCausalLM.from_pretrained(p)
    return m.to(torch.bfloat16).cuda()


def report(tag, m):
    a, r, tc, tr = evaluate(m, heldA)
    b, rb, tb, trb = evaluate(m, heldB)
    print(f"{tag}: heldA={a:.1%}({int(a*tc)}/{tc}) heldB={b:.1%}({int(b*tb)}/{tb}) reject={r:.0%}", flush=True)


# ---------- ① 基线 v4 ----------
print("\n=== baseline ===", flush=True)
m4 = load(V4); report("v4(baseline)", m4)

# ---------- ② 模型合并重启（报告 5.1.2）—— 内存友好版：safetensors 惰性逐张量读，避免同时持有两份 float32 ----------
print("\n=== model merging (v3 ⊕ v4) ===", flush=True)
MERGED_SAFE = os.path.join(MERGED, "model.safetensors")


def lean_merge(d3, d4, outdir):
    from safetensors import safe_open
    from safetensors.torch import save_file
    f3, f4 = os.path.join(d3, "model.safetensors"), os.path.join(d4, "model.safetensors")
    if not (os.path.exists(f3) and os.path.exists(f4)):
        raise FileNotFoundError("需要两边的 model.safetensors")
    os.makedirs(outdir, exist_ok=True)
    out = {}
    with safe_open(f3, framework="pt", device="cpu") as h3, safe_open(f4, framework="pt", device="cpu") as h4:
        k3 = set(h3.keys())
        for k in h4.keys():
            t4 = h4.get_tensor(k)
            if k in k3:
                t3 = h3.get_tensor(k)
                out[k] = ((t3.float() + t4.float()).mul(0.5)).to(t4.dtype)
                del t3
            else:
                out[k] = t4
            del t4
    save_file(out, os.path.join(outdir, "model.safetensors"))
    del out
    for fn in ("config.json", "generation_config.json", "tokenizer.json", "tokenizer_config.json",
               "vocab.json", "merges.txt", "added_tokens.json", "special_tokens_map.json", "chat_template.jinja"):
        src = os.path.join(d4, fn)
        if os.path.exists(src):
            import shutil; shutil.copy2(src, os.path.join(outdir, fn))
    return outdir


try:
    lean_merge(V3, V4, MERGED)
    mm = load(MERGED)
    report("v3⊕v4 merged", mm)
    del mm
    torch.cuda.empty_cache()
except Exception:
    import traceback; traceback.print_exc()

# ---------- ③ verifier-GRPO（从 v4 续训；无 vLLM） ----------
print("\n=== verifier-GRPO from v4 ===", flush=True)
del m4
torch.cuda.empty_cache()
from trl import GRPOConfig, GRPOTrainer

init = V4 if os.path.isdir(V4) else MODEL_ID
model = AutoModelForCausalLM.from_pretrained(init).to(torch.bfloat16).cuda()
gold_by_prompt = {}
rows = []
for m in train:
    p = build_prompt(m[0]["content"])
    rows.append({"prompt": p, "gold": m[1]["content"]})
    gold_by_prompt[p] = m[1]["content"]


def reward_funcs(completions, gold=None, **kw):
    return [verify(c, g) for c, g in zip(completions, gold)]


cfg = GRPOConfig(output_dir=V5, num_generations=6, per_device_train_batch_size=12,
                 gradient_accumulation_steps=1, max_completion_length=48, max_steps=150,
                 learning_rate=5e-6, logging_steps=20, save_strategy="no", bf16=True,
                 report_to=[], disable_tqdm=True, temperature=1.0)
tr = GRPOTrainer(model=model, reward_funcs=reward_funcs, args=cfg,
                 train_dataset=rows, processing_class=tok)
tr.train()
os.makedirs(V5, exist_ok=True)
model.save_pretrained(V5); tok.save_pretrained(V5)
print("V5_SAVED", flush=True)
report("v5(verifier-GRPO)", model)
print("V5_DONE", flush=True)
