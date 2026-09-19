#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""kws_probe.py —— 2x2 对照: encoder 输出取哪一段 x joiner 两输入的顺序

判据: 正确组合 → softmax 更尖 (top1 显著高于 25%、熵显著低于 ln4=1.386)
"""
import sys

import numpy as np

sys.path.insert(0, "/home/radxa")
from kws_run import D, audio, fbank, read_out, run  # noqa: E402

f = fbank(audio())
f.T.astype(np.float32).tofile(D + "/e0.dat")          # 80x29 布局
for i in range(1, 38):
    np.zeros(1024, dtype=np.float32).tofile("%s/e%d.dat" % (D, i))
np.zeros(1, dtype=np.float32).tofile(D + "/e38.dat")
run("s_enc39.txt")
enc = read_out(0)
print("encoder out0 长度 =", len(enc))

np.array([0, 0], dtype=np.float32).tofile(D + "/d0.dat")
run("s_dec.txt")
dec = read_out(0)


def prob(a):
    p = np.exp(a - a.max())
    return p / p.sum()


def try_combo(slice_name, enc_part, order):
    if order == "enc,dec":
        enc_part.astype(np.float32).tofile(D + "/in0.dat")
        dec[:320].astype(np.float32).tofile(D + "/in1.dat")
    else:
        dec[:320].astype(np.float32).tofile(D + "/in0.dat")
        enc_part.astype(np.float32).tofile(D + "/in1.dat")
    run("s_joiner2.txt")
    lg = read_out(0)
    if lg is None or len(lg) < 263:
        print("%-14s %-8s 输出异常 len=%s" % (slice_name, order, len(lg) if lg is not None else "None"))
        return 0.0
    p = prob(lg[:263])
    ent = float(-(p * np.log(p + 1e-10)).sum())
    o = np.argsort(-p)[:3]
    print("%-14s %-8s top1=%5.1f%%  entropy=%.3f  top3=%s" % (
        slice_name, order, p[o[0]] * 100, ent,
        ", ".join("%d:%.1f%%" % (i, p[i] * 100) for i in o)))
    return p[o[0]]


best = (0, "", "")
for sname, part in [("enc[:320]", enc[:320]),
                    ("enc[-320:]", enc[-320:]),
                    ("enc[640:960]", enc[640:960])]:
    for order in ["enc,dec", "dec,enc"]:
        v = try_combo(sname, part, order)
        if v > best[0]:
            best = (v, sname, order)
print("\n最佳组合: %s / %s (top1=%.1f%%)" % (best[1], best[2], best[0] * 100))
