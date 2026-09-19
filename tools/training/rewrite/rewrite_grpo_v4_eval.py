# -*- coding: utf-8 -*-
"""rewrite_grpo_v4_eval.py — v4 模型环境奖励评测（eval-only，T4 fp16 适配）

v4 训练已完成（models/qwen3_lyco_rewrite_v4.tar 2026-09-10 09:05 下载），
本脚本只跑 evaluate()：25 组口语模式 → lyv.lookup 命中与意图正确率，
对照 v2 基线 7/10 与 v3 README 记录 64%。
"""
import sys

sys.path.insert(0, "/workspace")
import lyv  # noqa: E402

OUT_DIR = "/workspace/qwen3_lyco_rewrite_v4"
PACK = "/workspace/lyv_tmp/pack_merged"

ORAL_PATTERNS = [
    ("我想写个Rust程序第一步干啥", "新建 rust 项目", "rust.project.create"),
    ("Rust项目从零开始怎么搞", "新建 rust 项目", "rust.project.create"),
    ("项目文件建在哪了", "新建 rust 项目", "rust.project.create"),
    ("开个新工程练练手", "cargo new 工程", "rust.project.create"),
    ("初始化一个代码项目", "cargo init 项目", "rust.project.create"),
    ("程序怎么让他动起来", "运行 项目", "rust.project.run"),
    ("写好的代码怎么跑", "运行 项目", "rust.project.run"),
    ("代码跑不起来怎么回事", "运行 项目", "rust.project.run"),
    ("跑个hello world看看", "cargo run hello world", "rust.project.run"),
    ("执行程序用什么命令", "cargo run 程序", "rust.project.run"),
    ("怎么进入那个文件夹", "cd 项目目录", "fs.chdir"),
    ("切到项目目录里", "cd 项目目录", "fs.chdir"),
    ("到代码目录下去", "cd 目录", "fs.chdir"),
    ("跳转到工程文件夹", "cd 工程目录", "fs.chdir"),
    ("依赖包怎么下载", "install 依赖", "py.pkg.install"),
    ("第三方库装一下", "install 第三方库", "py.pkg.install"),
    ("npm的包咋安装", "npm install 包", "node.pkg.install"),
    ("缺个模块装哪个命令", "npm install 模块", "node.pkg.install"),
    ("代码提交到仓库", "git commit 提交", "git.push"),
    ("改动推到远程", "git push 推送", "git.push"),
    ("保存我的修改", "git commit 修改", "git.push"),
    ("上传代码到github", "git push github", "git.push"),
    ("项目怎么构建出来", "build 项目", "rust.project.build"),
    ("编译一下代码", "cargo build 代码", "rust.project.build"),
    ("打个发布版本", "cargo build release", "rust.project.build"),
]


def build_prompt(q, tok):
    return tok.apply_chat_template(
        [{"role": "system", "content": "你是检索查询改写器。把口语化问题改写成知识库检索关键词(操作+对象), 只输出关键词, 10字以内。"},
         {"role": "user", "content": q}],
        add_generation_prompt=True, enable_thinking=False, tokenize=False)


def evaluate():
    from transformers import AutoModelForCausalLM, AutoTokenizer
    import torch
    tok = AutoTokenizer.from_pretrained(OUT_DIR)
    # T4 无 bf16 计算支持，加载时转 fp16（transformers 5.x 参数名 dtype）
    try:
        model = AutoModelForCausalLM.from_pretrained(
            OUT_DIR, dtype=torch.float16, device_map="cuda")
    except TypeError:
        model = AutoModelForCausalLM.from_pretrained(
            OUT_DIR, torch_dtype=torch.float16, device_map="cuda")
    by_intent = {}
    hits = 0
    total = 0
    for oral, standard, intent in ORAL_PATTERNS:
        total += 1
        prompt = build_prompt(oral, tok)
        inputs = tok(prompt, return_tensors="pt", add_special_tokens=False).to("cuda")
        out = model.generate(**inputs, max_new_tokens=32, do_sample=False)
        text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=False).strip()
        result = lyv.lookup(PACK, text)
        ok = result is not None and result["intent"] == intent
        hits += ok
        by_intent.setdefault(intent, [0, 0])
        by_intent[intent][1] += 1
        by_intent[intent][0] += ok
        print(f"[{'OK ' if ok else 'FAIL'}] {oral[:18]} -> {text[:26]!r} -> {result['intent'] if result else 'NO_HIT'}", flush=True)
    for intent, (h, t) in sorted(by_intent.items()):
        print(f"  {intent}: {h}/{t}")
    print(f"环境奖励评测: {hits}/{total} = {hits/total:.0%} (v2 基线 7/10, v3 README 64%)", flush=True)


if __name__ == "__main__":
    evaluate()
