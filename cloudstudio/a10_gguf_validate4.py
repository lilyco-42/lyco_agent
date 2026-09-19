# 补充验证: ① router 直接 -sys/-p 便捷用法 ② grpo 工具调用 (需预渲染 tools)
import os, subprocess, json

HOME = os.path.expanduser("~")
CLI = os.path.join(HOME, "llama.cpp", "build", "bin", "llama-cli")
PY = "/root/lyco_agent/.venv/bin/python"
ROUTER = f"{HOME}/router_v4-Q4_K_M.gguf"
GRPO = f"{HOME}/grpo-Q4_K_M.gguf"

HW_SYS = ("你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。"
          "只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。")


def call(args, timeout=120):
    try:
        r = subprocess.run([CLI] + args, capture_output=True, text=True,
                           timeout=timeout, stdin=subprocess.DEVNULL)
        return (r.stdout or ""), (r.stderr or "")
    except subprocess.TimeoutExpired:
        return "", "TIMEOUT"


print("=== ① router: -sys + -p + -st (README 便捷写法) ===", flush=True)
ok = 0
cases = [("cpu 温度多少", "hw temp"), ("gpio line 12 什么电平", "hw gpio get")]
for q, exp in cases:
    out, err = call(["-m", ROUTER, "-sys", HW_SYS, "-p", q, "-n", "48", "--temp", "0", "-ngl", "0", "-st"])
    hit = exp in out
    ok += hit
    print(f"  [{'PASS' if hit else 'FAIL'}] {q!r} -> {exp!r}", flush=True)
    if not hit:
        print("       out尾部:", repr(out[-200:]), " err:", err[-200:], flush=True)
print(f"router 便捷写法: {ok}/{len(cases)}", flush=True)

print("\n=== ② grpo: 工具调用 (预渲染 tools) ===", flush=True)
TOOLS = [
    {"type": "function", "function": {"name": "lyv_knowledge", "description": "查询视频知识库",
     "parameters": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}}},
    {"type": "function", "function": {"name": "vnn_identify", "description": "识别图片内容",
     "parameters": {"type": "object", "properties": {"image": {"type": "string"}}, "required": ["image"]}}},
]
GRPO_SYS = "You are lyco, a helpful assistant. You can call tools."
jobs = [{"text": "怎么新建 rust 项目", "sys": GRPO_SYS, "tools": TOOLS},
        {"text": "你好呀", "sys": GRPO_SYS, "tools": TOOLS}]
open("/tmp/_jobs2.json", "w", encoding="utf-8").write(json.dumps(jobs, ensure_ascii=False))
code = r'''
import json, warnings
warnings.filterwarnings("ignore")
from transformers import AutoTokenizer
jobs = json.load(open("/tmp/_jobs2.json", encoding="utf-8"))
tok = AutoTokenizer.from_pretrained("Qwen/Qwen3-0.6B")
for i, j in enumerate(jobs):
    msgs = [{"role":"system","content":j["sys"]},{"role":"user","content":j["text"]}]
    open(f"/tmp/_g{i}.txt","w",encoding="utf-8").write(
        tok.apply_chat_template(msgs, tools=j["tools"], tokenize=False,
                                add_generation_prompt=True, enable_thinking=False))
print("OK")
'''
open("/tmp/_r2.py", "w", encoding="utf-8").write(code)
r = subprocess.run([PY, "/tmp/_r2.py"], capture_output=True, text=True, timeout=600)
print((r.stdout or "").strip(), (r.stderr or "")[-300:], flush=True)

expects = [("怎么新建 rust 项目", "lyv_knowledge"), ("你好呀", None)]
gok = 0
for i, (q, exp) in enumerate(expects):
    out, err = call(["-m", GRPO, "-f", f"/tmp/_g{i}.txt", "-n", "96", "--temp", "0", "-ngl", "0", "-st"])
    has_call = ("tool_call" in out) or ('"name"' in out)
    hit = (exp in out) if exp else (not has_call)
    gok += hit
    print(f"  [{'PASS' if hit else 'FAIL'}] {q!r} -> {exp or '不调工具'}", flush=True)
    if not hit:
        print("       out尾部:", repr(out[-260:]), flush=True)
print(f"grpo: {gok}/{len(expects)}", flush=True)
print("VALIDATE4_DONE", flush=True)
