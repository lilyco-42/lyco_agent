#!/usr/bin/env python3
"""lyv v0.1 — Lyco Learning Video knowledge pipeline (lyco_agent MVP)

设计修正已内化: sidecar 元数据(不发明编码) / ASR 领域词典纠错 / OCR 失败量化判定
/ ASR句子边界×场景检测交叉校验 / 词典精确命中优先+FTS兜底的混合检索 / grounding check

依赖: ffmpeg+ffprobe+tesseract 必需; opencv-python-headless 可选(vnn 级联,
      缺失时 grounding 失败诚实降级到学习队列)
      CloudStudio ASR: CS_COOKIE 或 CS_JPS+CS_TOKEN 环境变量 + node + cute_box/cs_*.mjs

build: video [+srt] -> .lyv pack (source + transcript + frames + ocr + segments.jsonl + sqlite)
query: text -> 混合检索(intent词典优先,FTS兜底) -> OCR grounding verify -> evidence
       verify 失败 -> VNN 特征激活识图 -> 学习队列
cut:   按时间戳按需切片(构建时不物理切分)
"""
import argparse, json, os, re, shutil, sqlite3, subprocess, sys
from pathlib import Path

FFMPEG, FFPROBE, TESS = "ffmpeg", "ffprobe", "tesseract"
# CloudStudio 远端通道(约束: NN 训练/推理只在 CS, 本地只做确定性编排)
# 需要: CS_COOKIE 或 CS_JPS+CS_TOKEN (node + D:/Code/cute_box/cs_*.mjs)
CS_BOX = os.environ.get("LYV_CS_BOX", r"D:\Code\cute_box")

def run(cmd, binary=False):
    return subprocess.run(cmd, capture_output=True, text=not binary)

def probe_duration(video):
    r = run([FFPROBE, "-v", "error", "-show_entries", "format=duration",
             "-of", "default=nw=1:nk=1", str(video)])
    return float(r.stdout.strip())

def scene_cuts(video, thr=0.3):
    """ffmpeg scdet 场景切换时间点(秒) — 用于句子边界交叉校验"""
    r = run([FFMPEG, "-v", "info", "-i", str(video),
             "-vf", f"select='gt(scene,{thr})',metadata=print", "-f", "null", "-"])
    return sorted({float(m.group(1)) for m in
                   re.finditer(r"pts_time:([\d.]+)", r.stderr + r.stdout)})

def adjust_boundaries(segs, cuts, win=1.5):
    """句子边界≠画面边界: 若场景切换落在句尾±win 内, 取更靠后者(保证画面动作完整)"""
    out, i = [], 0
    for t0, t1, tx in segs:
        for c in cuts:
            if t1 - win <= c <= t1 + win and c > t1:
                t1 = c + 0.1
        out.append((t0, t1, tx))
    return out

# ---------- transcript ----------
def parse_srt(path):
    txt = Path(path).read_text(encoding="utf-8-sig")
    def ts(p):
        h, m, rest = p.split(":"); s, ms = rest.split(",")
        return int(h) * 3600 + int(m) * 60 + int(s) + int(ms) / 1000
    out = []
    for b in re.split(r"\n\s*\n", txt.strip()):
        lines = [l for l in b.splitlines() if l.strip()]
        if len(lines) >= 3 and "-->" in lines[1]:
            a, z = lines[1].split("-->")
            out.append((ts(a.strip()), ts(z.strip()), " ".join(lines[2:]).strip()))
    return out

def whisper_transcribe(wav, lang="zh"):
    """本地 whisper(仅显式回退; 默认约束 NN 推理只在 CloudStudio)"""
    from faster_whisper import WhisperModel
    model = WhisperModel("tiny", device="cpu", compute_type="int8")
    segs, _ = model.transcribe(str(wav), language=lang, vad_filter=False)
    return [(s.start, s.end, s.text.strip()) for s in segs if s.text.strip()]

