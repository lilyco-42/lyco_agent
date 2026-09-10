# -*- coding: utf-8 -*-
"""merge_packs.py — 合并多个知识包 (飞轮第二圈: pack_final + pack_devops)

生产形态: 用户的知识包是持续增长的, 多次 learn 的产物需要合并。
合并要点: id 是"行号语义" (u000 = segments.jsonl 第 1 行), 必须重编号。
"""
import json
import shutil
import sqlite3
import sys
from pathlib import Path

SRC = ["smoke/pack_final", "smoke/pack_devops"]
OUT = "smoke/pack_merged"


def merge():
    out = Path(OUT)
    if out.exists():
        shutil.rmtree(out)
    for d in ["frames", "ocr", "knowledge", "index"]:
        (out / d).mkdir(parents=True)

    all_units = []
    frame_map = {}  # 旧帧文件 → 新名
    for src in SRC:
        src = Path(src)
        lines = (src / "knowledge" / "segments.jsonl").read_text(encoding="utf-8").splitlines()
        for line in lines:
            u = json.loads(line)
            all_units.append(u)
    # 重编号 + 复制帧
    for i, u in enumerate(all_units):
        old_frame = u["frame"]
        new_id = f"m{i:03d}"
        new_frame = f"frames/{new_id}_f0.webp"
        src_frame = Path(SRC[0]) / old_frame if (Path(SRC[0]) / old_frame).exists() \
            else Path(SRC[1]) / old_frame
        if src_frame.exists():
            shutil.copy(src_frame, out / new_frame)
        u["id"] = new_id
        u["frame"] = new_frame
        u.setdefault("prereq", [])
        u.setdefault("command", None)
        u.setdefault("frame_t", (u["t0"] + u["t1"]) / 2)
        u.setdefault("ocr", "")
        u.setdefault("ocr_conf", 0.0)
        u.setdefault("strong", [])
        u.setdefault("weak", [])
        all_units[i] = u

    jsonl = "\n".join(json.dumps(u, ensure_ascii=False) for u in all_units)
    (out / "knowledge" / "segments.jsonl").write_text(jsonl, encoding="utf-8")

    db = sqlite3.connect(out / "index" / "knowledge.sqlite")
    db.executescript(
        """
        CREATE TABLE segments(id TEXT PRIMARY KEY, t0 REAL, t1 REAL, text TEXT,
         intent TEXT, command TEXT, frame TEXT, ocr TEXT, ocr_conf REAL,
         strong TEXT, weak TEXT);
        CREATE VIRTUAL TABLE seg_fts USING fts5(id, text, entities, intent, strong);
        """
    )
    sys.path.insert(0, str(Path(__file__).parent))
    from lyv import tokens
    for u in all_units:
        strong = " ".join(u["strong"])
        db.execute(
            "INSERT INTO segments VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            (u["id"], u["t0"], u["t1"], u["text"], u["intent"], u["command"],
             u["frame"], u["ocr"], u["ocr_conf"], strong, " ".join(u["weak"])))
        db.execute(
            "INSERT INTO seg_fts VALUES(?,?,?,?,?)",
            (u["id"], " ".join(tokens(u["text"])),
             " ".join(tokens(" ".join(u["command"].split()) if u["command"] else "")),
             u["intent"], " ".join(tokens(strong))))
    db.commit()
    db.close()
    print(f"merged: {len(all_units)} units from {len(SRC)} packs")
    intents = sorted({u['intent'] for u in all_units})
    print("intents:", intents)


if __name__ == "__main__":
    merge()
