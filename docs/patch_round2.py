#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""patch_round2.py —— 商业化第三轮: 可靠度评分 + USDT 充值接进 compute UI

A) compute/server.py:
   A1 修 ALTER services 后的索引回归 (r[10]/r[11] -> 键访问), 市场加完成率统计
   A2 新增 POST /api/v1/usdt/order (compute 用户直接建 USDT 充值单, 代理写 billing)
B) compute/ui.html:
   B1 市场卡显示完成率
   B2 renderRecharge 换成 USDT 充值流 (替代模拟支付)
幂等; 自动备份。
"""
import shutil
import time

APPLY = "--apply" in __import__("sys").argv
ok = []

# ===== A) compute/server.py =====
CS = "/opt/compute/server.py"
s = open(CS, encoding="utf-8").read()

old_svc = '''    def services(self):
        online = online_device_ids()
        rows = db.execute("""SELECT s.*, d.name AS dname, d.hardware FROM services s JOIN devices d ON d.id=s.device_id WHERE COALESCE(s.enabled,1)=1 ORDER BY s.created_at DESC""").fetchall()
        items = []
        for r in rows:
            d = service_dict(r, online)
            d["deviceName"] = r[10]
            d["hardware"] = r[11]
            items.append(d)
        return self._json({"items": items})'''
new_svc = '''    def services(self):
        online = online_device_ids()
        rows = db.execute("""SELECT s.*, d.name AS dname, d.hardware,
            (SELECT COUNT(*) FROM tasks t WHERE t.service_id=s.id AND t.status='success') AS done_cnt,
            (SELECT COUNT(*) FROM tasks t WHERE t.service_id=s.id AND t.status='failed') AS fail_cnt
            FROM services s JOIN devices d ON d.id=s.device_id
            WHERE COALESCE(s.enabled,1)=1 ORDER BY s.created_at DESC""").fetchall()
        items = []
        for r in rows:
            d = service_dict(r, online)
            d["deviceName"] = r["dname"]
            d["hardware"] = r["hardware"]
            done, fail = r["done_cnt"], r["fail_cnt"]
            tot = done + fail
            d["done"] = done
            d["fail"] = fail
            d["rate"] = round(done * 100.0 / tot, 1) if tot else None
            items.append(d)
        return self._json({"items": items})'''
if 'r["done_cnt"]' in s:
    ok.append(("A1 市场+完成率(修索引回归)", "已打过"))
elif old_svc in s:
    s = s.replace(old_svc, new_svc, 1)
    ok.append(("A1 市场+完成率(修索引回归)", True))
else:
    ok.append(("A1 市场+完成率(修索引回归)", False))

# A2) usdt_order 代理
if '"v1", "usdt"' not in s:
    handler = '''    def usdt_order(self):
        u = self._auth()
        body = json.loads(self._body(65536))
        try:
            amount = round(float(body.get("amount_usdt", 0)), 3)
        except Exception:
            raise ValueError("amount_usdt 必须是数字")
        if amount < 1:
            raise ValueError("最低充值 1 USDT")
        credits = int(amount * 10)
        import secrets as _sec
        ono = "U" + _sec.token_hex(6).upper()
        bc = sqlite3.connect("/opt/studio-billing/billing.db", timeout=15)
        urow = bc.execute("SELECT uid FROM identity_links WHERE service='compute' AND external_id=?",
                          (u[0],)).fetchone()
        if not urow:
            bc.close()
            raise ValueError("账号未关联统一身份, 无法充值")
        bc.execute("INSERT INTO usdt_orders(order_no,uid,amount_usdt,credits,purpose,status,created_at)"
                   " VALUES(?,?,?,?,?,?,?)",
                   (ono, urow[0], amount, credits, "topup", "pending", now()))
        bc.commit(); bc.close()
        return self._json({"ok": True, "order_no": ono, "amount_usdt": amount, "credits": credits,
                           "wallet": "TKL9TuXXHu9oubcH36SndwKXwp3W1fxzUJ",
                           "note": "转账正好 %g USDT, 链上确认后自动到账" % amount})

'''
    anchor = "    def _owner_check(self, device_id):"
    if anchor in s:
        s = s.replace(anchor, handler + anchor, 1)
        ok.append(("A2 usdt_order 代理", True))
    else:
        ok.append(("A2 usdt_order 代理", False))

    a3 = '''            if api[:2] == ["v1", "usdt"] and len(api) == 3 and api[2] == "order" and method == "POST":
                return self.usdt_order()
'''
    anchor3 = '            if api[:2] == ["v1", "withdraw"] and method == "POST":'
    if anchor3 in s:
        s = s.replace(anchor3, a3 + anchor3, 1)
        ok.append(("A3 usdt 路由", True))
    else:
        ok.append(("A3 usdt 路由", False))
else:
    ok.append(("A2/A3 usdt", "已打过"))

# ===== B) ui.html =====
US = "/opt/compute/ui.html"
u = open(US, encoding="utf-8").read()

b1_old = '''<span class="tag">'+esc(s.type)+'</span><span class="tag">'+esc(s.hardware||"未知硬件")+'</span>'''
b1_new = '''<span class="tag">'+esc(s.type)+'</span><span class="tag">'+esc(s.hardware||"未知硬件")+'</span>'+(s.rate!=null?'<span class="tag" style="color:'+(s.rate>=90?"#7c7":(s.rate>=60?"#fb6":"#c66"))+'">完成率 '+s.rate+'% ('+s.done+'/'+(s.done+s.fail)+')</span>':'')'''
if 'data-stoggle' not in u and b1_old in u:
    pass
if b1_old in u and "完成率" not in u:
    u = u.replace(b1_old, b1_new, 1)
    ok.append(("B1 市场完成率展示", True))
elif "完成率" in u:
    ok.append(("B1 市场完成率展示", "已打过"))
else:
    ok.append(("B1 市场完成率展示", False))

r_old = """  el.innerHTML='<div class="card"><h3>充值积分</h3><div class="meta">1 元 = 10 积分（模拟支付测试）</div><div class="row" style="margin-bottom:12px"><button data-cr="100">100 积分 (¥10)</button><button data-cr="500">500 积分 (¥50)</button><button data-cr="1000">1000 积分 (¥100)</button></div><div class="form-item full"><label>自定义积分</label><input id="rcCustom" type="number" min="10" placeholder="10-1000000"></div><div class="row" style="margin-top:10px"><button class="primary" id="rcGo" type="button">生成订单</button></div><div class="msg" id="rcMsg"></div></div>';
  el.querySelectorAll("[data-cr]").forEach(function(b){b.addEventListener("click",function(){createOrder(Number(b.dataset.cr));});});
  document.getElementById("rcGo").addEventListener("click",function(){var v=Number(document.getElementById("rcCustom").value);if(v>=10)createOrder(v);});"""
r_new = """  el.innerHTML='<div class="card"><h3>USDT 充值 (TRC20)</h3><div class="meta">10 积分 = 1 USDT。生成订单后向收款地址转账对应金额, 链上确认后自动到账 (约 1-2 分钟)。</div><div class="form-item full"><label>USDT 金额 (最低 1)</label><input id="rcUsdt" type="number" min="1" step="0.01" value="5"></div><div class="row" style="margin-top:10px"><button class="primary" id="rcGo" type="button">生成充值订单</button></div><div class="msg" id="rcMsg"></div><div id="rcOrder" style="margin-top:8px"></div></div>';
  document.getElementById("rcGo").addEventListener("click",function(){
    var v=Number(document.getElementById("rcUsdt").value);
    if(!(v>=1)){document.getElementById("rcMsg").className="msg err";document.getElementById("rcMsg").textContent="最低 1 USDT";return;}
    api("/api/v1/usdt/order",{method:"POST",body:JSON.stringify({amount_usdt:v})}).then(function(o){
      document.getElementById("rcMsg").className="msg ok";
      document.getElementById("rcMsg").textContent="订单 "+o.order_no+" 已生成: 到账 "+o.credits+" 积分";
      document.getElementById("rcOrder").innerHTML='<div class="code">收款地址(TRC20): '+o.wallet+'<br>转账金额: 正好 '+o.amount_usdt+' USDT<br>订单号: '+o.order_no+'</div>';
    }).catch(function(e){document.getElementById("rcMsg").className="msg err";document.getElementById("rcMsg").textContent=e.message;});
  });"""
if "USDT 充值 (TRC20)" in u:
    ok.append(("B2 USDT 充值表单", "已打过"))
elif r_old in u:
    u = u.replace(r_old, r_new, 1)
    ok.append(("B2 USDT 充值表单", True))
else:
    ok.append(("B2 USDT 充值表单", False))

print("=== 补丁清单 ===")
for name, st in ok:
    print("  %-32s %s" % (name, "✓" if st is True else st))
failed = [n for n, st in ok if st is False]
if APPLY:
    if failed:
        print("!! 有未命中, 放弃写入")
        raise SystemExit(1)
    for p in (CS, US):
        shutil.copy2(p, p + ".bak-r2-" + time.strftime("%Y%m%d%H%M%S"))
    open(CS, "w", encoding="utf-8").write(s)
    open(US, "w", encoding="utf-8").write(u)
    print("已写入 (均已备份)")
else:
    print("(dry-run: 加 --apply 执行)")
