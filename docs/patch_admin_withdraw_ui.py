#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""patch_admin_withdraw_ui.py —— 管理页加提现审批区块 (幂等)"""
SRC = "/opt/studio-billing/app.py"
s = open(SRC, encoding="utf-8").read()
if "widTb" in s:
    print("已打过, 跳过")
    raise SystemExit(0)

# 1) 页面加提现区块 (在"最近流水"前插一个表)
old1 = "<h3>最近流水</h3>"
new1 = ('<h3>提现申请</h3><table id="wt"><thead><tr><th>时间</th><th>用户</th><th>积分</th>'
        '<th>USDT</th><th>地址</th><th>状态</th><th>操作</th></tr></thead><tbody></tbody></table>'
        '<h3>最近流水</h3>')
assert old1 in s, "anchor1 miss"
s = s.replace(old1, new1, 1)

# 2) load() 里拉提现单并渲染
old2 = ' msg("加载 OK","ok");'
new2 = ''' const w=await api("admin/withdrawals");
 const wt=document.querySelector("#wt tbody");wt.innerHTML="";
 for(const x of w.items){const tr=document.createElement("tr");
   tr.innerHTML="<td>"+new Date(x.created_at*1000).toLocaleString("zh-CN")+"</td><td>"+(x.username||x.uid)+"</td><td>"+x.credits+"</td><td>"+x.usdt+"</td><td style='font-size:11px'>"+x.address+"</td><td>"+x.status+"</td>"+
   (x.status==="pending"?"<td><button onclick=\\"wact("+x.id+",'paid')\\">已打款</button> <button onclick=\\"wact("+x.id+",'reject')\\">拒绝</button></td>":"<td>"+(x.tx_hash||"")+"</td>");
   wt.appendChild(tr);}
''' + old2
assert old2 in s, "anchor2 miss"
s = s.replace(old2, new2, 1)

# 3) wact 函数
old3 = "async function adjust(){"
new3 = '''async function wact(id,act){
 try{const tx=act==="paid"?prompt("USDT tx_hash(可空):",""):"";
 const r=await api("admin/withdrawals/"+id,{method:"POST",headers:{"Content-Type":"application/json"},
   body:JSON.stringify({action:act,tx_hash:tx||""})});
 msg("提现"+(act==="paid"?"已标记打款":"已拒绝并退分"),"ok");load()}catch(e){msg("失败: "+e.message,"err")}}
async function adjust(){'''
assert old3 in s, "anchor3 miss"
s = s.replace(old3, new3, 1)

open(SRC, "w", encoding="utf-8").write(s)
print("管理页提现审批区块 ✓ (3 处)")
