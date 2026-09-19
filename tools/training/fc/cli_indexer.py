# -*- coding: utf-8 -*-
"""cli_indexer.py — CLI 工具自动索引器 (基模学习能力强化)

能力: 给一个新 CLI 工具名 (如 jj), 自动:
  1. 发现: which --help → 解析 Commands 列表 (子命令名 + 一句话描述)
  2. 逐子命令: <tool> help <sub> → 解析 Usage/参数/说明
  3. 生成知识包: sqlit/FTS 索引 (intent=tool.sub, text=描述, strong=子命令词)
  4. 用户问 "jj 怎么看历史" → 检索命中 jj.log → 返回用法

这是「学习能力>Scaling Law」的运行时实现: 不重训模型, 运行时读手册。
lilyco 框架应用更优 — lilyco app --schema 直接给 JSON, 免解析。
"""
import json
import re
import sqlite3
import subprocess
import sys
from pathlib import Path


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=30)


def index_cli(tool, pack_dir, max_depth=1):
    """索引一个 CLI 工具: --help 解析 → 子命令 → 知识包"""
    help_out = run([tool, "--help"])
    if help_out.returncode != 0 and not help_out.stdout:
        return {"error": f"{tool} --help 失败"}

    # 解析顶层 Commands 段 (clippy 风格: "  name   描述")
    commands = {}
    in_commands = False
    for line in help_out.stdout.splitlines():
        if re.match(r"^(Commands|Subcommands):\s*$", line.strip()):
            in_commands = True
            continue
        if in_commands:
            m = re.match(r"^  (\S+)\s{2,}(.+)$", line)
            if m:
                commands[m.group(1)] = m.group(2).strip()
            elif line.strip() == "" and commands:
                break

    entries = [{"sub": None, "desc": f"{tool} 命令行工具",
                "help_text": help_out.stdout}]
    for sub in list(commands)[:60]:  # 上限 60 个子命令
        sub_help = run([tool, "help", sub]) or run([tool, sub, "--help"])
        entries.append({
            "sub": sub, "desc": commands[sub],
            "help_text": (sub_help.stdout or sub_help.stderr or "")[:4000],
        })

    build_pack(tool, entries, pack_dir)
    return {"tool": tool, "commands": len(commands), "entries": len(entries)}


def build_pack(tool, entries, pack_dir):
    pack = Path(pack_dir)
    for d in ["frames", "knowledge", "index"]:
        (pack / d).mkdir(parents=True, exist_ok=True)

    db = sqlite3.connect(pack / "index" / "knowledge.sqlite")
    db.executescript(
        """
        CREATE TABLE IF NOT EXISTS segments(id TEXT PRIMARY KEY, t0 REAL DEFAULT 0,
         t1 REAL DEFAULT 0, text TEXT, intent TEXT, command TEXT, frame TEXT,
         ocr TEXT, ocr_conf REAL DEFAULT 1.0, strong TEXT, weak TEXT);
        CREATE VIRTUAL TABLE IF NOT EXISTS seg_fts USING fts5(id, text, entities,
         intent, strong);
        """
    )
    # 清掉同工具旧条目 (幂等)
    for row in db.execute("SELECT id FROM segments WHERE intent LIKE ?", (f"{tool}.%",)):
        db.execute("DELETE FROM seg_fts WHERE id=?", row)
    db.execute("DELETE FROM segments WHERE intent LIKE ?", (f"{tool}.%",))

    def seg(text):
        out, buf = [], ""
        for ch in text:
            if ch.isspace():
                if buf: out.append(buf); buf = ""
                continue
            if ch.isascii():
                buf += ch
            else:
                if buf: out.append(buf); buf = ""
                out.append(ch)
        if buf: out.append(buf)
        return out

    for i, e in enumerate(entries):
        uid = f"{tool}_{i:03d}"
        intent = f"{tool}.{e['sub']}" if e["sub"] else f"{tool}.main"
        command = f"{tool} {e['sub']}" if e["sub"] else tool
        # strong = 子命令名 + 描述里的 ASCII 词
        strong = " ".join(seg(command))
        text = " ".join(seg(f"{command} : {e['desc']} : 用法见帮助"))
        db.execute(
            "INSERT OR REPLACE INTO segments VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            (uid, 0.0, 0.0, text, intent, command, "", e["help_text"][:200], 1.0,
             " ".join(strong), ""))
        db.execute(
            "INSERT OR REPLACE INTO seg_fts VALUES(?,?,?,?,?)",
            (uid, " ".join(seg(f"{command} {e['desc']}")),
             " ".join(seg(command)), intent, " ".join(strong)))

    # help_text 全文也入 FTS (第二表, 检索详情)
    db.executescript(
        "CREATE VIRTUAL TABLE IF NOT EXISTS help_fts USING fts5(intent, content);")
    for e in entries:
        intent = f"{tool}.{e['sub']}" if e["sub"] else f"{tool}.main"
        db.execute("INSERT OR REPLACE INTO help_fts VALUES(?,?)",
                   (intent, e["help_text"]))

    db.commit()
    db.close()
    meta = {"tool": tool, "entries": len(entries),
            "format": "LYV-CLI 0.1"}
    (pack / "knowledge" / f"{tool}_meta.json").write_text(
        json.dumps(meta, ensure_ascii=False), encoding="utf-8")


