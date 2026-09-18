#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""patch_billing_admin_ui.py —— 给 studio-billing 追加后台管理界面 (纯追加, 幂等)

新增:
  GET  /api/admin/users          全部用户+统一余额+身份映射
  GET  /api/admin/ledger         最近流水
  POST /api/admin/points/adjust  手动调整积分 (正负皆可, 写 ledger + admin_audit)
  GET  /admin                    管理页面 (内嵌 HTML)
"""
SRC = "/opt/studio-billing/app.py"

src = open(SRC, encoding="utf-8").read()
if "admin_users_list" in src:
    print("已打过补丁, 跳过")
    raise SystemExit(0)

HTML = """<!doctype html><html lang="zh"><head><meta charset="utf-8">
<title>lain42 积分后台</title>
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>
body{font-family:system-ui,sans-serif;max-width:960px;margin:24px auto;padding:0 16px;background:#111;color:#eee}
h1{font-size:20px}table{border-collapse:collapse;width:100%;font-size:14px}
td,th{border:1px solid #333;padding:6px 8px;text-align:left}th{background:#1c1c1c}
input,button{padding:6px 8px;background:#222;color:#eee;border:1px solid #444;border-radius:4px}
button{cursor:pointer}button:hover{background:#333}.ok{color:#7c7}.err{color:#c66}
.row{display:flex;gap:8px;flex-wrap:wrap;margin:8px 0}#msg{white-space:pre-wrap}
</style></head><body>
<h1>lain42 积分后台 <small style="font-size:12px;color:#888">(10 积分 = 1 USDT)</small></h1>
<div class="row">管理密钥: <input id="tok" type="password" style="width:280px" placeholder="BILLING_ADMIN_TOKEN">
<button onclick="save()">保存</button> <button onclick="load()">刷新</button></div>
<div id="msg"></div>
<h3>用户与余额</h3><table id="ut"><thead><tr><th>uid</th><th>用户名</th><th>email</th><th>余额</th><th>身份映射</th><th>操作</th></tr></thead><tbody></tbody></table>
<h3>调整积分</h3>
<div class="row">用户名: <input id="u" style="width:140px">
增减: <input id="d" type="number" style="width:100px" value="10"> (可为负)
事由: <input id="r" style="width:260px" value="管理员调整">
<button onclick="adjust()">执行</button></div>
<h3>最近流水</h3><table id="lt"><thead><tr><th>时间</th><th>用户</th><th>增减</th><th>余额</th><th>事由</th></tr></thead><tbody></tbody></table>
<script>
const T=()=>localStorage.getItem("admintok")||"";
function save(){localStorage.setItem("admintok",document.getElementById("tok").value);load()}
async function api(p,opt){const r=await fetch(p,Object.assign({},opt||{},{headers:Object.assign({"X-Admin-Token":T()},(opt||{}).headers||{})}));
if(!r.ok){throw new Error((await r.text()).slice(0,200))}return r.json()}
async function load(){
 try{const d=await api("/api/admin/users");
 const tb=document.querySelector("#ut tbody");tb.innerHTML="";
 for(const u of d.items){
   const links=(d.links||[]).filter(l=>l.uid===u.id).map(l=>l.service+":"+l.external_username).join(", ");
   const tr=document.createElement("tr");
   tr.innerHTML="<td>"+u.id+"</td><td>"+u.username+"</td><td>"+(u.email||"")+"</td><td><b>"+u.balance+"</b></td><td style='color:#888'>"+links+"</td>"+
     "<td><button onclick=\\"quick('"+u.username+"',10)\\">+10</button> <button onclick=\\"quick('"+u.username+"',-10)\\">-10</button></td>";
   tb.appendChild(tr);}
 const l=await api("/api/admin/ledger?n=12");
 const lt=document.querySelector("#lt tbody");lt.innerHTML="";
 for(const e of l.items){const tr=document.createElement("tr");
   tr.innerHTML="<td>"+e.created_at+"</td><td>"+(e.username||"?")+"</td><td>"+e.delta+"</td><td>"+e.balance_after+"</td><td>"+e.reason+"</td>";
   lt.appendChild(tr);}
 msg("加载 OK","ok");}catch(e){msg("失败: "+e.message,"err")}}
async function quick(u,d){document.getElementById("u").value=u;document.getElementById("d").value=d;adjust()}
async function adjust(){
 try{const r=await api("/api/admin/points/adjust",{method:"POST",headers:{"Content-Type":"application/json"},
   body:JSON.stringify({username:u.value,delta:parseInt(d.value),reason:r.value||"管理员调整"})});
 msg("已调整: "+u.value+" "+(r.delta>0?"+":"")+r.delta+" -> 余额 "+r.balance_after,"ok");load()}catch(e){msg("失败: "+e.message,"err")}}
function msg(t,c){const m=document.getElementById("msg");m.textContent=t;m.className=c||""}
if(T())document.getElementById("tok").value=T();load();
</script></body></html>
"""

CODE = '''

# ===== lyco: 后台管理界面 (统一身份+积分) =====
from fastapi.responses import HTMLResponse as _HTMLResp

_ADMIN_UI_HTML = """__ADMIN_UI__"""


@app.get("/api/admin/users")
async def admin_users_list(
    x_admin_token: str | None = Header(None),
    authorization: str | None = Header(None),
):
    """全部用户 + 统一余额 + 身份映射"""
    actor = require_admin(x_admin_token, authorization)
    conn = db()
    items = conn.execute(
        """SELECT u.id, u.username, u.email, u.founder,
                  COALESCE(p.balance, 0) AS balance
           FROM users u LEFT JOIN points_accounts p ON p.user_id = u.id
           ORDER BY u.id""").fetchall()
    try:
        links = conn.execute(
            "SELECT uid, service, external_id, external_username FROM identity_links").fetchall()
    except Exception:
        links = []
    return {"actor": actor["actor"],
            "items": [dict(r) for r in items],
            "links": [dict(l) for l in links]}


@app.get("/api/admin/ledger")
async def admin_ledger_tail(
    n: int = 12,
    x_admin_token: str | None = Header(None),
    authorization: str | None = Header(None),
):
    """最近积分流水 (带用户名)"""
    actor = require_admin(x_admin_token, authorization)
    conn = db()
    rows = conn.execute(
        """SELECT l.created_at, u.username, l.delta, l.balance_after, l.reason
           FROM points_ledger l LEFT JOIN users u ON u.id = l.user_id
           ORDER BY l.id DESC LIMIT ?""", (max(1, min(n, 100)),)).fetchall()
    return {"items": [dict(r) for r in rows]}


@app.post("/api/admin/points/adjust")
async def admin_points_adjust(
    req: Request,
    x_admin_token: str | None = Header(None),
    authorization: str | None = Header(None),
):
    """手动调整积分: delta 可正可负; 写 ledger + admin_audit"""
    actor = require_admin(x_admin_token, authorization)
    body = await req.json()
    if not isinstance(body, dict):
        raise HTTPException(400, "请求体必须是 JSON 对象")
    username = str(body.get("username", "")).strip()
    try:
        delta = int(body.get("delta"))
    except Exception:
        raise HTTPException(400, "delta 必须是整数")
    if delta == 0:
        raise HTTPException(400, "delta 不能为 0")
    reason = str(body.get("reason") or "管理员调整").strip()[:200]
    conn = db()
    row = conn.execute(
        "SELECT id FROM users WHERE username=? OR email=?", (username, username)).fetchone()
    if not row:
        raise HTTPException(404, "user not found")
    uid = int(row["id"])
    import time as _t
    conn.execute("BEGIN IMMEDIATE")
    bal = conn.execute("SELECT balance FROM points_accounts WHERE user_id=?", (uid,)).fetchone()
    newbal = (bal[0] if bal else 0) + delta
    if bal:
        conn.execute("UPDATE points_accounts SET balance=?, updated_at=? WHERE user_id=?",
                     (newbal, int(_t.time()), uid))
    else:
        conn.execute("INSERT INTO points_accounts(user_id,balance,updated_at) VALUES(?,?,?)",
                     (uid, newbal, int(_t.time())))
    conn.execute(
        "INSERT INTO points_ledger(user_id,delta,balance_after,reason,reference,idem_key,created_at,meta)"
        " VALUES(?,?,?,?,?,?,?,?)",
        (uid, delta, newbal, "admin:" + reason, username,
         "admin-%s-%d" % (actor["actor"], _t.time_ns()), int(_t.time()),
         '{"actor":"%s"}' % actor["actor"]))
    try:
        conn.execute(
            "INSERT INTO admin_audit(actor,action,target,reason,metadata,created_at)"
            " VALUES(?,?,?,?,?,?)",
            (actor["actor"], "points.adjust", username,
             "%+d (%s)" % (delta, reason), "", int(_t.time())))
    except Exception:
        pass
    conn.commit()
    return {"ok": True, "username": username, "delta": delta, "balance_after": newbal}


@app.get("/admin", response_class=_HTMLResp)
async def admin_ui():
    return _ADMIN_UI_HTML
# ===== lyco: 管理界面结束 =====
'''.replace("__ADMIN_UI__", HTML)

open(SRC, "w", encoding="utf-8").write(src + "\n" + CODE)
print("已追加管理界面代码到", SRC)
