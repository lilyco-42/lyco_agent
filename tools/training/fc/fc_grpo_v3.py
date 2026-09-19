# -*- coding: utf-8 -*-
# FC GRPO V3 — 从 V2 续训, NO_HIT 二次分类定向修 (闲聊误调+lyv/llm 混淆)。BASE=grpo_v2/final, A10 运行。
"""qwen_grpo_v2.py — 工具集扩展 GRPO (V2): 7 工具 9 类 prompt

新增工具: html_gen / html_render_video / rembg_remove / llm_generate / video_info
prompt 类别: 知识查询/识图/闲聊 (V1) + 生成页面/页面转视频/抠图/通用生成/视频信息 (V2)
奖励同 V1: 调对工具+1, 参数关键词+0.5, 该闲聊不调+1, 乱调 0
从 V1 产物续训 (qwen3_lyco_grpo/final), 200 步, FC 8-case 验证
"""
import json, random, re
import vllm.distributed.weight_transfer.nccl_engine as _ne
if not hasattr(_ne, 'NCCLTrainerSendWeightsArgs'):
    _ne.NCCLTrainerSendWeightsArgs = type('NCCLTrainerSendWeightsArgs', (), {})


MODEL_ID = "Qwen/Qwen3-0.6B"
OUT_DIR = "/workspace/qwen3_lyco_grpo_v3"
BASE_MODEL = "/workspace/qwen3_lyco_grpo_v2/final"
MAX_STEPS = 90

TOOLS = [
    {"type": "function", "function": {
        "name": "lyv_knowledge",
        "description": "查询【视频演示过的具体操作/命令步骤】: 用户问某软件/开发工具怎么用、某命令怎么敲、某操作在教程视频里怎么做。纯开放问答/创作/闲聊不属于此类。",
        "parameters": {"type": "object", "properties": {
            "query": {"type": "string", "description": "想学的操作"},
            "pack": {"type": "string", "description": "知识包路径"},
        }, "required": ["query"]}}},
    {"type": "function", "function": {
        "name": "vnn_identify",
        "description": "识别图片内容: 对截图/画面分类 (终端/GUI/自然/文档)",
        "parameters": {"type": "object", "properties": {
            "image": {"type": "string", "description": "图片路径"},
        }, "required": ["image"]}}},
    {"type": "function", "function": {
        "name": "html_gen",
        "description": "生成 HTML 页面文件: 给页面需求描述, 调用大模型生成完整单文件 HTML 并保存",
        "parameters": {"type": "object", "properties": {
            "prompt": {"type": "string", "description": "页面需求描述"},
            "output": {"type": "string", "description": "输出 html 文件路径"},
        }, "required": ["prompt", "output"]}}},
    {"type": "function", "function": {
        "name": "html_render_video",
        "description": "把 HTML 页面转成视频: headless 浏览器逐秒截图后合成 mp4",
        "parameters": {"type": "object", "properties": {
            "html": {"type": "string", "description": "输入 html 路径"},
            "output": {"type": "string", "description": "输出 mp4 路径"},
            "seconds": {"type": "integer", "description": "视频秒数"},
        }, "required": ["html", "output"]}}},
    {"type": "function", "function": {
        "name": "rembg_remove",
        "description": "抠图: 去除图片背景, 输出透明背景 png",
        "parameters": {"type": "object", "properties": {
            "image": {"type": "string", "description": "输入图片路径"},
            "output": {"type": "string", "description": "输出 png 路径"},
        }, "required": ["image", "output"]}}},
    {"type": "function", "function": {
        "name": "llm_generate",
        "description": "通用文本创作/开放生成: 写文案、故事、诗、邮件、翻译、润色、起标题等纯文字创作, 不涉及具体软件操作, 不落文件",
        "parameters": {"type": "object", "properties": {
            "prompt": {"type": "string", "description": "生成需求"},
        }, "required": ["prompt"]}}},
    {"type": "function", "function": {
        "name": "video_info",
        "description": "查看视频信息: 时长/分辨率/帧率",
        "parameters": {"type": "object", "properties": {
            "video": {"type": "string", "description": "视频路径"},
        }, "required": ["video"]}}},
]