def cs_transcribe(wav, lang="zh", model_size="small", initial_prompt="终端操作演示视频: 命令行与 GUI 操作, 简体中文字幕。"):
    """CloudStudio 远端 ASR: 上传 wav -> cs_exec 跑 faster-whisper -> 返回 (t0,t1,text) 列表。
    依赖 CS_COOKIE 或 CS_JPS+CS_TOKEN 环境变量。默认 small: A10 上 small 与 tiny 同级耗时, 质量远优"""
    if not (os.environ.get("CS_COOKIE") or
            (os.environ.get("CS_JPS") and os.environ.get("CS_TOKEN"))):
        return None  # 未配置远端通道, 让调用方决定回退
    # Jupyter contents API 路径相对 root(/workspace), 需相对路径且父目录须存在
    remote = f"lyv_tmp/{Path(wav).name}"
    run(["node", str(Path(CS_BOX) / "cs_exec.mjs"), "import os; os.makedirs('/workspace/lyv_tmp', exist_ok=True); print('dir ok')"])
    up = run(["node", str(Path(CS_BOX) / "cs_upload.mjs"), str(wav), remote])
    # Windows 上 node 退出时偶发 libuv 断言(0xC0000409), 不能用 returncode 判成败, 看 stdout
    if "HTTP 2" not in up.stdout:
        print(f"[cs] upload FAIL: {up.stdout[-200:]} {up.stderr[-200:]}", file=sys.stderr)
        return None
    code = (f"from faster_whisper import WhisperModel\n"
            f"m = WhisperModel({model_size!r}, device='cuda', compute_type='float16')\n"
            f"segs, _ = m.transcribe('/workspace/{remote}', language={lang!r}, vad_filter=False,\n"
            f"                        initial_prompt={initial_prompt!r})\n"
            f"import json; print('@@LYV@@' + json.dumps("
            f"[[round(s.start,3), round(s.end,3), s.text.strip()] for s in segs if s.text.strip()],"
            f" ensure_ascii=False))")
    ex = run(["node", str(Path(CS_BOX) / "cs_exec.mjs"), code])
    out = ex.stdout
    if "@@LYV@@" not in out:
        print(f"[cs] exec FAIL: {(out + ex.stderr)[-400:]}", file=sys.stderr)
        return None
    rows = json.loads(out.split("@@LYV@@", 1)[1].splitlines()[0])
    return [(a, b, t) for a, b, t in rows if t]

def transcribe(wav, lang="zh"):
    """远端优先, 未配置/失败时回退本地(打印警示, 保持诚实降级)"""
    r = cs_transcribe(wav, lang)
    if r is not None:
        return r, "whisper-tiny@cloudstudio"
    print("[lyv] 警告: CloudStudio 未配置, 回退本地 CPU whisper (违反 NN-远端约束)", file=sys.stderr)
    return whisper_transcribe(wav, lang), "whisper-tiny-local-fallback"

# ---------- ASR 领域词典纠错 ----------
DOMAIN_CMDS = ["cargo new", "cargo run", "cargo build", "cargo test", "cargo init",
               "git clone", "git push", "git commit", "pip install", "npm install",
               "cd hello", "mkdir", "hello world", "hello", "new"]

def lev(a, b):
    if abs(len(a) - len(b)) > 2: return 99
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j-1] + 1, prev[j-1] + (ca != cb)))
        prev = cur
    return prev[-1]

def correct_asr(text):
    """专有名词/命令容易被 ASR 打错: 词级模糊匹配(编辑距离<=1, 词长>=4)矫正到领域词典"""
    def fix_word(w):
        wl = w.lower()
        if len(wl) < 4: return w
        for cmd in DOMAIN_CMDS:
            for kw in cmd.split():
                if wl != kw and lev(wl, kw) <= 1:
                    return kw
        return w
    fixed = " ".join(fix_word(w) if ASCII_ONLY.match(w) else w
                     for w in text.split())
    # 二次: 矫正后命中完整命令短语
    low = fixed.lower()
    for cmd in sorted(DOMAIN_CMDS, key=len, reverse=True):
        if cmd in low and cmd not in fixed:
            pass
    return fixed

ASCII_ONLY = re.compile(r"^[A-Za-z0-9_\-]+$")
ASCII_TOK = re.compile(r"[A-Za-z][A-Za-z0-9_\-]*")

