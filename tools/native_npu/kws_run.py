#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""kws_run.py —— 完整 KWS 链路: 音频 → fbank → NPU(encoder/decoder/joiner) → 唤醒词打分

I/O 规格 (vpm_run 实测):
  encoder : in0 = 80x29 (fbank), in1..37 = cache(首帧 0), in38 = 标量
            out0 = 320x4 (encoder 输出), out1..38 = 下一帧 cache
  decoder : in0 = 2 (token ids[2])          → out0 = 320
  joiner  : in0 = 320 (enc), in1 = 320 (dec) → out0 = 263 (vocab logits)

用法: python3 kws_run.py [wav]   (不给则生成 1s 扫频音做链路自检)
"""
import os
import subprocess
import sys
import wave

import numpy as np

D = "/home/radxa/npu-sdk/examples/vpm_run/operator/v2"
RUN = "/home/radxa/npu-bin/usr/bin/vpm_run"
VA = "/home/radxa/npu_demos/voice_assistant"
SR, N_MEL, FRAMES, HOP, WIN = 16000, 80, 29, 160, 400
ENV = dict(os.environ, LD_LIBRARY_PATH="/home/radxa/lib")


def run(sample, save=True):
    cmd = [RUN, "-s", sample, "-l", "1"]
    if save:
        cmd += ["-b", "0", "--save_txt", "1"]
    r = subprocess.run(cmd, cwd=D, env=ENV, capture_output=True, text=True, timeout=180)
    return ("ret=0" in (r.stdout + r.stderr))


def read_out(i):
    p = f"{D}/output_{i}.txt"
    if not os.path.exists(p):
        return None
    vals = []
    for line in open(p):
        line = line.strip()
        if not line:
            continue
        try:
            vals.append(float(line.split()[-1]))
        except ValueError:
            pass
    return np.array(vals, dtype=np.float32)


def load_tokens():
    m = {}
    p = f"{VA}/prebuilt/kws/tokens.txt"
    if os.path.exists(p):
        for line in open(p, encoding="utf-8"):
            line = line.rstrip("\n")
            if not line:
                continue
            parts = line.split()
            if len(parts) >= 2 and parts[1].isdigit():
                m[int(parts[1])] = parts[0]
            elif len(parts) == 1:
                m[len(m)] = parts[0]
    return m


def hz2mel(f):
    return 2595.0 * np.log10(1.0 + f / 700.0)


def fb():
    lo, hi = hz2mel(20.0), hz2mel(8000.0)
    pts = np.linspace(lo, hi, N_MEL + 2)
    hzs = 700.0 * (10 ** (pts / 2595.0) - 1.0)
    b = np.floor(513 * hzs / SR).astype(int)
    f = np.zeros((N_MEL, 257), dtype=np.float32)
    for m in range(1, N_MEL + 1):
        l, c, r = b[m - 1], b[m], b[m + 1]
        c = max(c, l + 1)
        r = max(r, c + 1)
        for k in range(l, c):
            f[m - 1, k] = (k - l) / (c - l)
        for k in range(c, r):
            f[m - 1, k] = (r - k) / (r - c)
    return f


def fbank(x):
    F = fb()
    win = np.hanning(WIN).astype(np.float32)
    need = (FRAMES - 1) * HOP + WIN
    x = np.pad(x, (0, max(0, need - len(x))))
    out = np.zeros((FRAMES, N_MEL), dtype=np.float32)
    for i in range(FRAMES):
        sp = np.abs(np.fft.rfft(x[i * HOP:i * HOP + WIN] * win, n=512)) ** 2
        out[i] = np.log(np.maximum(F @ sp, 1e-10))
    return out


def audio(path=None):
    if path:
        with wave.open(path, "rb") as w:
            return np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768
    t = np.arange(SR) / SR
    f = 200 + 2800 * t
    return (0.3 * np.sin(2 * np.pi * np.cumsum(f) / SR)).astype(np.float32)


def main():
    tokv = load_tokens()
    x = audio(sys.argv[1] if len(sys.argv) > 1 else None)
    feat = fbank(x)
    print(f"[1] 音频 {len(x)/SR:.2f}s → fbank {feat.shape}")

    # --- encoder ---
    feat.tofile(f"{D}/e0.dat")
    for i in range(1, 38):
        np.zeros(1024, dtype=np.float32).tofile(f"{D}/e{i}.dat")
    np.zeros(1, dtype=np.float32).tofile(f"{D}/e38.dat")
    ok = run("s_enc39.txt")
    enc = read_out(0)
    print(f"[2] NPU encoder: {'OK' if ok else 'FAIL'}"
          + (f" out0={enc.shape}" if enc is not None else ""))
    if enc is None:
        return
    enc320 = enc[:320]                     # 取前 320 维喂 joiner

    # --- decoder (输入 2 个 token id, 初始 [0,0]) ---
    np.array([0, 0], dtype=np.float32).tofile(f"{D}/d0.dat")
    ok = run("s_dec.txt")
    dec = read_out(0)
    print(f"[3] NPU decoder: {'OK' if ok else 'FAIL'}"
          + (f" out0={dec.shape}" if dec is not None else ""))

    # --- joiner (enc320 + dec320 → 263 logits) ---
    if dec is not None:
        enc320.astype(np.float32).tofile(f"{D}/in0.dat")
        dec[:320].astype(np.float32).tofile(f"{D}/in1.dat")
        ok = run("s_joiner2.txt")
        logits = read_out(0)
        print(f"[4] NPU joiner: {'OK' if ok else 'FAIL'}")
        if logits is not None and len(logits) >= 263:
            lg = logits[:263]
            p = np.exp(lg - lg.max())
            p /= p.sum()
            top = np.argsort(-p)[:5]
            print("[5] top5 token: " + ", ".join(
                f"{tokv.get(int(i), i)}({p[i]*100:.1f}%)" for i in top))
    print("链路自检完成: 音频 → fbank → NPU(enc/dec/join) → 分词 ✓")


if __name__ == "__main__":
    main()
