# -*- coding: utf-8 -*-
"""dl_8b.py — 下载 Qwen3-8B AWQ (A10 单卡可跑的底座)"""
from huggingface_hub import snapshot_download
import sys

repo = sys.argv[1] if len(sys.argv) > 1 else "Qwen/Qwen3-8B-AWQ"
dest = "/workspace/qwen3_8b_awq"

snapshot_download(repo, local_dir=dest)
print("DL_COMPLETE")