def tokens(text):
    t = [w.lower() for w in ASCII_TOK.findall(text)]
    zh = re.sub(r"[^\u4e00-\u9fff]", " ", text)
    t += [zh[i:i+2] for i in range(len(zh) - 1) if zh[i:i+2].strip()]
    return t

# ---------- Event Miner ----------
INTENTS = {  # (tool, subcommand) -> intent id
    ("cargo", "new"): "rust.project.create", ("cargo", "run"): "rust.project.run",
    ("cargo", "build"): "rust.project.build", ("cargo", "init"): "rust.project.init",
    ("git", "clone"): "git.clone", ("git", "push"): "git.push",
    ("pip", "install"): "py.pkg.install", ("npm", "install"): "node.pkg.install",
}
PREREQ = {"rust.project.create": ["rust.cargo-installed"],
          "rust.project.run": ["rust.project.create"]}
# 已知工具的合法子命令: 工具词命中后, 子命令必属此集合(编辑距离<=1 纠错, 如 ASR "cargo no"->"cargo new")
SUBCMDS = {"cargo": {"new", "run", "build", "test", "init"},
           "git": {"clone", "push", "pull", "commit", "init"},
           "pip": {"install"}, "npm": {"install"}}

def mine_event(text, ocr_tokens):
    """规则版 Event Miner v0: text -> intent/command/entities/strong/weak
    强关联 = 文本∩OCR 双源命中(跨模态互证); 其余为弱关联"""
    cm = re.search(r"\b(cargo|git|npm|pip|python|cd|mkdir|ping|ls|dir)\b\s*([^\n。；;，,]*)",
                   text, re.I)
    entities, strong, weak = [], set(), set()
    tool = sub = cmd = None
    if cm:
        tool = cm.group(1).lower()
        rest = cm.group(2).strip()
        sub = rest.split(" ")[0].lower() if rest else None
        # 上下文纠错: 工具词命中后, 子命令按编辑距离矫正到合法集合(集合小+上下文强, 允许 dist<=2, 如 no->new)
        if tool in SUBCMDS and sub and sub not in SUBCMDS[tool]:
            cand = min(SUBCMDS[tool], key=lambda s: lev(sub, s))
            if lev(sub, cand) <= 2:
                rest = cand + rest[len(sub):]
                sub = cand
        cmd = (tool + " " + rest).strip()
        entities = [tool] + ([sub] if sub else []) + ASCII_TOK.findall(rest)[1:]
        intent = INTENTS.get((tool, sub), f"{tool}.{sub or 'run'}")
    else:
        intent = "misc.talk"
    tt = set(tokens(text))
    if tool:
        for e in [tool] + ([sub] if sub else []):
            (strong if e in ocr_tokens else weak).add(e)
    for t in tt:
        if len(t) < 2 or t.isdigit(): continue
        if t in ocr_tokens: strong.add(t)
        else: weak.add(t)
    return {"intent": intent, "command": cmd, "entities": entities,
            "strong": sorted(strong), "weak": sorted(weak - strong)[:12]}

# ---------- OCR + 量化失败判定 ----------
def ocr_frame(img, lang="eng"):
    r = run([TESS, str(img), "stdout", "--psm", "6", "-l", lang])
    txt = r.stdout.strip()
    # tsv 需经输出文件; scoop 版 tesseract 不认 'tsv' configfile, 用 -c tessedit_create_tsv=1
    base = img if isinstance(img, Path) else Path(img)
    stem = base.with_suffix("")
    r2 = run([TESS, str(base), str(stem), "-c", "tessedit_create_tsv=1", "-l", lang, "--psm", "6"])
    confs = []
    tsv = stem.with_suffix(".tsv")
    if r2.returncode == 0 and tsv.exists():
        for line in tsv.read_text(encoding="utf-8", errors="replace").splitlines()[1:]:
            p = line.split("\t")
            if len(p) >= 12 and re.match(r"^\d+(\.\d+)?$", p[10].strip()) and float(p[10]) >= 0:
                confs.append(float(p[10]))
        tsv.unlink()
    conf = sum(confs) / len(confs) / 100 if confs else 0.0
    return txt, conf

