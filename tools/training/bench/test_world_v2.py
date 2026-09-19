# -*- coding: utf-8 -*-
"""test_world_v2.py — 测试扩容后重训的世界知识模型 (远端执行)"""
import json
import subprocess

CONFIG = """model_path = "model/world_base.json"
mode = "chat"
train_backend = "candle"
steps = 0
infer_backend = "gpu"
question = "天空是什么颜色"
port = 8080
"""

QUESTIONS = ["天空是什么颜色", "鱼会游泳吗", "太阳从哪边落下", "怎么新建rust项目"]

# 写测试配置
open("/workspace/lyco_chat/world_final_test.toml", "w").write(CONFIG)

env_prefix = "export LD_LIBRARY_PATH=/usr/local/cuda/lib64:$LD_LIBRARY_PATH"
for q in QUESTIONS:
    cfg = CONFIG.replace('question = "天空是什么颜色"', f'question = "{q}"')
    open("/workspace/lyco_chat/world_final_test.toml", "w").write(cfg)
    r = subprocess.run(
        ["bash", "-c",
         f"cd /workspace/lyco_chat && {env_prefix} && timeout 60 ./target/release/demo_chat --config world_final_test.toml 2>&1 | tail -3"],
        capture_output=True, text=True, timeout=70)
    print(f"Q: {q}")
    print(r.stdout[-260:])
    print()
