#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""patch_contact_bind.py —— 用户绑定 QQ/微信 (联系存档)

A) compute/server.py: users 加 qq/wechat 列;
   GET/POST /api/v1/profile/contact (GET 预填, POST 保存)
B) billing 管理用户列表带出联系方式 (经 identity_links 查 compute.db)
C) ui.html 积分 tab 加"联系方式"卡片 (预填 + 保存)
幂等; 自动备份。
"""
import shutil
import time

APPLY = "--apply" in __import__("sys").argv
ok = []

# ===== A) compute/server.py =====
CS = "/opt/compute/server.py"
s = open(CS, encoding="utf-8").read()

if "profile/contact" not in s:
    a1 = 'import sys as _sys, time as _tmod'
    if a1 in s:
        s = s.replace(a1, a1 + '''
try:
    db.execute("ALTER TABLE users ADD COLUMN qq TEXT")
except Exception:
    pass
try:
    db.execute("ALTER TABLE users ADD COLUMN wechat TEXT")
except Exception:
    pass''', 1)
        ok.append(("A1 qq/wechat 列(运行时 ensure)", True))

    handlers = '''    def profile_contact_get(self):
        u = self._auth()
        row = db.execute("SELECT qq, wechat FROM users WHERE id=?", (u[0],)).fetchone()
        return self._json({"qq": row[0] or None, "wechat": row[1] or None})

    def profile_contact_save(self):
        u = self._auth()
        body = json.loads(self._body(65536))
        qq = str(body.get("qq", "")).strip()
        wechat = str(body.get("wechat", "")).strip()
        if qq:
            if not qq.isdigit() or not 5 <= len(qq) <= 12:
                raise ValueError("QQ 号必须是 5-12 位数字")
        if len(wechat) > 40:
            raise ValueError("微信号过长")
        with db_lock:
            db.execute("UPDATE users SET qq=?, wechat=? WHERE id=?", (qq or None, wechat or None, u[0]))
            db.commit()
        return self._json({"ok": True, "qq": qq or None, "wechat": wechat or None})

'''
    anchor = "    def _owner_check(self, device_id):"
    if anchor in s:
        s = s.replace(anchor, handlers + anchor, 1)
        ok.append(("A2 handler", True))

    a3 = '''            if api[:2] == ["v1", "profile"] and len(api) == 3 and api[2] == "contact" and method == "GET":
                return self.profile_contact_get()
            if api[:2] == ["v1", "profile"] and len(api) == 3 and api[2] == "contact" and method == "POST":
                return self.profile_contact_save()
'''
    anchor3 = '            if api[:2] == ["v1", "withdraw"] and method == "POST":'
    if anchor3 in s:
        s = s.replace(anchor3, a3 + anchor3, 1)
        ok.append(("A3 路由", True))
else:
    ok.append(("compute", "已打过"))

# ===== B) billing 管理列表带联系方式 =====
BS = "/opt/studio-billing/app.py"
b = open(BS, encoding="utf-8").read()
if "contact_from_compute" not in b:
    b_anchor = '''    return {"actor": actor["actor"],
            "items": [dict(r) for r in items],
            "links": [dict(l) for l in links]}'''
    b_new = '''    # contact_from_compute: 从 compute.db 带出绑定的 QQ/微信 (经 identity_links)
    contacts = {}
    try:
        import sqlite3 as _sq3
        _cc = _sq3.connect("/var/lib/compute/compute.db", timeout=15)
        for l in links:
            if l["service"] == "compute":
                _r = _cc.execute("SELECT qq, wechat FROM users WHERE id=?", (l["external_id"],)).fetchone()
                if _r and (_r[0] or _r[1]):
                    contacts[l["uid"]] = {"qq": _r[0], "wechat": _r[1]}
        _cc.close()
    except Exception:
        pass
    return {"actor": actor["actor"],
            "items": [dict(r, contact=contacts.get(r["id"])) for r in items],
            "links": [dict(l) for l in links]}'''
    if b_anchor in b:
        b = b.replace(b_anchor, b_new, 1)
        ok.append(("B 管理列表带联系方式", True))
    else:
        ok.append(("B 管理列表带联系方式", False))
else:
    ok.append(("billing", "已打过"))

# ===== C) ui.html 联系方式卡片 =====
US = "/opt/compute/ui.html"
u = open(US, encoding="utf-8").read()
if 'id="ctGo"' not in u:
    wd_anchor = 'document.getElementById("creditBox").appendChild(_wd);'
    if wd_anchor in u:
        ct = wd_anchor + '''
    var _ct=document.createElement("div");
    _ct.innerHTML='<div class="card"><h3>联系方式绑定</h3><div class="row">QQ: <input id="ctQq" style="width:140px" placeholder="QQ 号(数字)"> 微信: <input id="ctWx" style="width:180px" placeholder="微信号"> <button class="primary" id="ctGo">保存</button></div><div class="meta">用于交易联系与账户找回 (不对外公开)。</div></div>';
    document.getElementById("creditBox").appendChild(_ct);
    api("/api/v1/profile/contact").then(function(d){
      document.getElementById("ctQq").value=d.qq||"";
      document.getElementById("ctWx").value=d.wechat||"";}).catch(function(){});
    document.getElementById("ctGo").addEventListener("click",function(){
      api("/api/v1/profile/contact",{method:"POST",body:JSON.stringify({qq:document.getElementById("ctQq").value.trim(),wechat:document.getElementById("ctWx").value.trim()})})
      .then(function(){alert("已保存");})
      .catch(function(e){alert("失败: "+e.message)});});'''
        u = u.replace(wd_anchor, ct, 1)
        ok.append(("C 联系方式卡片", True))
    else:
        ok.append(("C 联系方式卡片", False))
else:
    ok.append(("ui", "已打过"))

print("=== 补丁清单 ===")
for name, st in ok:
    print("  %-30s %s" % (name, "✓" if st is True else st))
failed = [n for n, st in ok if st is False]
if APPLY:
    if failed:
        print("!! 有未命中, 放弃写入")
        raise SystemExit(1)
    for p in (CS, BS, US):
        shutil.copy2(p, p + ".bak-contact-" + time.strftime("%Y%m%d%H%M%S"))
    open(CS, "w", encoding="utf-8").write(s)
    open(BS, "w", encoding="utf-8").write(b)
    open(US, "w", encoding="utf-8").write(u)
    print("已写入 (均已备份)")
else:
    print("(dry-run: 加 --apply 执行)")