def frame_sharpness(img):
    """传统 CV 轻量打分: Laplacian 方差(清晰度)。numpy 缺失返回 None"""
    try:
        import numpy as np
    except ImportError:
        return None
    raw = run([FFMPEG, "-v", "error", "-i", str(img), "-vf", "format=gray,scale=320:-1",
               "-f", "rawvideo", "-pix_fmt", "gray", "-"], binary=True)
    if raw.returncode != 0 or not raw.stdout: return None
    a = np.frombuffer(raw.stdout, dtype=np.uint8)
    w = 320; h = len(a) // w
    a = a[:h*w].reshape(h, w).astype(float)
    lap = a[1:-1,1:-1]*4 - a[:-2,1:-1] - a[2:,1:-1] - a[1:-1,:-2] - a[1:-1,2:]
    return float(lap.var())

def ocr_pass(ocr_text, ocr_conf, expected_tokens, min_conf=0.5, max_dist=2):
    """量化 fallback 判定: 置信度>=阈值 且 非空 且 与预期词精确/编辑距离命中"""
    if not ocr_text or ocr_conf < min_conf: return False
    ot = set(tokens(ocr_text))
    if any(e in ot for e in expected_tokens): return True
    for w in ocr_text.split():
        for e in expected_tokens:
            if len(e) >= 4 and lev(w.lower(), e) <= max_dist:
                return True
    return False

# ---------- build ----------
def srt_dump(segs):
    def fmt(t):
        h, m = int(t // 3600), int(t % 3600 // 60)
        return f"{h:02d}:{m:02d}:{t % 60:06.3f}".replace(".", ",")
    return "\n".join(f"{i}\n{fmt(a)} --> {fmt(b)}\n{tx}\n"
                     for i, (a, b, tx) in enumerate(segs, 1))

def build(video, out, srt=None, lang="zh", ocr_lang="eng", frames_per_unit=2):
    video, out = Path(video), Path(out)
    if out.exists(): shutil.rmtree(out)
    for d in ("transcript", "frames", "ocr", "knowledge", "index"):
        (out / d).mkdir(parents=True, exist_ok=True)
    shutil.copy2(video, out / ("source" + video.suffix))
    dur = probe_duration(video)

    if srt:
        segs = parse_srt(srt); asr_src = "srt"
    else:
        wav = out / "transcript" / "audio.wav"
        run([FFMPEG, "-y", "-v", "error", "-i", str(video), "-vn",
             "-ac", "1", "-ar", "16000", str(wav)])
        segs, asr_src = transcribe(wav, lang)
    segs = [(a, b, correct_asr(t)) for a, b, t in segs]
    segs = adjust_boundaries(segs, scene_cuts(video))
    (out / "transcript" / "transcript.srt").write_text(srt_dump(segs), encoding="utf-8")

    units = []
    for i, (t0, t1, text) in enumerate(segs):
        uid = f"u{i:03d}"
        best = None
        for k in range(frames_per_unit):
            tm = t0 + (t1 - t0) * (k + 0.5) / frames_per_unit
            f = out / "frames" / f"{uid}_f{k}.webp"
            run([FFMPEG, "-y", "-v", "error", "-ss", f"{tm:.3f}", "-i", str(video),
                 "-frames:v", "1", str(f)])
            otxt, conf = ocr_frame(f, ocr_lang)
            sharp = frame_sharpness(f)
            (out / "ocr" / f"{uid}_f{k}.json").write_text(json.dumps(
                {"text": otxt, "conf": conf, "sharpness": sharp}, ensure_ascii=False),
                encoding="utf-8")
            score = len(set(tokens(text)) & set(tokens(otxt))) * 2 + conf \
                    + (sharp or 0) / 1e6
            if best is None or score > best[0]:
                best = (score, tm, f, otxt, conf)
        _, ftm, frame, otxt, oconf = best
        ev = mine_event(text, set(tokens(otxt)))
        units.append({"id": uid, "t0": round(t0, 2), "t1": round(t1, 2), "text": text,
                      "frame": str(frame.relative_to(out)), "frame_t": round(ftm, 2),
                      "ocr": otxt, "ocr_conf": round(oconf, 3),
                      "prereq": PREREQ.get(ev["intent"], []), **ev})

    (out / "knowledge" / "segments.jsonl").write_text(
        "\n".join(json.dumps(u, ensure_ascii=False) for u in units), encoding="utf-8")
    db = sqlite3.connect(out / "index" / "knowledge.sqlite")
    db.execute("CREATE TABLE segments(id TEXT PRIMARY KEY, t0 REAL, t1 REAL, text TEXT,"
               " intent TEXT, command TEXT, frame TEXT, ocr TEXT, ocr_conf REAL,"
               " strong TEXT, weak TEXT)")
    db.execute("CREATE VIRTUAL TABLE seg_fts USING fts5(id, text, entities, intent, strong)")
    for u in units:
        db.execute("INSERT INTO segments VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                   (u["id"], u["t0"], u["t1"], u["text"], u["intent"], u["command"],
                    u["frame"], u["ocr"], u["ocr_conf"], " ".join(u["strong"]),
                    " ".join(u["weak"])))
        db.execute("INSERT INTO seg_fts VALUES(?,?,?,?,?)",
                   (u["id"], " ".join(tokens(u["text"])),
                    " ".join(tokens(" ".join(u["entities"]))), u["intent"],
                    " ".join(tokens(" ".join(u["strong"])))))
    db.commit(); db.close()
    meta = {"format": "LYV 0.1", "video": video.name, "duration": dur,
            "asr_src": asr_src, "units": len(units)}
    (out / "meta.json").write_text(json.dumps(meta, ensure_ascii=False, indent=1), encoding="utf-8")
    print(json.dumps(meta, ensure_ascii=False))
    for u in units:
        print(f'  {u["id"]} [{u["t0"]}-{u["t1"]}] intent={u["intent"]} cmd={u["command"]!r} '
              f'strong={u["strong"]} ocr_conf={u["ocr_conf"]}')

