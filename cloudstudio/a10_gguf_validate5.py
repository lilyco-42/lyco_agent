# 验证 v5: 只在 assistant 标记之后判定 (避免 prompt 回显污染); 并测 --chat-template-kwargs 关思考
import os, subprocess, json

HOME = os.path.expanduser("~")
CLI = os.path.join(HOME, "llama.cpp", "build", "bin", "llama-cli")
PY = "/root/lyco_agent/.venv/bin/python"
ROUTER = f"{HOME}/router_v4-Q4_K_M.gguf"
GRPO = f"{HOME}/grpo-Q4_K_M.gguf"
HW_SYS = ("你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。"
          "只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。")
GRPO_SYS = "You are lyco, a helpful assistant. You can call tools."
TOOLS = [
    {"type": "function", "function": {"name": "lyv_knowledge", "description": "查询视频知识库",
     "parameters": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}}},
    {"type": "function", "function": {"name": "vnn_identify", "description": "识别图片内容",
     "parameters": {"type": "object", "properties": {"image": {"type": "string"}}, "required": ["image"]}}},
]


def call(args, timeout=120):
    try:
        r = subprocess.run([CLI] + args, capture_output=True, text=True,
                           timeout=timeout, stdin=subprocess.DEVNULL)
        return (r.stdout or ""), (r.stderr or "")
    except subprocess.TimeoutExpired:
        return "", "TIMEOUT"


def generated(out):
    """只取最后一个 assistant 标记之后的内容 —— 排除 prompt 回显"""
    if "<|im_start|>assistant" in out:
        return out.rsplit("<|im_start|>assistant", 1)[-1]
    return out


def render_all(jobs, tag):
    open(f"/tmp/_jobs_{tag}.json", "w", encoding="utf-8").write(json.dumps(jobs, ensure_ascii=False))
    code = r'''
import json, sys, warnings
warnings.filterwarnings("ignore")
from transformers import AutoTokenizer
tag = sys.argv[1]
jobs = json.load(open(f"/tmp/_jobs_{tag}.json", encoding="utf-8"))
tok = AutoTokenizer.from_pretrained("Qwen/Qwen3-0.6B")
for i, j in enumerate(jobs):
    msgs = [{"role":"system","content":j["sys"]},{"role":"user","content":j["text"]}]
    open(f"/tmp/_{tag}{i}.txt","w",encoding="utf-8").write(
        tok.apply_chat_template(msgs, tools=j.get("tools"), tokenize=False,
                                add_generation_prompt=True, enable_thinking=False))
print("RENDER_OK", len(jobs))
'''
    open("/tmp/_r5.py", "w", encoding="utf-8").write(code)
    r = subprocess.run([PY, "/tmp/_r5.py", tag], capture_output=True, text=True, timeout=600)
    print((r.stdout or "").strip(), (r.stderr or "")[-200:], flush=True)


print("=== router_v4 (预渲染, thinking=off) ===", flush=True)
rc = [("现在多少主频", "hw cpu"), ("gpio line 12 什么电平", "hw gpio get 0 12"),
      ("讲个笑话", "无需调用硬件命令")]
render_all([{"text": q, "sys": HW_SYS} for q, _ in rc], "rt")
ok = 0
for i, (q, exp) in enumerate(rc):
    out, err = call(["-m", ROUTER, "-f", f"/tmp/_rt{i}.txt", "-n", "48", "--temp", "0", "-ngl", "0", "-st"])
    g = generated(out)
    hit = exp in g
    ok += hit
    print(f"  [{'PASS' if hit else 'FAIL'}] {q!r} -> {exp!r}   生成={g.strip()[:60]!r}", flush=True)
print(f"router_v4: {ok}/{len(rc)}", flush=True)

print("\n=== grpo (预渲染 tools, thinking=off) ===", flush=True)
gc = [("怎么新建 rust 项目", "lyv_knowledge"), ("你好呀", None)]
render_all([{"text": q, "sys": GRPO_SYS, "tools": TOOLS} for q, _ in gc], "gp")
gok = 0
for i, (q, exp) in enumerate(gc):
    out, err = call(["-m", GRPO, "-f", f"/tmp/_gp{i}.txt", "-n", "96", "--temp", "0", "-ngl", "0", "-st"])
    g = generated(out)
    has_call = "tool_call" in g
    hit = (exp in g) if exp else (not has_call)
    gok += hit
    print(f"  [{'PASS' if hit else 'FAIL'}] {q!r} -> {exp or '不调工具'}   生成={g.strip()[:80]!r}", flush=True)
print(f"grpo: {gok}/{len(gc)}", flush=True)

print("\n=== 便捷写法: --chat-template-kwargs 关思考 (router) ===", flush=True)
out, err = call(["-m", ROUTER, "-sys", HW_SYS, "-p", "gpio line 12 什么电平",
                 "-n", "48", "--temp", "0", "-ngl", "0", "-st",
                 "--chat-template-kwargs", '{"enable_thinking": false}'])
g = generated(out)
print("  生成:", repr(g.strip()[:120]), flush=True)
print("  含 hw gpio get:", "hw gpio get" in g, flush=True)
print("VALIDATE5_DONE", flush=True)