HTML_Q = ["帮我做一个个人主页", "生成一个登录页面", "做一个产品介绍页",
          "写个 html 小游戏页面", "生成简历页面", "做一个倒计时页面",
          "帮我写个展示大屏页面", "生成一个计算器网页", "做个仪表盘页面",
          "生成活动宣传页"]
HTML_ARG = ["页面", "主页", "html", "网页"]
H2V_Q = ["把这个页面转成视频", "html 变成 mp4", "页面录成视频",
         "把刚才的网页做成视频", "html 生成视频", "把页面导出视频"]
H2V_ARG = ["html", "页面", "mp4", "视频"]
REMBG_Q = ["把这张图的背景去掉", "抠图这张照片", "人物抠出来", "去背景",
           "透明背景处理", "这张图抠图", "帮我抠出主体", "背景移除"]
REMBG_ARG = ["图", "抠", "背景", "png"]
LLM_Q = ["帮我写一段产品文案", "续写这个故事", "给我写首诗", "翻译这段话",
         "写个周报", "帮我起个标题", "润色这段文字", "写一封邮件"]
LLM_ARG = ["文案", "写", "故事", "诗"]
VINFO_Q = ["这个视频多长", "看看视频分辨率", "视频信息是什么", "这个 mp4 多大",
           "视频帧率多少", "查一下这个视频"]
VINFO_ARG = ["视频", "mp4", "多长", "分辨率"]
KNOW_Q = ["怎么新建 rust 项目", "怎么运行项目", "怎么编译项目", "如何安装依赖",
          "怎么提交代码", "怎么进入目录"]
IDENT_Q = ["帮我看看这张截图", "识别这个画面", "屏幕上是什么", "这个界面是什么应用"]
CHAT_Q = ["你好呀", "讲个笑话", "1 加 1 等于几", "你叫什么名字", "今天星期几",
          "再见", "推荐一首歌", "地球为什么是圆的",
          "今天天气不错", "怎么做红烧肉", "红烧肉要放多少糖", "猫喜欢吃什么",
          "怎么写好作文", "人生有什么意义", "给我推荐几本书", "谢谢你啦",
          "周末干什么好", "天空为什么是蓝色的", "帮我算 23 乘 4", "你叫什么名字呀"]


def gen_prompts(seed=42):
    rng = random.Random(seed)
    rows = []
    groups = [
        (KNOW_Q, "lyv_knowledge", ["rust", "项目", "run", "build", "clone"]),
        (IDENT_Q, "vnn_identify", ["截图", "图", "画面"]),
        (HTML_Q, "html_gen", HTML_ARG),
        (H2V_Q, "html_render_video", H2V_ARG),
        (REMBG_Q, "rembg_remove", REMBG_ARG),
        (LLM_Q, "llm_generate", LLM_ARG),
        (VINFO_Q, "video_info", VINFO_ARG),
    ]
    for qs, tool, args in groups:
        for q0 in qs:
            rows.append({"prompt": q0, "expected_tool": tool, "expected_arg": rng.choice(args)})
            pre = rng.choice(["请问 ", "", "", "帮我 ", "麻烦 "])
            rows.append({"prompt": (pre + q0 + rng.choice(["", " 谢谢", " 吧"])),
                         "expected_tool": tool, "expected_arg": rng.choice(args)})
    for q in CHAT_Q:
        rows.append({"prompt": q, "expected_tool": None, "expected_arg": None})
        rows.append({"prompt": q + " 呢", "expected_tool": None, "expected_arg": None})
    rng.shuffle(rows)
    return rows


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


