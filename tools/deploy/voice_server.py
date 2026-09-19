#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""voice_server.py — lyco 对话端 (DeepSeek 风格 UI + 语音)

全链在板上闭环, 零云:
  手机浏览器 → ASR(faster-whisper) → 路由器(llama-server, 本地 0.6B) → hw CLI → 灯
安全: 只执行 `hw ` 前缀命令 (白名单), 其余一律拒绝。
"""
import io
import json
import os
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
<title>lyco · 本地助手</title>
<style>
*{box-sizing:border-box} body{font-family:system-ui,-apple-system,"Segoe UI",sans-serif;margin:0;background:#f3f4f6;color:#1a1a1a;height:100dvh;display:flex;flex-direction:column}
#hd{background:#fff;border-bottom:1px solid #e5e7eb;padding:12px 20px;display:flex;align-items:center;gap:10px}
#hd .logo{width:30px;height:30px;border-radius:8px;background:#4D6BFE;color:#fff;display:flex;align-items:center;justify-content:center;font-weight:700}
#hd .t{font-weight:600;font-size:15px} #hd .s{font-size:12px;color:#9ca3af;margin-left:auto}
#chat{flex:1;overflow-y:auto;width:100%;max-width:760px;margin:0 auto;padding:24px 16px;display:flex;flex-direction:column}
.welcome{text-align:center;margin-top:8vh}
.welcome .big{font-size:26px;font-weight:700} .welcome .big em{font-style:normal;color:#4D6BFE}
.welcome .sub{color:#6b7280;font-size:14px;margin-top:10px}
.chips{display:flex;flex-wrap:wrap;gap:10px;justify-content:center;margin-top:26px}
.chip{background:#fff;border:1px solid #e5e7eb;border-radius:12px;padding:12px 16px;font-size:14px;cursor:pointer;transition:.15s}
.chip:hover{border-color:#4D6BFE;color:#4D6BFE}
.m{max-width:78%;padding:12px 16px;border-radius:16px;font-size:15px;white-space:pre-wrap;line-height:1.6;margin-top:16px;word-break:break-word}
.u{align-self:flex-end;background:#4D6BFE;color:#fff;border-bottom-right-radius:4px}
.a{align-self:flex-start;background:#fff;border:1px solid #e5e7eb;border-bottom-left-radius:4px}
.a.cmd{color:#059669;font-family:ui-monospace,monospace;font-size:13px;margin-top:-10px;padding:8px 14px}
.dots span{display:inline-block;width:6px;height:6px;border-radius:50%;background:#9ca3af;margin-right:4px;animation:b 1.2s infinite}
.dots span:nth-child(2){animation-delay:.2s}.dots span:nth-child(3){animation-delay:.4s}
@keyframes b{0%,80%,100%{opacity:.25}40%{opacity:1}}
#bar{background:#f3f4f6;padding:12px 16px 20px}
#box{max-width:760px;margin:0 auto;background:#fff;border:1px solid #e5e7eb;border-radius:18px;padding:10px 12px;display:flex;gap:8px;align-items:center;box-shadow:0 2px 10px rgba(0,0,0,.04)}
#in{flex:1;border:0;outline:none;font-size:15px;background:transparent;color:#1a1a1a}
.ib{width:38px;height:38px;border-radius:50%;border:0;background:#eef1f6;cursor:pointer;font-size:16px;display:flex;align-items:center;justify-content:center}
#send{background:#4D6BFE;color:#fff}
#file{display:none}
</style></head><body>
<div id=hd><div class=logo>ly</div><div class=t>lyco 助手</div><div class=s>Radxa 本地运行 · 零云</div></div>
<div id=chat>
 <div class=welcome id=wel>
  <div class=big>嗨，我是 <em>lyco</em></div>
  <div class=sub>跑在你家 Radxa 上的本地助手 —— 语音或文字，全在板上完成，数据不出门</div>
  <div class=chips>
   <div class=chip onclick=go('打开台灯')>💡 打开台灯</div>
   <div class=chip onclick=go('把蓝灯关了')>⏻ 关掉蓝灯</div>
   <div class=chip onclick=go('cpu 多少度')>🌡️ cpu 温度</div>
   <div class=chip onclick=go('风扇调到 150')>🌀 风扇调速</div>
   <div class=chip onclick=go('看看内存')>💾 内存状态</div>
  </div>
 </div>
</div>
<div id=bar><div id=box>
 <input id=in placeholder="输入指令，或点 🎤 说话">
 <button class=ib id=mic title=语音>🎤</button>
 <button class=ib id=send title=发送>➤</button>
 <input id=file type=file accept="audio/*" capture=microphone>
 <button class=ib id=up title=语音文件>📎</button>
</div></div>
<script>
const chat=document.getElementById('chat'), inp=document.getElementById('in');
function add(cls,text,cmd){
  const d=document.createElement('div'); d.className='m '+cls; d.textContent=text;
  chat.appendChild(d); chat.scrollTop=chat.scrollHeight; return d;
}
function addcmd(cmd){ const c=document.createElement('div'); c.className='a cmd';
  c.textContent='▸ '+cmd; chat.appendChild(c); chat.scrollTop=chat.scrollHeight; }
function go(t){ turn(t); }
function speak(t){ try{const u=new SpeechSynthesisUtterance(t.replace(/\\n/g,' '));
 u.lang='zh-CN';u.rate=1.15;speechSynthesis.cancel();speechSynthesis.speak(u);}catch(e){} }
async function turn(text){
  if(!text.trim()) return;
  const w=document.getElementById('wel'); if(w) w.remove();
  add('u',text); inp.value='';
  const d=add('a',''); d.innerHTML='<span class=dots><span></span><span></span><span></span></span>';
  const r=await fetch('/chat',{method:'POST',headers:{'Content-Type':'application/json'},
    body:JSON.stringify({text})});
  const j=await r.json();
  d.textContent=j.reply;
  if(j.command) addcmd(j.command);
  chat.scrollTop=chat.scrollHeight; speak(j.reply.split('\\n')[0]);
}
document.getElementById('send').onclick=()=>turn(inp.value);
inp.onkeydown=e=>{if(e.key==='Enter')turn(inp.value);};
let rec=null, chunks=[], micBusy=false;
const mic=document.getElementById('mic');
mic.onclick=async()=>{
  if(rec&&rec.state==='recording'){rec.stop();return;}
  if(!window.isSecureContext){
    add('a','⚠️ 当前是 http 页面，浏览器禁用麦克风。请改用 https://192.168.10.69:8443 （接受一次证书警告即可），或点 📎 上传语音文件。');
    return;
  }
  if(micBusy) return; micBusy=true;
  try{
    const stream=await navigator.mediaDevices.getUserMedia({audio:true});
    rec=new MediaRecorder(stream); chunks=[];
    rec.ondataavailable=ev=>chunks.push(ev.data);
    rec.onstop=async()=>{
      stream.getTracks().forEach(t=>t.stop());
      mic.textContent='🎤'; micBusy=false;
      const blob=new Blob(chunks);
      if(blob.size>1000) await sendAudio(blob);
    };
    rec.start(); mic.textContent='⏹ 停止';
  }catch(err){ micBusy=false; add('a','🎤 麦克风不可用: '+err.message+' — 可点 📎 上传语音文件。'); }
};
document.getElementById('up').onclick=()=>document.getElementById('file').click();
document.getElementById('file').onchange=e=>{
  const f=e.target.files[0]; if(!f) return;
  if(f.size>1000) sendAudio(f);
  e.target.value='';
};
async function sendAudio(blob){
  const w=document.getElementById('wel'); if(w) w.remove();
  add('u','🎤 (语音)');
  const d=add('a',''); d.innerHTML='<span class=dots><span></span><span></span><span></span></span>';
  const fd=new FormData(); fd.append('audio',blob);
  try{
    const r=await fetch('/asr',{method:'POST',body:fd});
    const j=await r.json(); d.remove();
    await turn(j.text||'(没听清)');
  }catch(e){ d.textContent='识别失败: '+e.message; }
}
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
        # ---- 算力平台 agent 友好的裸字节端点 (暴露本机能力给 lain42.top/compute) ----
        if self.path.startswith("/node/"):
            import subprocess
            out = b""
            try:
                if self.path == "/node/hw":
                    # 白名单: 只允许 hw 命令
                    cmd = body.decode("utf-8", "replace").strip()
                    if not cmd.startswith("hw "):
                        out = b"ERR: only hw commands allowed"
                    else:
                        parts = cmd.split()
                        p = subprocess.run(["sudo", "-n", "/usr/local/bin/" + parts[0]] + parts[1:],
                                           capture_output=True, timeout=60)
                        out = (p.stdout + p.stderr)[:65536]
                elif self.path == "/node/chat":
                    text = body.decode("utf-8", "replace").strip()
                    r = handle_turn(text)
                    out = (r.get("reply", "") + "\n" + r.get("command", "")).encode("utf-8")
                elif self.path == "/node/asr":
                    import io as _io
                    import wave as _wave
                    from faster_whisper.audio import decode_audio
                    pcm = decode_audio(_io.BytesIO(body), sampling_rate=16000)
                    with _wave.open("/home/radxa/node_asr.wav", "wb") as w:
                        w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000)
                        w.writeframes((pcm * 32767).astype("int16").tobytes())
                    segs, _ = model.transcribe("/home/radxa/node_asr.wav", language="zh",
                                               hotwords="台灯 蓝灯 绿灯 风扇 温度 打开 关闭 灯 关")
                    out = "".join(s.text for s in segs).strip().encode("utf-8")
                elif self.path == "/node/ve2":
                    # body: 第一行 "WxH", 其后为 NV12 裸帧 -> H.264 (硬件编码)
                    head, _, raw = body.partition(b"\n")
                    wh = head.decode().strip()
                    w_, h_ = [int(x) for x in wh.lower().split("x")]
                    fr = len(raw) // (w_ * h_ * 3 // 2)
                    open("/tmp/ve2_in.yuv", "wb").write(raw)
                    p = subprocess.run(["sudo", "-n", "/usr/bin/vencoderdemo",
                                        "-i", "/tmp/ve2_in.yuv", "-n", str(fr), "-f", "0",
                                        "-o", "/tmp/ve2_out.h264", "-s", wh, "-d", wh,
                                        "-r", "30", "-enc_num", "1"],
                                       capture_output=True, timeout=900)
                    out = open("/tmp/ve2_out.h264", "rb").read() if p.returncode == 0 else b"ERR: ve2 encode failed"
                elif self.path == "/node/kws":
                    # body: 16k 单声道 wav -> NPU 唤醒词判定
                    open("/tmp/kws_in.wav", "wb").write(body)
                    d = "/home/radxa/npu_demos/kws_npu_demo"
                    env = dict(os.environ, LD_LIBRARY_PATH="/home/radxa/lib")
                    p = subprocess.run([d + "/kws_npu_demo_a733",
                                        "-nb0", "/home/radxa/npu_demos/voice_assistant/prebuilt/kws/encoder_float_a733.nb",
                                        "-nb1", "/home/radxa/npu_demos/voice_assistant/prebuilt/kws/decoder_float_a733.nb",
                                        "-nb2", "/home/radxa/npu_demos/voice_assistant/prebuilt/kws/joiner_float_a733.nb",
                                        "-i", "/tmp/kws_in.wav",
                                        "-k", "/home/radxa/npu_demos/voice_assistant/keywords_wake.txt"],
                                       capture_output=True, timeout=300, cwd=d, env=env)
                    txt = (p.stdout + p.stderr).decode("utf-8", "replace")
                    hits = [l for l in txt.splitlines() if l.startswith("HITS")]
                    out = ("\n".join(hits) or "HITS: (none)").encode("utf-8")
                else:
                    out = b"ERR: unknown node endpoint"
            except Exception as e:
                out = ("ERR: %s: %s" % (type(e).__name__, e)).encode("utf-8")
            self._send(200, out, "application/octet-stream")
            return
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
            # 先把上传音频解码成 16k 单声道 WAV, 再对该 WAV 转写。
            # 踩坑: 直接把原始上传字节(BytesIO)喂 transcribe 时 PyAV 有时解不出音频
            # (返回空且极快), 而 decode_audio + WAV 文件这条路径稳定。
            WAV = "/home/radxa/last_audio.wav"
            try:
                from faster_whisper.audio import decode_audio
                import wave
                pcm = decode_audio(io.BytesIO(audio), sampling_rate=16000)
                with wave.open(WAV, "wb") as w:
                    w.setnchannels(1)
                    w.setsampwidth(2)
                    w.setframerate(16000)
                    w.writeframes((pcm * 32767).astype("int16").tobytes())
                print(f"[asr] 已保存 {WAV} ({len(pcm)/16000:.2f}s)", flush=True)
            except Exception as e:
                print(f"[asr] 解码/保存失败: {e}", flush=True)
            try:
                # tiny int8 是这块 CPU 的上限 (base 实测 16x 实时率, 不可用);
                # 准确率靠 hotwords 领域词 + initial_prompt 简体偏置, 不靠换大模型
                segs, info = model.transcribe(
                    WAV, language="zh",
                    initial_prompt="以下是普通话的句子。打开台灯。把蓝灯关掉。风扇调到一百五。",
                    hotwords="台灯 蓝灯 绿灯 风扇 温度 内存 磁盘 打开 关闭 亮 灯 调到")
                # ⚠️ 必须迭代 segs 才会真正执行转写 (transcribe 返回生成器)
                text = "".join(s.text for s in segs).strip()
                try:
                    from opencc import OpenCC
                    text = OpenCC("t2s").convert(text)
                except ImportError:
                    pass
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
