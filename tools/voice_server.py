#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""voice_server.py — lyco 对话端 (DeepSeek 风格 UI + 语音)

全链在板上闭环, 零云:
  手机浏览器 → ASR(faster-whisper) → 路由器(llama-server, 本地 0.6B) → hw CLI → 灯
安全: 只执行 `hw ` 前缀命令 (白名单), 其余一律拒绝。
"""
import io
import json
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from faster_whisper import WhisperModel

ASR_MODEL = "/home/radxa/models/faster-whisper-tiny"
LLAMA = "http://127.0.0.1:8080/v1/chat/completions"
ROUTER_SYS = ("你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。"
              "只输出命令本身，不要解释；与硬件无关的请求输出 (无需调用硬件命令)。 /no_think")

print("loading whisper...", flush=True)
model = WhisperModel(ASR_MODEL, device="cpu", compute_type="int8")
print("ready", flush=True)


def route(text: str):
    """意图 → 命令 (本地 llama-server)。返回 (cmd, raw)。"""
    try:
        body = json.dumps({
            "messages": [{"role": "system", "content": ROUTER_SYS},
                         {"role": "user", "content": text + " /no_think"}],
            "temperature": 0, "max_tokens": 64,
        }).encode()
        req = urllib.request.Request(LLAMA, data=body,
                                     headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=120) as r:
            j = json.loads(r.read())
        raw = j["choices"][0]["message"]["content"].strip()
        # 取第一行非空内容作为命令
        cmd = next((l.strip() for l in raw.splitlines() if l.strip()), "")
        return cmd, raw
    except Exception as e:
        return "", f"(路由器不可用: {e})"


def exec_cmd(cmd: str):
    """只允许 hw 前缀 (白名单), 经 sudo 免密执行。返回 (ok, summary)。"""
    if not cmd.startswith("hw "):
        return False, "该命令超出当前允许范围 (仅支持 hw 硬件命令)"
    import subprocess
    parts = cmd.split()
    # 必须拼成 /usr/local/bin/hw —— 与 sudoers 规则完全一致, 否则 sudo 要密码 (实测踩坑)
    exe = "/usr/local/bin/" + parts[0]
    p = subprocess.run(["sudo", "-n", exe] + parts[1:],
                       capture_output=True, text=True, timeout=30)
    out = (p.stdout + p.stderr).strip()
    return p.returncode == 0, out or f"(exit={p.returncode})"


def handle_turn(user_text: str):
    """一轮对话: 路由 → 执行 → 汇总回复"""
    if not user_text.strip():
        return {"reply": "没听清，再说一遍？", "command": "", "ok": False}
    cmd, raw = route(user_text)
    if "(无需调用硬件命令)" in cmd or "(无需调用硬件命令)" in raw:
        return {"reply": "这个不需要动硬件，有什么硬件指令随时说。", "command": "", "ok": True}
    if not cmd.startswith("hw "):
        return {"reply": f"我没看懂，能换个说法吗？(路由: {raw[:60]})", "command": cmd, "ok": False}
    ok, summary = exec_cmd(cmd)
    reply = ("已执行 ✓\n" + summary) if ok else (f"执行失败 ✗\n{summary}")
    return {"reply": reply, "command": cmd, "ok": ok}


PAGE = """<!doctype html><html><head><meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<title>lyco 对话</title>
<style>
body{font-family:system-ui;background:#0f1115;color:#e8e8e8;margin:0;display:flex;flex-direction:column;height:100dvh}
#hd{padding:10px 16px;background:#171a21;font-size:15px;color:#8ab4ff}
#chat{flex:1;overflow-y:auto;padding:12px 14px;display:flex;flex-direction:column;gap:10px}
.m{max-width:82%;padding:10px 14px;border-radius:14px;font-size:15px;white-space:pre-wrap;line-height:1.5}
.u{align-self:flex-end;background:#2b6cff;color:#fff;border-bottom-right-radius:4px}
.a{align-self:flex-start;background:#1e222b;border-bottom-left-radius:4px}
.cmd{color:#7fd18a;font-family:monospace;font-size:13px}
#bar{display:flex;gap:8px;padding:10px;background:#171a21}
#in{flex:1;background:#22262f;border:1px solid #333a45;color:#eee;border-radius:20px;padding:10px 16px;font-size:15px;outline:none}
button{background:#2b6cff;color:#fff;border:0;border-radius:20px;padding:10px 16px;font-size:15px}
#file{display:none}
</style></head><body>
<div id=hd>lyco · Radxa 本地助手 <span style="color:#666;font-size:12px">(全本地, 零云)</span></div>
<div id=chat></div>
<div id=bar>
 <input id=in placeholder="输入, 或点麦克风说话">
 <button id=mic>🎤</button>
 <button id=send>发送</button>
 <input id=file type=file accept="audio/*" capture=microphone>
 <button id=up title="语音文件">📁</button>
</div>
<script>
const chat = document.getElementById('chat'), inp = document.getElementById('in');
function add(who, text, cmd){
  const d = document.createElement('div'); d.className = 'm ' + who;
  d.textContent = text;
  if(cmd){ const c = document.createElement('div'); c.className='cmd'; c.textContent='▸ '+cmd; d.appendChild(c); }
  chat.appendChild(d); chat.scrollTop = chat.scrollHeight;
}
function speak(t){ try{ const u=new SpeechSynthesisUtterance(t.replace(/\\n/g,' '));
  u.lang='zh-CN'; u.rate=1.15; speechSynthesis.cancel(); speechSynthesis.speak(u);}catch(e){} }
async function turn(text){
  if(!text.trim()) return;
  add('u', text); inp.value='';
  const d = add('a', '…');
  const r = await fetch('/chat', {method:'POST',
    headers:{'Content-Type':'application/json'}, body: JSON.stringify({text})});
  const j = await r.json();
  d.textContent = j.reply;
  if(j.command){ const c=document.createElement('div'); c.className='cmd';
    c.textContent='▸ '+j.command; d.appendChild(c); }
  chat.scrollTop = chat.scrollHeight;
  speak(j.reply.split('\\n')[0]);
}
document.getElementById('send').onclick = () => turn(inp.value);
inp.onkeydown = e => { if(e.key==='Enter') turn(inp.value); };
document.getElementById('mic').onclick = () => document.getElementById('file').click();
document.getElementById('up').onclick = () => document.getElementById('file').click();
document.getElementById('file').onchange = e => {
  const f = e.target.files[0]; if(!f) return;
  add('u', '🎤 (语音消息)');
  const fd = new FormData(); fd.append('audio', f);
  const d = add('a', '识别中…');
  fetch('/asr', {method:'POST', body:fd}).then(r=>r.json()).then(async j => {
    d.remove(); await turn(j.text || '(没听清)');
  });
  e.target.value='';
};
add('a', '你好！我是跑在 Radxa 上的本地助手。\\n试试: "打开台灯" / "cpu 多少度" / "风扇调到 150"');
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
        n = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(n)
        if self.path == "/chat":
            try:
                text = json.loads(body).get("text", "")
            except Exception:
                text = ""
            self._send(200, json.dumps(handle_turn(text), ensure_ascii=False)
                       .encode("utf-8"), "application/json")
            return
        if self.path == "/asr":
            ct = self.headers.get("Content-Type", "")
            audio = body
            if "multipart/form-data" in ct:
                boundary = ct.split("boundary=")[-1].strip('"').encode()
                for part in body.split(b"--" + boundary):
                    if b"filename" in part or b"audio" in part:
                        _, _, data = part.partition(b"\r\n\r\n")
                        if data.endswith(b"\r\n"):
                            data = data[:-2]
                        if len(data) > 100:
                            audio = data
                        break
            t0 = time.time()
            text = ""
            try:
                segs, info = model.transcribe(io.BytesIO(audio), language="zh")
                text = "".join(s.text for s in segs).strip()
            except Exception as e:
                text = f"(解码失败: {e})"
            secs = round(time.time() - t0, 2)
            print(f"[asr] {secs}s -> {text!r}", flush=True)
            self._send(200, json.dumps({"text": text, "secs": secs},
                                       ensure_ascii=False).encode("utf-8"),
                       "application/json")
            return
        self._send(404, b"{}", "application/json")


if __name__ == "__main__":
    import os
    import ssl
    import threading

    threading.Thread(
        target=ThreadingHTTPServer(("0.0.0.0", 8000), H).serve_forever, daemon=True
    ).start()
    cert, key = "/home/radxa/certs/cert.pem", "/home/radxa/certs/key.pem"
    if os.path.exists(cert):
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(cert, key)
        srv = ThreadingHTTPServer(("0.0.0.0", 8443), H)
        srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
        print("chat ui: http://:8000 + https://:8443", flush=True)
        srv.serve_forever()
    else:
        ThreadingHTTPServer(("0.0.0.0", 8000), H).serve_forever()
