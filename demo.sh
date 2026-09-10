#!/bin/bash
# demo.sh — lyco_agent 端到端演示: 学习视频 → 检索 → OCR验证 → 图文输出
# 前置: cargo build --release -p lycore; smoke/ 内有 demo_tts4.mp4 + demo.srt
set -e
cd "$(dirname "$0")"
LY=lycore/target/release/lycore

echo "=========================================="
echo "lyco_agent 演示 — 能学、能验证、能执行"
echo "=========================================="

echo ""
echo "[1/4] 学习视频 (ffmpeg 抽帧 + tesseract OCR + 强/弱关联挖掘 + sqlite/FTS 索引)"
rm -rf smoke/demo_pack
$LY learn --video smoke/demo_tts4.mp4 --srt smoke/demo.srt --pack smoke/demo_pack

echo ""
echo "[2/4] 用户问: 怎么新建 rust 项目"
$LY ask --pack smoke/demo_pack "怎么新建 rust 项目" | head -10

echo ""
echo "[3/4] OCR grounding 验证 (re-OCR 关键帧, 置信度判定)"
cd smoke
python3 - <<'PYEOF'
import sys
sys.path.insert(0, '../tools')
from pathlib import Path
import lyv
ev = lyv.lookup('demo_pack', '怎么新建 rust 项目')
if ev:
    otxt, conf = lyv.ocr_frame(Path('demo_pack') / ev['frame'])
    ok = lyv.ocr_pass(otxt, conf, ev['strong'])
    print(f"  关键帧 OCR: {otxt[:50]}")
    print(f"  置信度: {conf:.2f} | 强关联词: {ev['strong']}")
    print(f"  VERIFY_{'PASS' if ok else 'FAIL'}")
PYEOF
cd ..

echo ""
echo "[4/4] 不懂的问题 → 诚实降级 + 学习队列"
$LY ask --pack smoke/demo_pack "怎么配置 nginx" | head -4
$LY harvest --pack smoke/demo_pack --out demo_training_tasks.jsonl

echo ""
echo "=========================================="
echo "闭环完成: learn → ask → verify → 学习队列 → 训练任务"
echo "=========================================="
