# -*- coding: utf-8 -*-
"""lyco_agent_loop.py — Agent 多轮工具执行循环 (R2 长任务链路)

架构:
  用户问题 → Qwen3-0.6B-GRPO (tool_call 决策)
          → 工具执行器 (lyv_knowledge / vnn_identify 真实执行, 走 sqlite/OCR)
          → 工具结果 JSON 回填对话 → 模型生成最终回答
          → (模型仍想调工具则继续循环, 上限 4 轮)

这是「混合长任务调用」的最小闭环: 模型决策 + 真实工具执行 + 多轮回填。
用法: python lyco_agent_loop.py "怎么新建 rust 项目"
"""
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import lyv  # noqa: E402

TOOLS = [
    {"type": "function", "function": {
        "name": "lyv_knowledge",
        "description": "查询视频知识库: 问怎么做某操作, 返回带时间戳的视频切片+关键帧+OCR验证文字",
        "parameters": {"type": "object", "properties": {
            "query": {"type": "string", "description": "想学的操作"},
            "pack": {"type": "string", "description": "知识包路径"},
        }, "required": ["query"]}}},
    {"type": "function", "function": {
        "name": "vnn_identify",
        "description": "OCR失败时启用内部识图神经网络, 特征激活式对图片打分描述",
        "parameters": {"type": "object", "properties": {
            "image": {"type": "string", "description": "图片路径"},
        }, "required": ["image"]}}},
]

MODEL_DIR = "/workspace/qwen3_lyco_grpo"   # A10 上的 GRPO 模型
LOCAL_MODEL = None                          # 本地路径 (可选)

def exec_tool(name, arguments):
    """真实工具执行 — lyv sqlite 检索 / VNN 识图, 返回结构化结果"""
    if name == "lyv_knowledge":
        pack = arguments.get("pack") or "pack_final"
        # 模型可能幻觉出不存在的路径 → 回退默认知识包
        if not Path(pack).exists():
            pack = "pack_final"
        r = lyv.lookup(pack, arguments.get("query", ""))
        if r is None:
            return {"ok": False, "tool": name,
                    "error": "NO_HIT: 知识库没有这个操作, 已入学习队列"}
        return {"ok": True, "tool": name,
                "answer": r["text"], "command": r["command"],
                "clip": f"{r['t0']}s-{r['t1']}s", "keyframe": r["frame"],
                "strong": r["strong"], "grounding": f"ocr_conf={r['ocr_conf']}"}
    if name == "vnn_identify":
        try:
            from vnn_proto import identify
            r = identify(arguments.get("image", ""), ocr_text="", ocr_conf=0.0)
            if r.get("skipped"):
                return {"ok": True, "tool": name, "result": "OCR 已通过, 无需 VNN"}
            return {"ok": True, "tool": name, "verdict": r["verdict"],
                    "conf": r["conf"],
                    "learning_queue": r.get("learning_queue", [])}
        except ImportError:
            return {"ok": False, "tool": name, "error": "VNN 不可用 (缺 opencv), 入学习队列"}
    return {"ok": False, "tool": name, "error": f"未知工具 {name}"}

def parse_call(text):
    m = re.search(r"<tool_call>\s*(\{.*?\})\s*</tool_call>", text, re.S)
    if m:
        try:
            return json.loads(m.group(1))
        except Exception:
            return None
    m = re.search(r'\{\s*"name"\s*:\s*"(\w+)"[^}]*\}', text)
    if m:
        return {"name": m.group(1)}
    return None

def build_prompt(messages, tok):
    return tok.apply_chat_template(messages, tools=TOOLS,
                                   add_generation_prompt=True,
                                   enable_thinking=False, tokenize=False)

def run(question, max_rounds=4):
    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer
    model_dir = MODEL_DIR if Path(MODEL_DIR).exists() else LOCAL_MODEL
    tok = AutoTokenizer.from_pretrained(model_dir)
    model = AutoModelForCausalLM.from_pretrained(
        model_dir, torch_dtype=torch.bfloat16, device_map="cuda")

    messages = [{"role": "system", "content": "You are lyco. 用工具回答操作类问题."},
                {"role": "user", "content": question}]

    for round_i in range(1, max_rounds + 1):
        prompt = build_prompt(messages, tok)
        inputs = tok(prompt, return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=300, do_sample=False)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False)

        call = parse_call(text)
        if call is None:
            # 最终回答 (剔除工具残留标签与 JSON 复读)
            answer = re.sub(r"<\|im_end\|>.*", "", text, flags=re.S).strip()
            answer = re.sub(r"</?(tool_call|tool_response|error)>", "", answer).strip()
            if answer.startswith("{") and '"ok"' in answer:
                # 模型复读工具结果 JSON → 转成人话 (诚实说不会/汇报结果)
                try:
                    r = json.loads(answer)
                    answer = ("抱歉，我还没学会这个操作，已加入学习队列。"
                              if not r.get("ok") else
                              f"{r.get('tool')} 查询完成: {r.get('answer','')}")
                except Exception:
                    answer = "抱歉，我还没学会这个操作，已加入学习队列。"
            print(f"[round {round_i}] FINAL: {answer}")
            return answer
        # 执行工具
        print(f"[round {round_i}] TOOL_CALL: {call}")
        result = exec_tool(call.get("name", ""), call.get("arguments", {}))
        print(f"[round {round_i}] RESULT: {json.dumps(result, ensure_ascii=False)[:200]}")
        # 回填 (assistant 发起调用 + tool 结果)
        messages.append({"role": "assistant", "content":
                         f"<tool_call>\n{json.dumps(call, ensure_ascii=False)}\n</tool_call>"})
        messages.append({"role": "tool",
                         "content": json.dumps(result, ensure_ascii=False)})
    print("[max_rounds] 达到轮数上限, 强制收束")
    return None

if __name__ == "__main__":
    q = sys.argv[1] if len(sys.argv) > 1 else "怎么新建 rust 项目"
    run(q)