# ---------- query (混合检索: intent 词典精确命中优先, FTS 兜底) ----------
RULES = [(r"(创建|新建|建立|create|new).{0,8}(项目|project)", "rust.project.create"),
         (r"(运行|跑|run).{0,8}(项目|project|程序)", "rust.project.run"),
         (r"进入|chdir|\bcd\b", "fs.chdir")]
def lookup(pack, q):
    """程序化检索 (供 agent 工具执行器调用): 返回 dict 或 None, 不打印"""
    pack = Path(pack)
    db = sqlite3.connect(pack / "index" / "knowledge.sqlite")
    db.row_factory = sqlite3.Row
    hit, how = None, None
    for pat, intent in RULES:
        if re.search(pat, q, re.I):
            hit = db.execute("SELECT * FROM segments WHERE intent=? "
                             "ORDER BY ocr_conf DESC LIMIT 1", (intent,)).fetchone()
            if hit: how = f"intent-dict:{intent}"; break
    if hit is None:
        # OR 语义: 中文 bigram 任一命中即可召回, rank 排序兜底(AND 语义会因缺词零召回)
        try:
            hit = db.execute("SELECT s.* FROM seg_fts f JOIN segments s ON s.id=f.id "
                             "WHERE seg_fts MATCH ? ORDER BY rank LIMIT 1",
                             (" OR ".join(tokens(q)),)).fetchone()
            how = "fts-fallback" if hit else None
        except sqlite3.OperationalError:
            hit = None  # MATCH 语法冲突等, 诚实零召回
    if hit is None:
        db.close()
        return None
    lines = (pack / "knowledge" / "segments.jsonl").read_text(encoding="utf-8").splitlines()
    u = json.loads(lines[int(hit["id"][1:])])
    result = {"retrieval": how, "intent": hit["intent"], "command": hit["command"],
              "t0": u["t0"], "t1": u["t1"], "frame": u["frame"], "frame_t": u["frame_t"],
              "text": u["text"], "strong": u["strong"], "weak": u["weak"][:6],
              "prereq": u.get("prereq", []), "ocr_conf": u["ocr_conf"],
              "id": u["id"]}
    db.close()
    return result

