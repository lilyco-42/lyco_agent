#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""kws_plens_test.py —— 对照: processed_lens(scalar 输入) 取不同值, 看 joiner 输出是否变

假设: processed_lens=0 表示"无有效帧"→ encoder 输出退化; 取 29(=一帧块全部有效) 应恢复。
"""
import struct
import sys

import numpy as np

sys.path.insert(0, "/home/radxa")
from kws_run import D, audio, fbank, read_out, run  # noqa: E402

f = fbank(audio())
f.astype(np.float32).tofile(D + "/e0.dat")            # 29 x 80 行优先 (与 pack_hw_to_vip 一致)
for i in range(1, 38):
    np.zeros(1024, dtype=np.float32).tofile("%s/e%d.dat" % (D, i))
del f


def joiner_top(plens):
    # input 38 = 标量 (1 个 float32); 同时把 int32 形式也写进去试
    with open(D + "/e38.dat", "wb") as fh:
        fh.write(struct.pack("<f", float(plens)))

    run("s_enc39.txt")
    enc = read_out(0)
    np.array([0, 0], dtype=np.float32).tofile(D + "/d0.dat")
    run("s_dec.txt")
    dec = read_out(0)
    enc[:320].astype(np.float32).tofile(D + "/in0.dat")
    dec[:320].astype(np.float32).tofile(D + "/in1.dat")
    run("s_joiner2.txt")
    lg = read_out(0)[:263]
    p = np.exp(lg - lg.max())
    p /= p.sum()
    ent = float(-(p * np.log(p + 1e-10)).sum())
    o = np.argsort(-p)[:3]
    return p[o[0]] * 100, ent, [(int(i), round(p[i] * 100, 1)) for i in o], enc[:3]


for v in [0, 1, 29, 30]:
    if v == 0:
        np.zeros(1, dtype=np.float32).tofile(D + "/e38.dat")
    top1, ent, top3, enc3 = joiner_top(v)
    print("plens=%-3d top1=%5.1f%%  entropy=%.3f  top3=%s  enc[0:3]=%s" % (
        v, top1, ent, top3, np.round(enc3, 4).tolist()))