# 通用 zh→en 意图映射 (跨语言检索桥, 领域词典可由 rules.json 扩展)
ZH_INTENT_MAP = {
    "查看": ["show", "log", "list", "display"], "看": ["show", "log"],
    "历史": ["log", "history", "evolog"], "记录": ["log", "history"],
    "创建": ["new", "create", "init"], "新建": ["new", "create", "init"],
    "推送": ["push"], "拉取": ["pull", "fetch"], "克隆": ["clone"],
    "提交": ["commit", "describe"], "修改": ["edit", "describe", "diff"],
    "删除": ["abandon", "delete", "remove"], "安装": ["install"],
    "比较": ["diff"], "差异": ["diff"], "合并": ["merge", "rebase"],
    "配置": ["config"], "文件": ["file"], "书签": ["bookmark"],
    "分支": ["bookmark", "branch"], "冲突": ["conflict", "resolve"],
    "撤销": ["undo", "abandon", "restore"], "回滚": ["undo", "restore"],
    "路径": ["path", "output", "input"], "输出": ["output"], "输入": ["input"],
    "格式": ["format"], "质量": ["quality"], "宽度": ["width"],
    "高度": ["height"], "预览": ["dry-run", "preview"], "压缩": ["compress"],
    "大小": ["size", "resize"], "尺寸": ["resize", "width"],
}


def index_lilyco(schema_exe, pack_dir):
    """lilyco 框架应用: <app> --schema 输出结构化 JSON, 直接入库 (免解析)"""
    out = run([schema_exe, "--schema"])
    if out.returncode != 0 or not out.stdout.strip().startswith("{"):
        return {"error": f"{schema_exe} --schema 失败"}
    schema = json.loads(out.stdout)
    name = schema["name"].lower()
    about = schema.get("about", "")
    entries = [{"sub": None, "desc": about, "help_text": json.dumps(schema, ensure_ascii=False)}]
    for arg in schema.get("args", []):
        aname = arg["name"]
        aabout = arg.get("about", "")
        req = "必填" if arg.get("required") else "可选"
        entries.append({
            "sub": aname, "desc": f"参数 {aname} ({req}): {aabout}",
            "help_text": json.dumps(arg, ensure_ascii=False),
        })
    build_pack(name, entries, pack_dir)
    return {"tool": name, "args": len(schema.get("args", [])), "entries": len(entries)}


def lookup_help(pack_dir, query, tool=None):
    """查帮助: FTS 命中 intent → 返回该子命令完整 help_text"""
    db = sqlite3.connect(Path(pack_dir) / "index" / "knowledge.sqlite")
    db.row_factory = sqlite3.Row
    like = f"{tool}.%" if tool else "%"
    rows = db.execute(
        "SELECT intent FROM segments WHERE intent LIKE ? ORDER BY id", (like,)).fetchall()
    if not rows:
        return None
    best, best_score = None, -1
    query_lower = query.lower()
    qwords = set(re.findall(r"[a-z0-9_\-]+|[一-鿿]", query_lower))
    # 跨语言扩展: 中文意图词 → 英文同义词
    for zh, ens in ZH_INTENT_MAP.items():
        if zh in query_lower:
            qwords.update(ens)
    for row in rows:
        intent = row["intent"]
        h = db.execute("SELECT content FROM help_fts WHERE intent=?", (intent,)).fetchone()
        if not h:
            continue
        content = h["content"].lower()
        score = sum(1 for w in qwords if w in content)
        # intent 名本身也是信号 (query 里含子命令名)
        sub = intent.split(".")[-1] if "." in intent else ""
        if sub and sub in query.lower():
            score += 5
        # main 条目的 help_text 是全命令清单, 天然包含所有子命令词 → 惩罚避免吞掉所有查询
        if intent.endswith(".main"):
            score = int(score * 0.4)
        if score > best_score:
            best_score, best = score, (intent, h["content"])
    db.close()
    return best


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--lilyco":
        r = index_lilyco(sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else "smoke/lilyco_pack")
        print(json.dumps(r, ensure_ascii=False))
        sys.exit(0)
    ap_tool = sys.argv[1] if len(sys.argv) > 1 else "jj"
    pack = sys.argv[2] if len(sys.argv) > 2 else "smoke/jj_pack"
    result = index_cli(ap_tool, pack)
    print(json.dumps(result, ensure_ascii=False))
    if len(sys.argv) > 3:
        q = " ".join(sys.argv[3:])
        hit = lookup_help(pack, q, ap_tool)
        if hit:
            print(f"\n查询: {q}\n→ {hit[0]}\n{hit[1][:600]}")
