#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""voice_server.py — Radxa 语音入口 (手机浏览器说话 → 板上 faster-whisper → 文本)

纯标准库 + faster-whisper。手机浏览器打开 http://<板子IP>:8000 即用。
架构对齐十年设计: 板上自治 (ASR 本地, 零云), 手机只是麦克风。
"""
import io
import json
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from faster_whisper import WhisperModel

MODEL = "/home/radxa/models/faster-whisper-tiny"
print("loading whisper...", flush=True)
model = WhisperModel(MODEL, device="cpu", compute_type="int8")
print("ready", flush=True)

PAGE = """<!doctype html><html><head><meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<title>lyco 语音入口</title>
<style>body{font-family:system-ui;background:#111;color:#eee;text-align:center;padding-top:10vh}
button{font-size:20px;padding:16px 36px;border-radius:40px;border:0;background:#2b6cff;color:#fff;margin:8px}
#file{color:#bbb;margin:10px}
#out{margin-top:20px;font-size:18px;white-space:pre-wrap;color:#9f9}</style></head><body>
<h2>lyco 语音入口</h2>
<button id=b>🎤 按住说话</button><br>
<input id=file type=file accept="audio/*" capture=microphone>
<div id=out>提示: 手机上若麦克风被禁, 用下面的"选择录音文件"</div>
<script>
let rec, chunks=[], btn=document.getElementById('b'), out=document.getElementById('out');
const fin = document.getElementById('file');

function speak(t){                      // TTS 反馈: 读出结果 (http 页面也可用)
  try{ const u=new SpeechSynthesisUtterance(t); u.lang='zh-CN'; u.rate=1.1;
       speechSynthesis.cancel(); speechSynthesis.speak(u); }catch(e){}
}
async function send(blob, tag){
  out.textContent = '识别中...';
  const fd = new FormData();
  fd.append('audio', blob, 'speech.' + (blob.type.includes('ogg')?'ogg':(blob.type.includes('mp4')?'mp4':'webm')));
  const r = await fetch('/asr', {method:'POST', body:fd});
  const j = await r.json();
  const t = j.text || '(空)';
  out.textContent = '[' + tag + ' ' + j.secs + 's] ' + t;
  speak('识别到: ' + t);
}
btn.onpointerdown = async e => {        // 按住说话 (需安全上下文: localhost 或白名单)
  try{
    const stream = await navigator.mediaDevices.getUserMedia({audio:true});
    rec = new MediaRecorder(stream); chunks = [];
    rec.ondataavailable = ev => chunks.push(ev.data);
    rec.start(); btn.textContent = '🔴 松开识别';
  }catch(err){ out.textContent = '麦克风被禁: 用下方"选择录音文件"'; }
};
btn.onpointerup = () => {
  if(!rec || rec.state !== 'recording'){ btn.textContent='🎤 按住说话'; return; }
  rec.onstop = () => send(new Blob(chunks), '按住说话');
  rec.stop(); btn.textContent = '🎤 按住说话';
};
fin.onchange = () => {                  // 文件入口: 走系统录音器, 不受 https 限制
  if(fin.files && fin.files[0]) send(fin.files[0], '文件');
};
</script></body></html>"""


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _send(self, code, body, ctype):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self._send(200, PAGE.encode("utf-8"), "text/html; charset=utf-8")

    def do_POST(self):
        if self.path != "/asr":
            self._send(404, b"{}", "application/json")
            return
        n = int(self.headers.get("Content-Length", 0))
        audio = self.rfile.read(n)          # webm/ogg 由浏览器 MediaRecorder 产出
        t0 = time.time()
        text = ""
        try:
            # faster-whisper 经 ffmpeg 直接吃 webm/ogg 容器, 无需转 wav
            segs, info = model.transcribe(io.BytesIO(audio), language="zh")
            text = "".join(s.text for s in segs).strip()
        except Exception as e:
            text = f"(解码失败: {e})"
        secs = round(time.time() - t0, 2)
        print(f"[asr] {secs}s -> {text!r}", flush=True)
        self._send(200, json.dumps({"text": text, "secs": secs},
                                   ensure_ascii=False).encode("utf-8"),
                   "application/json")


ThreadingHTTPServer(("0.0.0.0", 8000), H).serve_forever()
