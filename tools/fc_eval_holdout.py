# -*- coding: utf-8 -*-
"""fc_eval_holdout.py — 100 例 held-out 评测（与训练集零重叠，评测/训练解耦）

为什么单独一个脚本：v1 的 "88%" 是评测污染(train≈test)的产物。
本脚本的 100 例**全部是训练集(TOOL_TRAIN/NEG_TRAIN/EVAL_* of fc_train_v3)里没有的新问法**，
分三档指标：工具选择准确率 / abstention 准确率 / 总体，并给按工具细分。

用法: python fc_eval_holdout.py <model_path|BASE>
"""
import json
import os
import re
import sys

MODEL_ID = "Qwen/Qwen3-0.6B"
TOOLS_JSON = "/workspace/tools_openai.json"
SYSTEM = "You are lyco, a helpful assistant. You can call tools."

def load_tools():
    if os.path.exists(TOOLS_JSON):
        return json.load(open(TOOLS_JSON, encoding="utf-8"))
    raise SystemExit("缺 tools_openai.json: 先跑 `lycore tools --out /workspace/tools_openai.json`")

# ---- 50 条工具请求（新问法；7 工具） ----
EVAL_TOOL = [
    ("怎么用 git 提交代码", "lyv_knowledge"),
    ("docker 怎么装", "lyv_knowledge"),
    ("怎么配置环境变量", "lyv_knowledge"),
    ("pip 安装包的命令是什么", "lyv_knowledge"),
    ("vim 怎么保存退出", "lyv_knowledge"),
    ("怎么解压 tar 包", "lyv_knowledge"),
    ("ssh 免密登录怎么配", "lyv_knowledge"),
    ("怎么查端口占用", "lyv_knowledge"),
    ("这是什么界面", "vnn_identify"),
    ("看下这张图里是什么", "vnn_identify"),
    ("帮我分析这张截图", "vnn_identify"),
    ("这个画面属于什么类型", "vnn_identify"),
    ("识别图片内容", "vnn_identify"),
    ("这图是终端还是网页", "vnn_identify"),
    ("看一下这个截屏", "vnn_identify"),
    ("把这个人物抠出来", "rembg_remove"),
    ("去掉背景只要主体", "rembg_remove"),
    ("图片去背景", "rembg_remove"),
    ("抠出这个 logo", "rembg_remove"),
    ("把这张照片的背景抹掉", "rembg_remove"),
    ("给我一张透明底的图", "rembg_remove"),
    ("去背处理一下", "rembg_remove"),
    ("帮我做个个人主页", "html_gen"),
    ("生成一个表单页面", "html_gen"),
    ("写一个简单的网页", "html_gen"),
    ("做个活动页", "html_gen"),
    ("生成 HTML 文件", "html_gen"),
    ("做一个导航页", "html_gen"),
    ("写个静态页面", "html_gen"),
    ("写一段自我介绍", "llm_generate"),
    ("帮我写封道歉信", "llm_generate"),
    ("写几句宣传语", "llm_generate"),
    ("把这段话改写一下", "llm_generate"),
    ("写个简短的产品说明", "llm_generate"),
    ("来一段朋友圈文案", "llm_generate"),
    ("帮我起个名字", "llm_generate"),
    ("把页面导出成视频", "html_render_video"),
    ("网页录制为 mp4", "html_render_video"),
    ("把这个 html 变成视频", "html_render_video"),
    ("页面转视频", "html_render_video"),
    ("把网页截成视频", "html_render_video"),
    ("html 生成 mp4", "html_render_video"),
    ("录制页面成视频", "html_render_video"),
    ("看下视频信息", "video_info"),
    ("这个视频多少帧", "video_info"),
    ("查视频时长", "video_info"),
    ("视频分辨率多少", "video_info"),
    ("mp4 的详细参数", "video_info"),
    ("看视频码率", "video_info"),
    ("这个视频的参数", "video_info"),
]