def query(pack, q, verify=True, ocr_lang="eng"):
    hit = lookup(pack, q)
    if hit is None:
        print("NO_HIT -> 入学习队列"); return
    print(f"Retrieval:  {hit['retrieval']}")
    print(f"Intent:     {hit['intent']}  Command: {hit['command']}")
    print(f"Evidence:   {hit['t0']}s -> {hit['t1']}s  keyframe={hit['frame']} (t={hit['frame_t']}s)")
    print(f"Text:       {hit['text']}")
    print(f"Strong:     {hit['strong']}   Weak: {hit['weak'][:6]}")
    print(f"Prereq:     {hit['prereq']}")
    if verify:
        otxt, conf = ocr_frame(pack / hit["frame"], ocr_lang)
        expect = set(hit["strong"]) | (set(tokens(hit["command"])) if hit["command"] else set())
        ok = ocr_pass(otxt, conf, expect)
        print(f"Grounding:  re-OCR conf={conf:.2f} expect={sorted(expect)}")
        if ok:
            print("            VERIFY_PASS")
        else:
            # 级联: OCR 失败 → VNN 特征激活识图 (vnn_proto, 本地 CPU)
            vnn = _vnn_identify(pack / hit["frame"], otxt, conf)
            if vnn:
                print(f"            VERIFY_FAIL -> VNN: verdict={vnn['verdict']} "
                      f"conf={vnn['conf']} activated={[e['neuron'] for e in vnn['activated']]}")
                if vnn.get("learning_queue"):
                    print(f"            学习队列: {vnn['learning_queue']}")
            else:
                print("            VERIFY_FAIL -> 学习队列 (VNN 不可用: 缺 opencv)")
        print(f"Clip:       ffmpeg -ss {hit['t0']} -to {hit['t1']} -i source.* -c copy clip_{hit['id'] if 'id' in hit else ''}.mp4")

def _vnn_identify(frame, ocr_text, ocr_conf, min_conf=0.5):
    """调用 VNN 原型(opencv 缺失时返回 None, 级联诚实降级到学习队列)"""
    try:
        sys.path.insert(0, str(Path(__file__).parent))
        from vnn_proto import identify
    except ImportError:
        return None
    try:
        return identify(frame, ocr_text=ocr_text, ocr_conf=ocr_conf, min_conf=min_conf)
    except Exception as e:  # 单帧失败不阻塞查询
        print(f"[vnn] FAIL: {e}", file=sys.stderr)
        return None

# ---------- cut ----------
def cut(pack, uid, out_clip):
    pack = Path(pack)
    segs = [json.loads(l) for l in (pack / "knowledge" / "segments.jsonl")
            .read_text(encoding="utf-8").splitlines()]
    u = next(x for x in segs if x["id"] == uid)
    src = next(pack.glob("source.*"))
    r = run([FFMPEG, "-y", "-v", "error", "-ss", str(u["t0"]), "-to", str(u["t1"]),
             "-i", str(src), "-c", "copy", str(out_clip)])
    print("OK" if r.returncode == 0 else "FAIL", out_clip)

if __name__ == "__main__":
    ap = argparse.ArgumentParser(prog="lyv")
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build"); b.add_argument("video"); b.add_argument("out")
    b.add_argument("--srt"); b.add_argument("--lang", default="zh")
    b.add_argument("--ocr-lang", default="eng"); b.add_argument("--frames", type=int, default=2)
    q = sub.add_parser("query"); q.add_argument("pack"); q.add_argument("q")
    q.add_argument("--no-verify", action="store_true"); q.add_argument("--ocr-lang", default="eng")
    c = sub.add_parser("cut"); c.add_argument("pack"); c.add_argument("uid"); c.add_argument("out")
    a = ap.parse_args()
    if a.cmd == "build": build(a.video, a.out, a.srt, a.lang, a.ocr_lang, a.frames)
    elif a.cmd == "query": query(a.pack, a.q, not a.no_verify, a.ocr_lang)
    elif a.cmd == "cut": cut(a.pack, a.uid, a.out)
