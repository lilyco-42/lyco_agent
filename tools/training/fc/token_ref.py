#!/usr/bin/env python3
"""token_ref.py — 生成 Python lyv.tokens() 的参考输出 (Rust 对齐测试的金标准)"""
import json
import sys

sys.path.insert(0, str(__import__("pathlib").Path(__file__).parent))

ASCII_TOK = __import__("re").compile(r"[A-Za-z][A-Za-z0-9_\-]*")

def tokens(text):
    t = [w.lower() for w in ASCII_TOK.findall(text)]
    zh = __import__("re").sub(r"[^\u4e00-\u9fff]", " ", text)
    t += [zh[i:i+2] for i in range(len(zh) - 1) if zh[i:i+2].strip()]
    return t

CASES = [
    "首先 cargo new hello_world 建立项目",
    "cargo run",
    "进入项目目录",
    "hello world 123",
    "",
    "!!! ###",
    "最后, Cargo Run 运行程序输出 Hello World。",
    "PS D:\\demo> cargo new demo",
    "然后 cd hello_world 进入项目目录",
]

if __name__ == "__main__":
    print(json.dumps({c: tokens(c) for c in CASES}, ensure_ascii=False, indent=1))
