# v7：受控尺寸对照 (Qwen3-0.6B vs Qwen3-1.7B) + 评测补硬（配对 McNemar + 失败分桶）
# lyco 铁律12：只变参数量，其余全锁死（数据/eval/超参/解码/环境）
#  - 同族基座(Qwen3) -> tokenizer/chat template/thinking 语义均不变
#  - 数据生成器、eval 集、lr/epochs/batch/bf16、greedy+temp0 全部一致
#  - eval 集先于 train 生成，与 train 规模解耦
import os, re, json, math, random

os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")
os.environ.setdefault("TRANSFORMERS_VERBOSITY", "error")

HW_SYS = ("你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。"
          "只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。")
NOOP_OUT = "(无需调用硬件命令)"
SMALL, BIG, MERGED = "Qwen/Qwen3-0.6B", "Qwen/Qwen3-1.7B", "/workspace/qwen3_router_merged"
OUT_S, OUT_B = "/workspace/router_sft_0.6b", "/workspace/router_sft_1.7b"

Re = random.Random(2026)   # eval 集专用
Rt = random.Random(7)      # train 专用

OBJ = {"blue": ["蓝灯", "蓝色指示灯", "blue led", "台灯"], "green": ["绿灯", "电源灯", "green led"]}

T_TRAIN = {
 "led_on": ["帮我打开{obj}", "把{obj}打开", "{obj}开一下", "开{obj}", "让{obj}亮起来", "{obj}点亮",
            "麻烦开{obj}", "把{obj}调成开的", "{obj}打开下", "open the {obj}", "{obj}给我开开"],
 "led_off": ["帮我关掉{obj}", "把{obj}关了", "{obj}关一下", "关{obj}", "turn off the {obj}", "{obj}熄灭",
             "别让{obj}亮着", "{obj}帮我关掉", "把{obj}灭了", "关一下{obj}呗"],
 "led_status": ["{obj}现在什么状态", "看看{obj}开没开", "{obj}亮着吗", "is the {obj} on", "查一下{obj}状态",
                "{obj}是开的还是关的"],
 "led_blink": ["让{obj}闪{n}下", "{obj}闪烁{n}次", "{obj}闪一闪, {n}次", "blink {obj} {n} times", "{obj}给我闪{n}回"],
 "temp": ["多少度", "cpu 温度", "现在热不热", "看下温度", "板子温度多少", "热吗现在", "cpu 多少度",
          "处理器温度", "cpu 热不热", "温度多少了", "给个温度"],
 "cpu": ["cpu 频率", "现在多少主频", "负载怎么样", "cpu 状态", "cpu 现在跑多少", "主频多少", "cpu 占用高吗"],
 "mem": ["内存还剩多少", "内存占用", "还有多少内存", "看下内存用掉多少", "内存情况", "还剩多少 ram", "查一下内存"],
 "disk": ["磁盘还剩多少", "存储满了没", "看下磁盘", "硬盘还剩多少空间", "disk space", "还剩多少存储"],
 "fan": ["风扇调到{v}", "风扇转速{v}", "把风扇设为{v}", "风扇开到{v}", "风扇速度改成{v}", "fan speed {v}"],
 "fan_status": ["风扇转多快", "看看风扇", "风扇状态", "风扇现在什么速度", "查风扇转速", "风扇开着吗", "fan rpm now"],
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
T_HARD = {
 "temp": ["板子烫不烫", "芯片现在凉快吗"], "cpu": ["处理器现在跑得快吗", "算力吃紧不"],
 "mem": ["运行内存还够用吗", "还剩多少地方能放临时数据"], "disk": ["还剩多少地方存东西", "空间快占完了吧"],
 "fan": ["散热那个给我拧到{v}", "让风扇呼呼转到{v}"], "fan_status": ["散热那个现在转得猛吗", "风扇响不响"],
 "info": ["告诉我这板子的来头", "这玩意儿是啥设备"],
 "led_on": ["把那盏{obj}给我弄亮", "{obj}该亮起来了"], "led_off": ["{obj}别亮了", "那盏{obj}给我掐了"],
 "led_status": ["{obj}现在是亮的不", "那{obj}有没有在亮"], "led_blink": ["让那{obj}眨{n}下眼", "{obj}哆嗦{n}次"],
}
T_FAIL = {
 "temp": ["机子烫手不", "烤不烤得慌"], "cpu": ["处理器累不累", "现在算得快不"],
 "mem": ["还有地方塞数据没", "暂存区挤不挤"], "disk": ["能存的东西还多吗", "装不下了吧"],
 "fan_status": ["那个转的东西还在转吗", "有风吗现在"], "fan": ["风扇给我转到{v}", "把风扇弄成{v}"],
 "info": ["给我介绍下这台设备", "这是啥型号的"],
 "led_on": ["蓝的那个给我搞亮点", "把灯打开下"], "led_off": ["把灯关了吧", "灯不用了"],
 "led_status": ["灯现在亮着没", "灯开着还是关着"], "led_blink": ["让灯闪{n}下", "灯闪{n}次看看"],
 "gpio_get": ["读下 {l} 脚", "第 {l} 脚现在是啥状态"], "gpio_set": ["把 {l} 脚拉到{v}", "第 {l} 脚输出{v}"],
}
# 扩充留出集：v7 的教训 —— 无槽位能力模板太少 + 整句去重 → heldA/heldB 被砍到 184/94，N 不足以做检验
T_HELD_EXTRA = {
 "led_on": ["请把{obj}通电打开", "{obj}麻烦点亮一下"],
 "led_off": ["{obj}请关停", "帮我把{obj}熄掉"],
 "led_status": ["{obj}目前是亮着的状态么"],
 "led_blink": ["{obj}请连闪{n}次"],
 "temp": ["温度读数给我", "现在芯片多少摄氏度"],
 "cpu": ["cpu 主频报一下", "处理器频率是多少"],
 "mem": ["剩余内存有多少", "memory 占用情况"],
 "disk": ["磁盘剩余空间多少", "存储使用情况如何"],
 "fan": ["把风扇转速设定为{v}", "风扇调到 {v} 转"],
 "fan_status": ["风扇当前转速是多少", "散热风扇在转么"],
 "gpio_get": ["读取 gpio{cn} 的第 {l} 引脚电平", "看一下 chip{cn} 的 line{l}"],
 "gpio_set": ["把 gpio{cn} 第 {l} 脚置为{v}", "设置 chip{cn} line{l} = {v}"],
 "info": ["板卡型号是什么", "给我系统硬件信息"],
}
T_HARD_EXTRA = {
 "temp": ["机身热得厉害吗"], "cpu": ["处理速度现在怎么样"], "mem": ["临时内存还宽裕么"],
 "disk": ["还有空位放文件吗"], "fan": ["给散热调成{v}档"], "fan_status": ["散热还在工作么"],
 "info": ["这台机器是什么来路"], "led_on": ["把那个{obj}弄亮堂点"], "led_off": ["{obj}可以灭了"],
 "led_status": ["{obj}现在亮没亮"], "led_blink": ["{obj}闪{n}下看看"],
}
for k, v in T_HELD_EXTRA.items():
    T_HELD.setdefault(k, []).extend(v)
for k, v in T_HARD_EXTRA.items():
    T_HARD.setdefault(k, []).extend(v)

NOOP_TRAIN = ["讲个笑话", "今天几号", "写一首诗", "帮我算 1+1", "唱首歌", "写个爬虫", "推荐一部电影", "你是谁",              "帮我订机票", "明天天气如何", "帮我写封邮件", "什么是黑洞", "帮我算一下 12*15", "今天股市怎么样",
              "打开浏览器", "播放一首歌", "查一下快递", "把屏幕亮度调高", "帮我连上 wifi", "重启一下路由器",
              "给摄像头调个焦距", "把音量调大点", "蓝牙打开一下"]
NOOP_HELD = ["给我讲个冷笑话", "圆周率前三位是多少", "明天适合穿什么", "帮我起个英文名字", "推荐一部科幻片",
             "把显示器亮度调暗", "帮我配对蓝牙耳机"]
CAPS = list(T_TRAIN.keys())


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


def slots_for(cap, rng, held):
    if cap.startswith("led"):
        c = rng.choice(list(OBJ)); kw = {"color": c, "obj": rng.choice(OBJ[c]), "n": 0}
        if cap == "led_blink":
            kw["n"] = rng.randint(8, 12) if held else rng.randint(1, 7)
        return kw
    if cap == "fan": return {"v": rng.randint(250, 255) if held else rng.randint(30, 240)}
    if cap == "gpio_get": return {"cn": rng.choice([0, 1]), "l": rng.randint(301, 380) if held else rng.randint(0, 300)}
    if cap == "gpio_set": return {"cn": rng.choice([0, 1]), "l": rng.randint(301, 380) if held else rng.randint(0, 300),
                                  "v": rng.choice([0, 1])}
    return {}


def gen(tpls, cap, rng, held):
    kw = slots_for(cap, rng, held)
    return [{"role": "user", "content": rng.choice(tpls).format(**kw)},
            {"role": "assistant", "content": make_cmd(cap, kw)}]


# ---------- eval 集先建（与 train 规模解耦） ----------
heldA, heldB, seen_eval = [], [], set()
for cap in CAPS:
    for _ in range(40):
        p = gen(T_HELD[cap], cap, Re, True)
        if p[0]["content"] not in seen_eval:
            seen_eval.add(p[0]["content"]); heldA.append(p)
for cap, tpls in T_HARD.items():
    for _ in range(30):
        p = gen(tpls, cap, Re, True)
        if p[0]["content"] not in seen_eval:
            seen_eval.add(p[0]["content"]); heldB.append(p)
for t in NOOP_HELD:
    heldA.append([{"role": "user", "content": t}, {"role": "assistant", "content": NOOP_OUT}])
    heldB.append([{"role": "user", "content": t}, {"role": "assistant", "content": NOOP_OUT}])
Re.shuffle(heldA); Re.shuffle(heldB)

# ---- 教师合成的新说法（我=强模型生成；全部“有线索”，刻意不造弱线索以免污染 heldB）----
# 只进训练，且与 heldA/heldB 的模板字符串必须零重叠（脚本内断言）
T_EXTRA = {
 "led_on": ["把{obj}给打开好吗", "{obj}开起来", "请开一下{obj}", "{obj}麻烦点亮", "把{obj}打开吧",
            "{obj}亮起来", "帮我点亮{obj}", "给{obj}上电"],
 "led_off": ["把{obj}关了吧", "请关闭{obj}", "{obj}关掉", "让{obj}熄掉", "{obj}不用亮了",
             "把{obj}关停", "关一下{obj}", "{obj}麻烦关掉"],
 "led_status": ["{obj}现在是亮的吗", "帮我看看{obj}状态", "{obj}开着没有", "{obj}亮没亮",
                "查下{obj}亮着没", "{obj}状态是啥", "看看{obj}有没有亮", "确认下{obj}状态"],
 "led_blink": ["{obj}闪{n}次吧", "让{obj}连闪{n}下", "{obj}快闪{n}次", "{obj}闪烁{n}回",
               "把{obj}设成闪{n}下", "{obj}给我连闪{n}次", "{obj}眨{n}下", "让{obj}闪{n}遍"],
 "temp": ["温度现在多少", "现在几度", "测下温度", "温度值给我", "当前温度", "现在温度多少度",
          "机器温度多少", "温度情况如何"],
 "cpu": ["看下 cpu 频率", "cpu 使用率", "处理器频率多少", "cpu 跑多少频率", "查 cpu 状态",
         "cpu 现在多少", "主频是多少", "cpu 频率多少"],
 "mem": ["内存使用情况", "看看内存", "内存还剩多少可用", "剩余内存", "内存现在多少", "查内存占用",
         "内存还有多少", "看下内存"],
 "disk": ["看看磁盘空间", "磁盘使用情况", "还剩多少磁盘", "查磁盘空间", "磁盘剩余", "硬盘空间还有多少",
          "看下磁盘占用", "存储情况如何"],
 "fan": ["风扇设置成{v}", "把风扇速度调到{v}", "风扇调到 {v}", "设置风扇转速 {v}", "风扇速度设为{v}",
         "把风扇转到{v}", "风扇转速调为{v}", "把风扇速度设成{v}"],
 "fan_status": ["风扇现在转多少", "看看风扇转速", "风扇速度多少", "风扇转数", "当前风扇转速",
                "风扇转得怎么样", "查下风扇速度", "风扇现在多少转"],
 "gpio_get": ["读下 gpio{cn} 的 line {l}", "gpio chip{cn} line{l} 电平", "看下 gpio{cn} {l} 脚",
              "读取 chip{cn} 第 {l} 脚", "gpio{cn} 的 {l} 脚是多少", "查 gpio line {l}",
              "gpio chip{cn} 第{l}脚电平", "读 {l} 号脚的电平"],
 "gpio_set": ["把 gpio{cn} 的 {l} 脚设成{v}", "gpio chip{cn} line{l} 置{v}", "让 chip{cn} {l} 脚输出{v}",
              "gpio{cn} {l} 脚写成{v}", "设置 gpio line {l} 为{v}", "gpio{cn} 第 {l} 脚拉成{v}",
              "把 {l} 号脚设为{v}", "gpio chip{cn} {l} 脚设为{v}"],
 "info": ["板子型号", "看下板子信息", "硬件信息", "设备信息", "这台是什么板子", "板子是什么",
          "查硬件信息", "board 信息"],
}
# 断言：新说法与留出集零重叠（否则 heldA/heldB 失去意义）
_held_strs = set()
for _cap, _ts in T_HELD.items():
    _held_strs.update(_ts)
for _cap, _ts in T_HARD.items():
    _held_strs.update(_ts)
_bad = [t for ts in T_EXTRA.values() for t in ts if t in _held_strs]
print("T_EXTRA 与留出模板重叠数 =", len(_bad), "(须0)", flush=True)
assert not _bad, _bad


def build_train(extra_cut, n_target, seed=7):
    """extra_cut: 每能力并入 T_EXTRA 前 N 条; n_target: 固定训练样本量（体积）
    —— 体积固定、只变“唯一说法数”，才是隔离“信息量 vs 重复量”的正确自变量"""
    rng = random.Random(seed)
    pool = {cap: list(T_TRAIN[cap]) + list(T_EXTRA[cap][:extra_cut]) for cap in CAPS}
    items = [(cap, t) for cap in CAPS for t in pool[cap]] + [("__noop__", t) for t in NOOP_TRAIN]
    tr = []
    for _ in range(n_target):
        cap, tpl = rng.choice(items)
        if cap == "__noop__":
            tr.append([{"role": "user", "content": tpl}, {"role": "assistant", "content": NOOP_OUT}])
        else:
            kw = slots_for(cap, rng, False)
            tr.append([{"role": "user", "content": tpl.format(**kw)},
                       {"role": "assistant", "content": make_cmd(cap, kw)}])
    rng.shuffle(tr)
    uniq = len({p[0]["content"] for p in tr})
    return tr, uniq

import torch
from transformers import AutoModelForCausalLM, AutoTokenizer, Trainer, TrainingArguments
from datasets import Dataset

CMD_RE = re.compile(r"^hw (led (blue|green|blue led|green led) (on|off|status|blink \d+)|temp|cpu|mem|disk|fan( \d+)?|gpio (get|set) \d+ \d+( \d+)?|info)$")


def cap_of(cmd):
    t = cmd.split()
    if len(t) < 2 or t[0] != "hw": return None
    if t[1] == "gpio" and len(t) > 2: return "gpio_" + t[2]
    if t[1] == "led": return "led"
    if t[1] == "fan": return "fan" if len(t) > 2 else "fan_status"
    return t[1]


def classify(pred, gold):
    """返回 (是否正确, 失败标签)"""
    p = re.sub(r"\s+", " ", pred.strip().split("\n")[0]).strip()
    g = re.sub(r"\s+", " ", gold.strip())
    if NOOP_OUT in g:
        if NOOP_OUT in p: return True, None
        return False, "unnecessary_call" if p.startswith("hw") else "refusal_miss"
    if p == g: return True, None
    if p.startswith("hw") and not CMD_RE.match(p): return False, "invalid_syntax"
    gc, pc = cap_of(g), cap_of(p)
    if gc and pc and gc == pc: return False, "wrong_slot"
    if pc: return False, "wrong_capability"
    return False, "prose_fallback"


def build_prompt(tok, user):
    return tok.apply_chat_template([{"role": "system", "content": HW_SYS}, {"role": "user", "content": user}],
                                   tokenize=False, add_generation_prompt=True, enable_thinking=False)


def encode(tok, msgs):
    m = [{"role": "system", "content": HW_SYS}] + msgs
    full = tok.apply_chat_template(m, tokenize=False, add_generation_prompt=False, enable_thinking=False)
    prm = tok.apply_chat_template(m[:-1], tokenize=False, add_generation_prompt=True, enable_thinking=False)
    ids = tok(full, truncation=True, max_length=192)["input_ids"]
    pids = tok(prm, truncation=True, max_length=192)["input_ids"]
    n = min(len(pids), len(ids)); lab = [-100] * n + ids[n:]
    if len(lab) != len(ids): lab = list(ids)
    return {"input_ids": ids, "labels": lab}


class Collator:
    def __init__(self, pad): self.pad = pad
    def __call__(self, fs):
        L = max(len(f["input_ids"]) for f in fs); ids, att, lab = [], [], []
        for f in fs:
            i, l = f["input_ids"], f["labels"]; pad = L - len(i)
            ids.append(i + [self.pad] * pad); att.append([1] * len(i) + [0] * pad); lab.append(l + [-100] * pad)
        return {"input_ids": torch.tensor(ids), "attention_mask": torch.tensor(att), "labels": torch.tensor(lab)}


def evaluate(model, tok, cases):
    """返回逐条 (bool, label) —— 便于配对检验与失败分桶"""
    model.eval(); out_res = []
    for msgs in cases:
        q, gold = msgs[0]["content"], msgs[1]["content"]
        ids = tok(build_prompt(tok, q), return_tensors="pt", add_special_tokens=False).to("cuda")
        with torch.no_grad():
            o = model.generate(**ids, max_new_tokens=32, do_sample=False, pad_token_id=tok.eos_token_id)
        pred = tok.decode(o[0][ids["input_ids"].shape[1]:], skip_special_tokens=True)
        out_res.append(classify(pred, gold))
    return out_res


def mcnemar(a, b):
    """a=0.6B, b=1.7B 的逐条布尔; 返回 (b 独对, a 独对, 双侧精确 p)"""
    b_only = sum(1 for x, y in zip(a, b) if (not x) and y)   # a 错 b 对 -> 1.7B 独对
    a_only = sum(1 for x, y in zip(a, b) if x and (not y))   # a 对 b 错 -> 0.6B 独对
    n = b_only + a_only
    if n == 0: return b_only, a_only, 1.0
    k = min(b_only, a_only)
    p = 2 * sum(math.comb(n, i) for i in range(0, k + 1)) * (0.5 ** n)
    return b_only, a_only, min(1.0, p)


def report(tag, res, label="A"):
    ok = sum(1 for r in res if r[0]); n = len(res)
    print(f"  {tag}: {ok}/{n} = {ok/n:.1%}", flush=True)
    from collections import Counter
    bad = Counter(r[1] for r in res if not r[0])
    if bad: print(f"    失败分桶: {dict(bad)}", flush=True)
    return [r[0] for r in res]


def ensure_download(model_id):
    """把模型预下载到本地目录（镜像优先 + 重试），避免训练中途下载被中断（v7 就是这么崩的）"""
    local = "/workspace/_base/" + model_id.split("/")[-1]
    if os.path.exists(os.path.join(local, "config.json")) and any(
            f.startswith("model") and f.endswith(".safetensors") for f in os.listdir(local)):
        print(f"  [dl] cached: {local}", flush=True)
        return local
    os.environ["HF_HUB_DOWNLOAD_TIMEOUT"] = "120"
    os.environ["HF_HUB_ETAG_TIMEOUT"] = "60"
    from huggingface_hub import snapshot_download
    pats = ["*.json", "*.safetensors", "*.txt", "*.model", "*.jinja"]
    for ep in ("https://hf-mirror.com", "https://huggingface.co"):
        os.environ["HF_ENDPOINT"] = ep
        for att in range(3):
            try:
                print(f"  [dl] {model_id} via {ep} try{att+1}", flush=True)
                snapshot_download(model_id, local_dir=local, allow_patterns=pats)
                print(f"  [dl] ok -> {local}", flush=True)
                return local
            except Exception as e:
                print(f"  [dl] fail {type(e).__name__}: {str(e)[:150]}", flush=True)
    raise SystemExit(f"pre-download failed for {model_id}")


def train_arm(model_id, out, data):
    path = ensure_download(model_id)
    tok = AutoTokenizer.from_pretrained(path); tok.pad_token = tok.eos_token
    rows = [encode(tok, m) for m in data]
    model = AutoModelForCausalLM.from_pretrained(path, torch_dtype=torch.bfloat16,
                                                attn_implementation="sdpa").cuda()
    # 两臂统一 adafactor：1.7B 全参 SFT 用 AdamW 在 24G 上状态量 ~13.6GB 会爆；换 adafactor 两臂同等对待，不引入额外变量
    args = TrainingArguments(output_dir=out, num_train_epochs=4, per_device_train_batch_size=16,
                             learning_rate=2e-5, logging_steps=200, save_strategy="no", bf16=True,
                             optim="adafactor", report_to=[], lr_scheduler_type="cosine",
                             warmup_ratio=0.03, disable_tqdm=True)
    Trainer(model=model, args=args, train_dataset=Dataset.from_list(rows),
            data_collator=Collator(tok.pad_token_id)).train()
    os.makedirs(out, exist_ok=True); model.save_pretrained(out); tok.save_pretrained(out)
    return model, tok


print(f"heldA={len(heldA)} heldB={len(heldB)}", flush=True)

# 2x2: 信息量(低/高) x 体积(1x/2x)。体积固定 -> 隔离"重复量"；extra_cut -> 隔离"信息量"
ARMS = [
    ("A1_infoLO_vol1x", 0, 1300),
    ("A2_infoHI_vol1x", 8, 1300),
    ("A3_infoLO_vol2x", 0, 2600),
    ("A4_infoHI_vol2x", 8, 2600),
]
res = {}
for tag, cut, n in ARMS:
    data, uniq = build_train(cut, n)
    seen_now = {q[0]["content"] for q in data}
    leak = sum(1 for p in heldA + heldB if p[0]["content"] in seen_now)
    print(f"\n=== {tag}: extra_cut={cut} N={len(data)} uniq={uniq} leak={leak}(须0) ===", flush=True)
    assert leak == 0
    m, tk = train_arm(SMALL, f"/workspace/rt_{tag}", data)
    ra = report(f"{tag} heldA", evaluate(m, tk, heldA))
    rb = report(f"{tag} heldB", evaluate(m, tk, heldB))
    res[tag] = (ra, rb, uniq, len(data))
    del m
    torch.cuda.empty_cache()

print("\n=== 2x2 配对检验 (McNemar 精确) ===", flush=True)
for label, hi, lo in (("信息量效应(同体积1x): HI vs LO", "A2_infoHI_vol1x", "A1_infoLO_vol1x"),
                      ("体积效应(同信息LO): 2x vs 1x", "A3_infoLO_vol2x", "A1_infoLO_vol1x"),
                      ("双高 vs 双低: A4 vs A1", "A4_infoHI_vol2x", "A1_infoLO_vol1x")):
    for setname, idx in (("heldA", 0), ("heldB", 1)):
        a, b = res[lo][idx], res[hi][idx]
        hi_only = sum(1 for x, y in zip(a, b) if (not x) and y)
        lo_only = sum(1 for x, y in zip(a, b) if x and (not y))
        n = hi_only + lo_only
        p = 1.0 if n == 0 else min(1.0, 2 * sum(math.comb(n, i) for i in range(0, min(hi_only, lo_only) + 1)) * (0.5 ** n))
        print(f"  {setname} | {label}: 高者独对={hi_only} 低者独对={lo_only} p={p:.4f} "
              f"{'显著' if p < 0.05 else '不显著'}", flush=True)

print("\n=== 汇总 (uniq = 训练集里唯一说法数) ===", flush=True)
for tag, (ra, rb, uniq, n) in res.items():
    print(f"  {tag:18s} N={n:5d} uniq={uniq:4d}  heldA={sum(ra)}/{len(ra)}  heldB={sum(rb)}/{len(rb)}", flush=True)
print("V7C_DONE", flush=True)
