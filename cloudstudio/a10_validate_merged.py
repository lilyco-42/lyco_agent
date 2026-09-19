# 抽验合并模型 GGUF（预渲染 thinking=off; -st; 只在 assistant 标记之后判定）
import os, subprocess, json

HOME = os.path.expanduser("~")
CLI = os.path.join(HOME, "llama.cpp", "build", "bin", "llama-cli")
PY = "/root/lyco_agent/.venv/bin/python"
G = f"{HOME}/router_merged-Q4_K_M.gguf"
HW_SYS = ("你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。"
          "只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。")

CASES = [("现在多少主频", "hw cpu"), ("板子烫不烫", "hw temp"),
         ("读一下 gpio0 的 97 号脚", "hw gpio get 0 97"), ("gpio line 12 什么电平", "hw gpio get 0 12"),
         ("把风扇调到 200", "hw fan 200"), ("板子什么型号", "hw info"),
         ("讲个笑话", "无需调用硬件命令"), ("把屏幕亮度调高", "无需调用硬件命令")]

open("/tmp/_mj.json", "w", encoding="utf-8").write(json.dumps(
    [{"sym": HW_SYS, "t": q} for q, _ in CASES], ensure_ascii=False))
code = r'''
import json, warnings
warnings.filterwarnings("ignore")
from transformers import AutoTokenizer
jobs = json.load(open("/tmp/_mj.json", encoding="utf-8"))
tok = AutoTokenizer.from_pretrained("Qwen/Qwen3-0.6B")
for i, j in enumerate(jobs):
    msgs = [{"role":"system","content":j["sym"]},{"role":"user","content":j["t"]}]
    open(f"/tmp/_m{i}.txt","w",encoding="utf-8").write(
        tok.apply_chat_template(msgs, tokenize=False, add_generation_prompt=True, enable_thinking=False))
print("OK")
'''
open("/tmp/_mr.py", "w", encoding="utf-8").write(code)
r = subprocess.run([PY, "/tmp/_mr.py"], capture_output=True, text=True, timeout=600)
print((r.stdout or "").strip(), (r.stderr or "")[-200:], flush=True)

ok = 0
for i, (q, exp) in enumerate(CASES):
    try:
        rr = subprocess.run([CLI, "-m", G, "-f", f"/tmp/_m{i}.txt", "-n", "48",
                             "--temp", "0", "-ngl", "0", "-st"],
                            capture_output=True, text=True, timeout=180, stdin=subprocess.DEVNULL)
        out = rr.stdout or ""
    except subprocess.TimeoutExpired:
        out = ""
    g = out.rsplit("<|im_start|>assistant", 1)[-1] if "<|im_start|>assistant" in out else out
    hit = exp in g
    ok += hit
    print(f"[{'PASS' if hit else 'FAIL'}] {q!r} -> {exp!r}", flush=True)
    if not hit:
        print("   got:", repr(g.strip()[:120]), flush=True)
print(f"merged GGUF: {ok}/{len(CASES)}", flush=True)
print("MERGED_VALIDATE_DONE", flush=True)
