# -*- coding: utf-8 -*-
"""start_vllm_remote.py — 远端写脚本+启动 vLLM"""
script = """#!/bin/bash
cd /workspace
export PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True
python3 -m vllm.entrypoints.openai.api_server --model /workspace/qwen38_awq --port 8081 --max-model-len 2048 --gpu-memory-utilization 0.95 > /workspace/vllm2.log 2>&1
"""
with open("/workspace/start_vllm2.sh", "w") as f:
    f.write(script)
import os
os.chmod("/workspace/start_vllm2.sh", 0o755)

import subprocess
p = subprocess.Popen(
    ["bash", "/workspace/start_vllm2.sh"],
    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    start_new_session=True)
print("VLLM_PID:", p.pid)
