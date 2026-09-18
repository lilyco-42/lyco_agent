#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""patch_owner_ui.py —— 设备主自助管理: 服务公开/下架/改价 + 收益视图 + 提现表单

A) compute/server.py:
   - services 表加 enabled 列; 市场列表只显示 enabled=1
   - 新路由: POST /api/v1/services/{sid}/update (own), GET /api/v1/my/earnings,
            POST /api/v1/withdraw (本地扣分 + billing 提现单 + 统一账本冻结)
   - my_devices 返回 services 带 enabled
B) billing app.py: 提现拒绝时同步退回 compute 本地积分
C) compute ui.html: 设备卡服务行加[上架/下架][改价]; 积分 tab 加提现表单
幂等; 自动备份。
"""
import shutil
import time

APPLY = "--apply" in __import__("sys").argv
ok = []


def guard(name, cond):
    ok.append((name, cond))
    return cond


# ================= A) compute/server.py =================
CS = "/opt/compute/server.py"
s = open(CS, encoding="utf-8").read()

# A1) enabled 列
if "ADD COLUMN enabled" not in s:
    anchor = 'db.execute("""CREATE TABLE IF NOT EXISTS services('
    if anchor in s:
        # 在 init 区后追加 ALTER (放 main 前不可靠, 直接在 init 附近加 try)
        s = s.replace(anchor,
                      'import sqlite3 as _sq3\ntry:\n    db.execute("ALTER TABLE services ADD COLUMN enabled INTEGER DEFAULT 1")\nexcept Exception:\n    pass\n\n' + anchor, 1)
        ok.append(("A1 enabled 列(代码内 ensure)", True))

# A2) 市场列表过滤下架服务
mkt_old = 'FROM services s JOIN devices d ON d.id=s.device_id ORDER BY s.created_at DESC'
mkt_new = 'FROM services s JOIN devices d ON d.id=s.device_id WHERE COALESCE(s.enabled,1)=1 ORDER BY s.created_at DESC'
if mkt_old in s:
    s = s.replace(mkt_old, mkt_new, 1)
    ok.append(("A2 市场过滤下架", True))
elif "COALESCE(s.enabled,1)=1" in s:
    ok.append(("A2 市场过滤下架", "已打过"))
else:
    ok.append(("A2 市场过滤下架", False))

# A3) my_devices 带 enabled
old_svc = '"services": [service_dict(s) for s in svc]'
new_svc = '"services": [dict(zip(s.keys(), tuple(s))) for s in svc]'
if old_svc in s:
    s = s.replace(old_svc, new_svc, 1)
    ok.append(("A3 my_devices 带 enabled", True))

# A4) 新 handler 方法 (插在 _owner_check 前)
if "def update_service" not in s:
    handlers = '''    def update_service(self, sid):
        u = self._auth()
        row = db.execute("SELECT s.id, d.owner_id FROM services s JOIN devices d ON d.id=s.device_id WHERE s.id=?",
                         (sid,)).fetchone()
        if not row or row[1] != u[0]:
            raise ValueError("无权修改该服务")
        body = json.loads(self._body(65536))
        fields, vals = [], []
        if "price" in body:
            price = int(body["price"])
            if price < 0:
                raise ValueError("价格不能为负")
            fields.append("price=?"); vals.append(price)
        if "name" in body and str(body["name"]).strip():
            fields.append("name=?"); vals.append(str(body["name"]).strip()[:60])
        if "desc" in body:
            fields.append("desc=?"); vals.append(str(body["desc"])[:200])
        if "enabled" in body:
            fields.append("enabled=?"); vals.append(1 if body["enabled"] else 0)
        if not fields:
            raise ValueError("没有可更新字段")
        vals.append(sid)
        with db_lock:
            db.execute("UPDATE services SET %s WHERE id=?" % ",".join(fields), vals)
            db.commit()
        return self._json({"ok": True})

    def my_earnings(self):
        u = self._auth()
        total = db.execute("SELECT COALESCE(SUM(amount),0) FROM credit_log WHERE user_id=? AND reason LIKE '服务收益%'",
                           (u[0],)).fetchone()[0]
        recent = db.execute(
            "SELECT amount, reason, created_at FROM credit_log WHERE user_id=? AND reason LIKE '服务收益%'"
            " ORDER BY created_at DESC LIMIT 20", (u[0],)).fetchall()
        return self._json({"total": total,
                           "recent": [{"amount": r[0], "reason": r[1], "created_at": r[2]} for r in recent]})

    def user_withdraw(self):
        u = self._auth()
        body = json.loads(self._body(65536))
        credits = int(body.get("credits", 0))
        address = str(body.get("address", "")).strip()
        if credits < 100:
            raise ValueError("最低提现 100 积分 (= 10 USDT)")
        if not address.startswith("T") or len(address) < 25:
            raise ValueError("请填写有效的 TRC20 收款地址")
        bal = db.execute("SELECT credits FROM users WHERE id=?", (u[0],)).fetchone()[0]
        if bal < credits:
            raise ValueError("积分不足 (当前 %d)" % bal)
        with db_lock:
            db.execute("UPDATE users SET credits=credits-? WHERE id=?", (credits, u[0]))
            db.execute("INSERT INTO credit_log(id,user_id,amount,reason,created_at) VALUES(?,?,?,?,?)",
                       (newid("cl"), u[0], -credits, "提现冻结 " + address[:16], now()))
            db.commit()
        usdt = round(credits / 10.0, 2)
        newbal = id_ledger.mirror("compute", u[0], -credits, "withdraw:freeze",
                                  "wd-" + str(_tmod.time_ns()))
        bc = sqlite3.connect("/opt/studio-billing/billing.db", timeout=15)
        urow = bc.execute("SELECT uid FROM identity_links WHERE service='compute' AND external_id=?",
                          (u[0],)).fetchone()
        bc.execute("INSERT INTO withdrawals(uid,credits,usdt,address,status,created_at) VALUES(?,?,?,?,?,?)",
                   (urow[0] if urow else None, credits, usdt, address, "pending", now()))
        bc.commit(); bc.close()
        return self._json({"ok": True, "usdt": usdt, "balance_after": newbal,
                           "note": "管理员 USDT 打款后到账"})

'''
    anchor = "    def _owner_check(self, device_id):"
    if anchor in s:
        s = s.replace(anchor, handlers + anchor, 1)
        ok.append(("A4 handler 方法", True))
    else:
        ok.append(("A4 handler 方法", False))

# A5) 路由分发
if 'api[:2] == ["v1", "services"]' not in s:
    a5 = '''            if api[:2] == ["v1", "services"] and len(api) == 4 and api[3] == "update" and method == "POST":
                return self.update_service(api[2])
            if api[:2] == ["v1", "withdraw"] and method == "POST":
                return self.user_withdraw()
'''
    anchor = '            if api[:2] == ["v1", "devices"] and len(api) == 4 and api[3] == "services" and method == "POST":'
    if anchor in s:
        s = s.replace(anchor, a5 + anchor, 1)
        ok.append(("A5 路由", True))
    else:
        ok.append(("A5 路由", False))
else:
    ok.append(("A5 路由", "已打过"))

# A6) GET /api/v1/my/earnings 路由
if 'api[:2] == ["v1", "my"]' not in s:
    a6 = '''            if api[:2] == ["v1", "my"] and len(api) == 3 and api[2] == "earnings" and method == "GET":
                return self.my_earnings()
'''
    anchor = '            if api[:2] == ["v1", "devices"] and len(api) == 4 and api[3] == "services" and method == "GET":'
    if anchor in s:
        s = s.replace(anchor, a6 + anchor, 1)
        ok.append(("A6 earnings 路由", True))
    else:
        ok.append(("A6 earnings 路由", False))
else:
    ok.append(("A6 earnings 路由", "已打过"))

# ================= B) billing reject 退回 compute 本地积分 =================
BS = "/opt/studio-billing/app.py"
b = open(BS, encoding="utf-8").read()
if "withdraw.reject 同步退回 compute" not in b:
    b_anchor = "             \"wid-reject-%d\" % wid, int(_t.time()), '{\"actor\":\"%s\"}' % actor[\"actor\"]))"
    if b_anchor in b:
        add = b_anchor + '''
        # withdraw.reject 同步退回 compute 本地积分
        try:
            _urow = conn.execute("SELECT external_id FROM identity_links WHERE uid=? AND service='compute'",
                                 (row["uid"],)).fetchone()
            if _urow:
                import sqlite3 as _sq3
                _cc = _sq3.connect("/var/lib/compute/compute.db", timeout=15)
                _cc.execute("UPDATE users SET credits=credits+? WHERE id=?", (delta, _urow[0]))
                _cc.commit(); _cc.close()
        except Exception:
            pass'''
        b = b.replace(b_anchor, add, 1)
        ok.append(("B reject 退本地积分", True))
    else:
        ok.append(("B reject 退本地积分", False))
else:
    ok.append(("B reject 退本地积分", "已打过"))

# ================= C) ui.html =================
US = "/opt/compute/ui.html"
u = open(US, encoding="utf-8").read()

c1_old = """      (dev.services||[]).forEach(function(s){html+='<span class="tag">'+esc(s.name)+' ('+s.port+') '+s.price+'积分</span>';});
      html+='</div><div class="row" style="margin-top:10px"><button data-svc="'+dev.id+'" type="button">添加服务</button></div></div>';"""
c1_new = """      (dev.services||[]).forEach(function(s){
        html+='<span class="tag">'+esc(s.name)+' ('+s.port+') '+s.price+'积分 '+(s.enabled===0?'<b style="color:var(--red)">[已下架]</b>':'')+'</span>';
        html+='<button data-stoggle="'+s.id+'" data-en="'+(s.enabled===0?1:0)+'">'+(s.enabled===0?'上架':'下架')+'</button>';
        html+='<button data-sprice="'+s.id+'" data-cur="'+s.price+'">改价</button>';
      });
      var earnEl=document.getElementById("earnBox");
      html+='</div><div class="row" style="margin-top:10px"><button data-svc="'+dev.id+'" type="button">添加服务</button></div></div>';"""
if c1_old in u:
    u = u.replace(c1_old, c1_new, 1)
    ok.append(("C1 设备卡服务行", True))
else:
    ok.append(("C1 设备卡服务行", False))

# C2) toggle/改价事件绑定 (挂在 loadDevices 的绑定区后)
bind_old = "el.querySelectorAll(\"[data-svc]\").forEach(function(b){b.addEventListener(\"click\",function(){openSvc(b.dataset.svc);});});"
bind_new = bind_old + """
    el.querySelectorAll("[data-stoggle]").forEach(function(b){b.addEventListener("click",function(){
      api("/api/v1/services/"+b.dataset.stoggle+"/update",{method:"POST",body:JSON.stringify({enabled:b.dataset.en==="1"})})
      .then(loadDevices).catch(function(e){alert("失败: "+e.message)});});});
    el.querySelectorAll("[data-sprice]").forEach(function(b){b.addEventListener("click",function(){
      var np=prompt("新价格(积分/次):", b.dataset.cur); if(np===null)return;
      api("/api/v1/services/"+b.dataset.sprice+"/update",{method:"POST",body:JSON.stringify({price:parseInt(np)})})
      .then(loadDevices).catch(function(e){alert("失败: "+e.message)});});});"""
if 'el.querySelectorAll("[data-stoggle]")' not in u and bind_old in u:
    u = u.replace(bind_old, bind_new, 1)
    ok.append(("C2 toggle/改价绑定", True))
else:
    ok.append(("C2 toggle/改价绑定", "已打过" if 'data-stoggle' in u else False))

# C3) 提现表单 (积分 tab, creditBox 渲染后)
if 'id="wdGo"' not in u:
    wd_old = "    document.getElementById(\"creditBox\").innerHTML=html;"
    wd_new = wd_new = """    document.getElementById("creditBox").innerHTML=html;
    var _wd=document.createElement("div");
    _wd.innerHTML='<div class="card"><h3>提现 (10 积分 = 1 USDT)</h3><div class="row">积分: <input id="wdAmt" type="number" value="100" style="width:110px"> TRC20 地址: <input id="wdAddr" style="width:280px" placeholder="T 开头的收款地址"> <button class="primary" id="wdGo">申请提现</button></div><div class="meta">最低 100 积分。提交后积分立即冻结，管理员 USDT 打款后完成。</div></div>';
    document.getElementById("creditBox").appendChild(_wd);
    document.getElementById("wdGo").addEventListener("click",function(){
      api("/api/v1/withdraw",{method:"POST",body:JSON.stringify({credits:parseInt(document.getElementById("wdAmt").value||"0"),address:document.getElementById("wdAddr").value})})
      .then(function(d){alert("提现申请已提交: "+d.usdt+" USDT\\n积分已冻结, 管理员打款后完成");loadCredits&&loadCredits();})
      .catch(function(e){alert("失败: "+e.message)});});"""
    if wd_old in u:
        u = u.replace(wd_old, wd_new, 1)
        ok.append(("C3 提现表单", True))
    else:
        ok.append(("C3 提现表单", False))
else:
    ok.append(("C3 提现表单", "已打过"))

print("=== 补丁清单 ===")
for name, st in ok:
    print("  %-28s %s" % (name, "✓" if st is True else st))
failed = [n for n, st in ok if st is False]
if APPLY:
    if failed:
        print("!! 有未命中的补丁点, 放弃写入 (原文件未动)")
        raise SystemExit(1)
    for p in (CS, BS, US):
        shutil.copy2(p, p + ".bak-ownerui-" + time.strftime("%Y%m%d%H%M%S"))
    open(CS, "w", encoding="utf-8").write(s)
    open(BS, "w", encoding="utf-8").write(b)
    open(US, "w", encoding="utf-8").write(u)
    print("已写入 (均已备份)")
else:
    print("(dry-run: 加 --apply 执行)")