def tool_reward(completions, expected_tool=None, expected_arg=None, **kw):
    rewards = []
    ets = expected_tool if isinstance(expected_tool, list) else [expected_tool] * len(completions)
    eas = expected_arg if isinstance(expected_arg, list) else [expected_arg] * len(completions)
    for comp, et, ea in zip(completions, ets, eas):
        call = parse_call(comp)
        if et is None:
            rewards.append(1.0 if call is None else 0.0)
        elif call and call.get("name") == et:
            args = json.dumps(call.get("arguments", {}), ensure_ascii=False)
            rewards.append(1.0 + (0.5 if (ea and ea in args) else 0.0))
        else:
            rewards.append(0.0)
    return rewards


def build_prompt(q, tok):
    return tok.apply_chat_template(
        [{"role": "system", "content": "You are lyco, a helpful assistant running on a Radxa SBC. You can call tools."},
         {"role": "user", "content": q}],
        tools=TOOLS, add_generation_prompt=True, enable_thinking=False, tokenize=False)


def train():
    from datasets import Dataset
    from transformers import AutoModelForCausalLM, AutoTokenizer
    from trl import GRPOConfig, GRPOTrainer
    import torch, os

    # V3/V4 是从前驱续训的定向修 (撤 chat 双倍权重等), 依赖前驱 final 存在;
    # 静默回退 base 会产出"看起来对但没续训"的错模型 —— fail-loud。
    if not os.path.exists(BASE_MODEL):
        raise SystemExit(f"缺前驱 {BASE_MODEL}: 需先跑上一阶段 GRPO (见 tools/fc_grpo_v*.py 链)")
    src = BASE_MODEL
    print(f"base: {src}", flush=True)
    tok = AutoTokenizer.from_pretrained(src)
    tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(
        src, torch_dtype=torch.bfloat16, attn_implementation="sdpa").cuda()

    rows = gen_prompts()
    for r in rows:
        r["prompt"] = build_prompt(r["prompt"], tok)
    ds = Dataset.from_list(rows)
    print(f"dataset: {len(ds)} prompts", flush=True)

    cfg = GRPOConfig(
        output_dir=OUT_DIR,
        per_device_train_batch_size=8,
        num_generations=4,
        max_steps=MAX_STEPS,
        learning_rate=5e-6,
        logging_steps=25,
        save_strategy="no",
        bf16=True,
        report_to=[], use_vllm=False,
    )
    trainer = GRPOTrainer(model=model, reward_funcs=tool_reward, args=cfg, train_dataset=ds)
    trainer.train()
    trainer.save_model(f"{OUT_DIR}/final")
    tok.save_pretrained(f"{OUT_DIR}/final")

    import torch as T
    model.eval()
    cases = [
        ("帮我做一个登录页面", "html_gen"),
        ("把页面转成视频", "html_render_video"),
        ("这张图抠图", "rembg_remove"),
        ("帮我写一段文案", "llm_generate"),
        ("这个视频多长", "video_info"),
        ("怎么新建 rust 项目", "lyv_knowledge"),
        ("识别这个画面", "vnn_identify"),
        ("今天天气不错", None),
    ]
    hit = 0
    for q, exp in cases:
        text = build_prompt(q, tok)
        ids = tok(text, return_tensors="pt").to("cuda")
        with T.no_grad():
            out = model.generate(**ids, max_new_tokens=200, do_sample=False,
                                 pad_token_id=tok.eos_token_id)
        gen = tok.decode(out[0][ids["input_ids"].shape[1]:], skip_special_tokens=True)
        call = parse_call(gen)
        got = call.get("name") if call else None
        ok = (got == exp)
        hit += ok
        print(f"{'HIT' if ok else 'MISS'} {q} -> {got} (want {exp})", flush=True)
    print(f"FC_V2_RESULT: {hit}/{len(cases)}", flush=True)
    print("GRPO_V2_DONE", flush=True)


if __name__ == "__main__":
    train()
