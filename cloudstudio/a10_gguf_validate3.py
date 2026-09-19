# GGUF 端到端验证 v3: 先 smoke test 定 flag, 再批量; stdin=DEVNULL + 短超时防挂死
import os, subprocess, json

HOME = os.path.expanduser("~")
CLI = os.path.join(HOME, "llama.cpp", "build", "bin", "llama-cli")
PY = "/root/lyco_agent/.venv/bin/python"
ROUTER = f"{HOME}/router_v4-Q4_K_M.gguf"

HW_SYS = ("你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。"
          "只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。")

print("=== help: 相关 flag ===", flush=True)
r = subprocess.run(f"{CLI} --help 2>&1 | grep -iE 'single|conversation|-st|--prompt|--file|jinja|chat-template|no-cnv'",
                   shell=True, capture_output=True, text=True)
print(r.stdout[:2000], flush=True)


def render_all(jobs):
    spec = []
    for j in jobs:
        spec.append(j)
    open("/tmp/_jobs.json", "w", encoding="utf-8").write(json.dumps(spec, ensure_ascii=False))
    code = r'''
import json, warnings
warnings.filterwarnings("ignore")
from transformers import AutoTokenizer
jobs = json.load(open("/tmp/_jobs.json", encoding="utf-8"))
tok = AutoTokenizer.from_pretrained("Qwen/Qwen3-0.6B")
for i, j in enumerate(jobs):
    msgs = [{"role":"system","content":j["sys"]},{"role":"user","content":j["text"]}]
    tools = j.get("tools")
    out = tok.apply_chat_template(msgs, tools=tools, tokenize=False,
                                  add_generation_prompt=True, enable_thinking=False)
    open(f"/tmp/_p{i}.txt","w",encoding="utf-8").write(out)
print("RENDER_ALL_OK", len(jobs))
'''
    open("/tmp/_render_all.py", "w", encoding="utf-8").write(code)
    r = subprocess.run([PY, "/tmp/_render_all.py"], capture_output=True, text=True, timeout=600)
    print((r.stdout or "").strip(), (r.stderr or "")[-400:], flush=True)
    return r.returncode == 0


def call(args, timeout=90, show=False):
    try:
        r = subprocess.run([CLI] + args, capture_output=True, text=True,
                           timeout=timeout, stdin=subprocess.DEVNULL)
    except subprocess.TimeoutExpired:
        return None, "TIMEOUT"
    if show:
        print(f"  rc={r.returncode}\n  STDERR:{(r.stderr or '')[-400:]}\n  STDOUT:{(r.stdout or '')[-300:]}", flush=True)
    return (r.stdout or ""), (r.stderr or "")


# ---- smoke test: 找出能出字的参数组合
print("\n=== smoke test ===", flush=True)
render_all([{"text": "现在多少主频", "sys": HW_SYS}])
SMOKE = [
    ["-m", ROUTER, "-f", "/tmp/_p0.txt", "-n", "24", "--temp", "0", "-ngl", "0", "-st"],
    ["-m", ROUTER, "-f", "/tmp/_p0.txt", "-n", "24", "--temp", "0", "-ngl", "0"],
    ["-m", ROUTER, "-p", open("/tmp/_p0.txt", encoding="utf-8").read(), "-n", "24", "--temp", "0", "-ngl", "0", "-st"],
]
working = None
for i, a in enumerate(SMOKE):
    out, err = call(a, show=True)
    print(f"  -- argset {i}: out_len={len(out) if out is not None else 'TIMEOUT'}", flush=True)
    if out and out.strip():
        working = a
        print(f"  => using argset {i}", flush=True)
        break
if not working:
    print("SMOKE_FAILED: 没有任何参数组合产出文本, 见上面 STDERR", flush=True)
    print("GGUF_VALIDATE_ABORT", flush=True)
    raise SystemExit(0)


def gen(idx):
    a = []
    for x in working:
        a.append(f"/tmp/_p{idx}.txt" if x == "/tmp/_p0.txt" else x)
    out, err = call(a, timeout=120)
    return out or ""


ROUTER_CASES = [("现在多少主频", "hw cpu"), ("板子烫不烫", "hw temp"),
                ("读一下 gpio0 的 97 号脚", "hw gpio get 0 97"), ("把风扇调到 200", "hw fan 200"),
                ("板子什么型号", "hw info"), ("讲个笑话", "无需调用硬件命令")]
print("\n=== router_v4 端到端 ===", flush=True)
render_all([{"text": q, "sys": HW_SYS} for q, _ in ROUTER_CASES])
ok = 0
for i, (q, exp) in enumerate(ROUTER_CASES):
    out = gen(i)
    hit = exp in out
    ok += hit
    print(f"  [{'PASS' if hit else 'FAIL'}] {q!r} -> {exp!r}", flush=True)
    if not hit:
        print("       out尾部:", repr(out[-200:]), flush=True)
print(f"router_v4: {ok}/{len(ROUTER_CASES)}", flush=True)
print("GGUF_VALIDATE_DONE", flush=True)