# ---- 50 条负样本（新问法；闲聊 + 我们 7 工具确实覆盖不到的请求） ----
EVAL_NEG = [
    "早", "在吗", "哈哈", "嗯嗯", "好的", "谢谢你了", "你是谁呀", "今天星期几",
    "我有点累", "无聊", "吃了吗", "晚安啦", "早上好", "中午吃什么", "你多大",
    "你会下棋吗", "给我讲个谜语", "推荐首歌", "今天开心吗", "你会写代码吗",
    "讲个历史故事", "帮我背单词", "随便说点什么", "拜拜", "你会画画吗",
    "帮我打开微信", "给我订个外卖", "帮我打车", "查下我的快递", "帮我转账",
    "给我发个短信", "帮我叫个闹钟", "给我查话费", "帮我连 WiFi", "给我看下股票",
    "帮我订酒店", "查一下航班", "帮我打印文件", "帮我关一下空调", "帮我预约体检",
    "给我查天气预警", "帮我关灯", "帮我考勤打卡", "帮我买张电影票", "给我推荐个餐厅",
    "帮我改一下我的密码", "给我加个好友", "帮我把手机静音", "给我放首歌", "帮我导航到公司",
]

def parse_call(text):
    m = re.search(r"<tool_call>\s*(\{.*?\})\s*</tool_call>", text, re.S)
    if m:
        try:
            return json.loads(m.group(1))
        except Exception:
            return None
    m = re.search(r'\{\s*"name"\s*:\s*"(\w+)"', text)
    if m:
        return {"name": m.group(1)}
    return None

def build_prompt(q, tok, tools):
    return tok.apply_chat_template(
        [{"role": "system", "content": SYSTEM}, {"role": "user", "content": q}],
        tools=tools, add_generation_prompt=True, enable_thinking=False, tokenize=False)

def main(src):
    from transformers import AutoModelForCausalLM, AutoTokenizer
    import torch

    tools = load_tools()
    print(f"eval src: {src}  | held-out = {len(EVAL_TOOL)} 工具 + {len(EVAL_NEG)} 负样本 = {len(EVAL_TOOL)+len(EVAL_NEG)} 例", flush=True)
    tok = AutoTokenizer.from_pretrained(src)
    model = AutoModelForCausalLM.from_pretrained(src, torch_dtype=torch.bfloat16, device_map="cuda")

    per_tool = {}
    tool_ok = 0
    for q, et in EVAL_TOOL:
        inputs = tok(build_prompt(q, tok, tools), return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=256, do_sample=False)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False)
        call = parse_call(text)
        got = call.get("name") if call else None
        ok = got == et
        tool_ok += ok
        d = per_tool.setdefault(et, [0, 0])
        d[1] += 1
        d[0] += ok
        print(f"[{'PASS' if ok else 'FAIL'}] {q} -> {got}", flush=True)

    neg_ok = 0
    for q in EVAL_NEG:
        inputs = tok(build_prompt(q, tok, tools), return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=256, do_sample=False)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False)
        call = parse_call(text)
        ok = call is None
        neg_ok += ok
        if not ok:
            print(f"[FAIL-neg] {q} -> {call.get('name')}", flush=True)

    n_tool, n_neg = len(EVAL_TOOL), len(EVAL_NEG)
    print("\n=== 结果 ===", flush=True)
    for t, (ok, tot) in sorted(per_tool.items()):
        print(f"  {t:20s} {ok}/{tot} = {ok/tot:.0%}", flush=True)
    print(f"工具选择准确率        : {tool_ok}/{n_tool} = {tool_ok/n_tool:.0%}", flush=True)
    print(f"abstention 准确率(不误调): {neg_ok}/{n_neg} = {neg_ok/n_neg:.0%}", flush=True)
    print(f"总体                : {tool_ok+neg_ok}/{n_tool+n_neg} = {(tool_ok+neg_ok)/(n_tool+n_neg):.0%}", flush=True)

if __name__ == "__main__":
    s = sys.argv[1] if len(sys.argv) > 1 else "BASE"
    main(MODEL_ID if s == "BASE" else s)
