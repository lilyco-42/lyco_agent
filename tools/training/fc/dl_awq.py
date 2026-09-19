# -*- coding: utf-8 -*-
"""dl_awq.py — 下载 Qwen3.8-27B AWQ-INT4 量化版"""
from huggingface_hub import snapshot_download

snapshot_download(
    "cyankiwi/Qwen3.8-27B-AWQ-INT4",
    local_dir="/workspace/qwen38_awq",
)
print("AWQ_DL_COMPLETE")
