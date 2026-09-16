#!/usr/bin/env python3
"""cli_datagen —— 意图→CLI 路由器的训练数据（泛化优先）

数据来自**真实写好的 hw CLI**（tools/hw），不是虚构工具。
泛化手段（对齐"考虑通用性"）:
  1. 多表达: 每个能力 ≥6 种说法 + 口语噪声
  2. 槽位泛化: led 颜色/次数/PWM 值/gpio chip-line 随机
  3. held-out: 特定表达与值域**只进 eval**（测未见组合）
  4. 该不调: 与硬件无关的意图 → 不产生命令
输出: train.jsonl / eval.jsonl（messages 格式，直接可 SFT）
"""
import argparse
import json
import random

R = random.Random(42)

# ---------- 表达模板 ({} 为槽位) ----------
T = {
    "led_on": [
        "帮我打开{obj}", "把{obj}打开", "{obj}开一下", "开{obj}",
        "turn on the {obj}", "让{obj}亮起来", "{obj}点亮",
        "把{obj}调成开的", "开一下{obj}呗", "麻烦开{obj}",
    ],
    "led_off": [
        "帮我关掉{obj}", "把{obj}关了", "{obj}关一下", "关{obj}",
        "turn off the {obj}", "{obj}熄灭", "别让{obj}亮着",
    ],
    "led_status": [
        "{obj}现在什么状态", "看看{obj}开没开", "{obj}亮着吗",
        "is the {obj} on", "查一下{obj}状态",
    ],
    "led_blink": [
        "让{obj}闪{n}下", "{obj}闪烁{n}次", "blink the {obj} {n} times",
        "{obj}闪一闪, {n}次",
    ],
    "temp": ["多少度", "cpu 温度", "现在热不热", "看下温度", "temperature",
             "板子温度多少", "热吗现在"],
    "cpu": ["cpu 频率", "现在多少主频", "负载怎么样", "cpu 状态", "cpu freq"],
    "mem": ["内存还剩多少", "memory", "内存占用", "还有多少内存"],
    "disk": ["磁盘还剩多少", "disk space", "存储满了没", "看下磁盘"],
    "fan": ["风扇调到{v}", "风扇转速{v}", "把风扇设为{v}", "fan speed {v}"],
    "fan_status": ["风扇转多快", "看看风扇", "fan rpm", "风扇状态",
                   "风扇现在什么速度", "风扇转没有", "fan speed status",
                   "fan rpm now", "查风扇转速", "fan?"],
    "gpio_get": ["读一下 gpio{cn} 的 {l} 号脚", "gpio line {l} 什么电平",
                 "读 gpio chip{cn} line{l}"],
    "gpio_set": ["把 gpio{cn} 的 {l} 号脚拉{v}", "gpio line {l} 设为 {v}",
                 "gpio{cn} line{l} 输出 {v}"],
    "info": ["板子什么型号", "board info", "系统信息", "什么板子"],
}

# 槽位 → 自然语言对象名
OBJ = {"blue": ["蓝灯", "蓝色指示灯", "blue led"],
       "green": ["绿灯", "电源灯", "green led"]}
HEAT = {"blue": "蓝灯", "green": "绿灯"}

def fmt(t, **kw):
    s = t.format(**kw)
    return s

def make_cmd(cap, kw):
    # 注意: 不能用 dict 字面量 (所有 f-string 会立即求值, 缺槽位即 KeyError)
    if cap == "led_on":
        return f"hw led {kw['color']} on"
    if cap == "led_off":
        return f"hw led {kw['color']} off"
    if cap == "led_status":
        return f"hw led {kw['color']} status"
    if cap == "led_blink":
        return f"hw led {kw['color']} blink {kw['n']}"
    if cap == "temp":
        return "hw temp"
    if cap == "cpu":
        return "hw cpu"
    if cap == "mem":
        return "hw mem"
    if cap == "disk":
        return "hw disk"
    if cap == "fan":
        return f"hw fan {kw['v']}"
    if cap == "fan_status":
        return "hw fan"
    if cap == "gpio_get":
        return f"hw gpio get {kw.get('cn', kw['c'])} {kw['l']}"
    if cap == "gpio_set":
        return f"hw gpio set {kw.get('cn', kw['c'])} {kw['l']} {kw['v']}"
    if cap == "info":
        return "hw info"
    raise ValueError(cap)

# held-out 划分: 这些表达/值域只进 eval (测泛化)
HELD_OUT_PHRASES = {"turn on the {obj}", "blink the {obj} {n} times",
                    "temperature", "fan speed {v}", "board info"}
HELD_OUT_VALUES = {"fan": (250, 255), "blink": (8, 12)}

