#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""lyco compute agent —— 把 Radxa A7A 的能力挂到 lain42.top/compute 算力平台

平台侧: POST /api/v1/device/services 注册服务; GET /api/v1/device/tasks 领任务;
        任务 payload 原样转发到 http://127.0.0.1:<port><path>, 结果回传。
鉴权:   X-Device-Token 头。

本机能力由 voice_server 的 /node/* 端点提供:
  /node/asr   语音识别 (faster-whisper, 纯 CPU 本地)
  /node/chat  命令路由 (本地 0.6B 模型: 自然语言 -> hw 命令)
  /node/kws   NPU 语音唤醒 (VIP9000, 实测 RTF 0.18)
  /node/ve2   VE2 视频硬编 (H.264 1080p60)
  /node/hw    硬件控制 (LED/风扇/温度/GPIO, 命令白名单)
"""
import json
import time
import urllib.error
import urllib.request

PLATFORM = "https://lain42.top/compute"
DEVICE_TOKEN = "ae634dc58f42532291fa80f4197b2ba7a4abc9fb211ed26c"

# (服务名, 类型, 本地端口, 路径, 积分/次, 描述)
SERVICES = [
    ("硬件控制 (A7A)", "custom", 8000, "/node/hw",  1,  "LED/风扇/温度/GPIO, 命令白名单"),
    ("本地语音识别",   "custom", 8000, "/node/asr", 2,  "faster-whisper 板上本地识别 (零云)"),
    ("命令路由 (LLM)", "llm",    8000, "/node/chat", 3, "本地 0.6B 路由器: 自然语言→hw 命令"),
    ("NPU 语音唤醒",   "custom", 8000, "/node/kws", 5,  "全志A733 VIP9000 NPU Zipformer (RTF 0.18)"),
    ("VE2 视频硬编",   "ffmpeg", 8000, "/node/ve2", 10, "A733 VE2 硬件 H.264 编码 1080p@60fps"),
]
POLL_INTERVAL = 3


def api(method, path, body=None, raw=None, ctype="application/json", token=None, timeout=120):
    url = PLATFORM + path
    data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
    headers = {"User-Agent": "lyco-compute-agent/1.0"}
    if token:
        headers["X-Device-Token"] = token
    if data is not None:
        headers["Content-Type"] = ctype
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.read()


def ensure_services():
    cur = json.loads(api("GET", "/api/v1/device/services", token=DEVICE_TOKEN) or b"{}")
    existing = {(s["port"], s["path"], s["type"]) for s in cur.get("items", [])}
    for name, stype, port, path, price, desc in SERVICES:
        if (port, path, stype) in existing:
            continue
        api("POST", "/api/v1/device/services",
            body={"name": name, "type": stype, "port": port, "path": path,
                  "price": price, "desc": desc}, token=DEVICE_TOKEN)
        print("registered service:", name, port, path, flush=True)


def run_task(t):
    url = "http://127.0.0.1:%d%s" % (t["port"], t["path"])
    try:
        payload = api("GET", "/api/v1/device/tasks/%s/payload" % t["id"],
                      token=DEVICE_TOKEN, timeout=60)
        req = urllib.request.Request(url, data=payload or b"",
                                     headers={"Content-Type": "application/octet-stream",
                                              "User-Agent": "lyco-compute-agent/1.0"},
                                     method="POST")
        with urllib.request.urlopen(req, timeout=900) as r:
            out = r.read()
        api("POST", "/api/v1/device/tasks/%s/result" % t["id"], raw=out,
            ctype="application/octet-stream", token=DEVICE_TOKEN)
        print("task ok:", t["id"], t.get("serviceName"), len(out), "bytes", flush=True)
    except Exception as e:
        try:
            api("POST", "/api/v1/device/tasks/%s/result" % t["id"],
                body={"status": "error", "error": str(e)[:500]}, token=DEVICE_TOKEN)
        except Exception:
            pass
        print("task fail:", t["id"], str(e)[:120], flush=True)


def main():
    print("lyco compute agent starting ->", PLATFORM, flush=True)
    while True:
        try:
            ensure_services()
            break
        except Exception as e:
            print("register retry:", str(e)[:120], flush=True)
            time.sleep(5)
    while True:
        try:
            data = api("GET", "/api/v1/device/tasks", token=DEVICE_TOKEN, timeout=30)
            for t in json.loads(data or b"{}").get("items", []):
                run_task(t)
        except Exception as e:
            print("poll error:", str(e)[:120], flush=True)
        time.sleep(POLL_INTERVAL)


if __name__ == "__main__":
    main()
