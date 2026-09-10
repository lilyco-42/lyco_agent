#!/bin/bash
# all_in_one_test.sh — 单会话: 起 Qwen3.8-27B + 等加载 + 跑 FC 测试 (规避 kernel 收割)
pkill -f llama-server 2>/dev/null
sleep 1
cd /workspace
setsid ./llama-b10883/llama-server -m qwen38_27b_q4.gguf --port 8081 --jinja -c 8192 -t 28 > llama38.log 2>&1 &
for i in $(seq 1 60); do
  if curl -s -m 2 http://127.0.0.1:8081/health >/dev/null 2>&1; then
    echo "SERVER_READY after ${i} polls"
    break
  fi
  sleep 2
done
python3 test_qwen38_fc.py 2>&1 | tail -6