def sample(cap, slots, heldout=False):
    kw = dict(slots)
    if "c" in kw:
        kw["cn"] = int(str(kw["c"]).replace("gpiochip", ""))
    t = R.choice(T[cap])
    if heldout:
        # 尽量抽 held-out 表达
        cands = [x for x in T[cap] if x in HELD_OUT_PHRASES]
        if cands:
            t = R.choice(cands)
    kw = dict(slots)
    if "c" in kw:
        kw["cn"] = int(str(kw["c"]).replace("gpiochip", ""))
    txt = fmt(t, **kw)
    return {"messages": [{"role": "user", "content": txt},
                         {"role": "assistant", "content": make_cmd(cap, kw)}]}

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--train", type=int, default=2400)
    ap.add_argument("--eval", type=int, default=300)
    ap.add_argument("-o", "--out", default=".")
    a = ap.parse_args()

    train, ev = [], []
    caps = ["led_on", "led_off", "led_status", "led_blink",
            "temp", "cpu", "mem", "disk", "fan", "fan_status",
            "gpio_get", "gpio_set", "info"]

    for _ in range(a.train):
        cap = R.choice(caps)
        if cap.startswith("led"):
            color = R.choice(list(OBJ))
            n = R.randint(1, 15) if cap == "led_blink" else 0
            s = sample(cap, {"color": color, "obj": R.choice(OBJ[color]), "n": n})
        elif cap == "fan":
            v = R.randint(30, 240)  # 训练不含 250-255 (held-out 值域)
            s = sample(cap, {"v": v})
        elif cap == "led_blink":
            s = sample(cap, {"n": R.randint(1, 7)})
        elif cap == "gpio_get":
            s = sample(cap, {"c": R.choice([0, "gpiochip0"]), "l": R.randint(0, 380)})
        elif cap == "gpio_set":
            s = sample(cap, {"c": R.choice([0, "gpiochip0"]),
                             "l": R.randint(0, 380), "v": R.choice([0, 1])})
        else:
            s = sample(cap, {})
        train.append(s)

    # 该不调必须进**训练集** (Hammer/v3 教训: 只放 eval → 拒绝率 0%)
    noop = ["讲个笑话", "今天几号", "你喜欢什么音乐", "写一首诗",
            "what is the capital of france", "帮我算 1+1", "唱首歌",
            "写个爬虫", "推荐一部电影", "1 加 1 等于几",
            "你是谁", "讲个鬼故事", "帮我订机票", "明天天气如何", "hello"]
    for t in noop:
        for _ in range(max(a.train // 100, 1)):
            train.append({"messages": [{"role": "user", "content": t},
                                       {"role": "assistant",
                                        "content": "(无需调用硬件命令)"}]})

    # eval: held-out 表达 + held-out 值域 (250-255 / blink 8-12) + 该不调
    noop = ["讲个笑话", "今天几号", "你喜欢什么音乐", "写一首诗",
            "what is the capital of france", "帮我算 1+1"]
    held = max(a.eval - len(noop), 1)
    for _ in range(held):
        cap = R.choice(caps)
        if cap.startswith("led"):
            color = R.choice(list(OBJ))
            n = R.randint(*HELD_OUT_VALUES["blink"]) if cap == "led_blink" else 0
            ev.append(sample(cap, {"color": color, "obj": R.choice(OBJ[color]),
                                   "n": n}, heldout=True))
        elif cap == "fan":
            ev.append(sample(cap, {"v": R.randint(*HELD_OUT_VALUES["fan"])},
                             heldout=True))
        elif cap == "gpio_get":
            ev.append(sample(cap, {"c": R.choice([0, "gpiochip0"]),
                                   "l": R.randint(0, 380)}, heldout=True))
        elif cap == "gpio_set":
            ev.append(sample(cap, {"c": R.choice([0, "gpiochip0"]),
                                   "l": R.randint(0, 380),
                                   "v": R.choice([0, 1])}, heldout=True))
        else:
            ev.append(sample(cap, {}, heldout=True))
    for t in noop:
        ev.append({"messages": [{"role": "user", "content": t},
                                {"role": "assistant",
                                 "content": "(无需调用硬件命令)"}]})

    R.shuffle(train)
    R.shuffle(ev)
    os.makedirs(a.out, exist_ok=True)
    for name, data in [("train.jsonl", train), ("eval.jsonl", ev)]:
        with open(os.path.join(a.out, name), "w", encoding="utf-8") as f:
            for d in data:
                f.write(json.dumps(d, ensure_ascii=False) + "\n")
    print(f"[cli_datagen] train={len(train)} eval={len(ev)} → {a.out}/")
    print("  泛化设计: 多表达+槽位随机; eval 含 held-out 表达/值域(fan 250-255, blink 8-12) + 该不调")


import os  # noqa: E402  (放底部避免顶部噪音)

if __name__ == "__main__":
    main()
