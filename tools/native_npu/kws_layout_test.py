#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""kws_layout_test.py —— 对照实验: 只变 fbank 的数据布局, 看哪个输出更"有主见"

判据: 正确的布局 → softmax 分布更尖 (top1 更高 / 熵更低)。
"""
import sys

import numpy as np

sys.path.insert(0, "/home/radxa")
from kws_run import D, audio, fbank, read_out, run  # noqa: E402


def score(layout_name, arr):
    arr.astype(np.float32).tofile(D + "/e0.dat")
    for i in range(1, 38):
        np.zeros(1024, dtype=np.float32).tofile("%s/e%d.dat" % (D, i))
    np.zeros(1, dtype=np.float32).tofile(D + "/e38.dat")
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
    order = np.argsort(-p)[:3]
    top = ", ".join("%d:%.1f%%" % (i, p[i] * 100) for i in order)
    print("%-22s top1=%.1f%%  entropy=%.3f  top3=[%s]" % (layout_name, p[order[0]] * 100, ent, top))
    return p[order[0]]


f = fbank(audio())          # (29, 80) = T x F
a = score("T x F (当前)", f)
b = score("F x T (转置)", f.T)
print("\n更尖的那个 = 正确布局:", "T x F" if a > b else "F x T")
