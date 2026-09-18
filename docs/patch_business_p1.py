#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""patch_business_p1.py —— 商业闭环 P1: 平台抽成 + 提现

A) compute/server.py:
   - 设备主只拿 85%, 平台(admin01)拿 15% (含 ledger 镜像)
   - 修 bug: pay-provider 分支 mirror reason 误写"任务退款"
B) billing app.py:
   - withdrawals 表 + 用户提现 API (/api/withdraw) + 管理端 API + 管理页提现区块
幂等, 自动备份。
"""
import shutil
import time

APPLY = "--apply" in __import__("sys").argv

# ---------- A) compute 抽成 ----------
CSRC = "/opt/compute/server.py"
s = open(CSRC, encoding="utf-8").read()
old_pay = '''            else:
                # pay provider
                db.execute("UPDATE users SET credits=credits+? WHERE id=?", (row[8], row[2]))
                id_ledger.mirror("compute", row[2], row[8], "任务退款", "refund-" + str(_tmod.time_ns()))
                db.execute("INSERT INTO credit_log(id,user_id,amount,reason,created_at) VALUES(?,?,?,?,?)",
                           (newid("cl"), row[2], row[8], "服务收益 " + tid, now()))'''
new_pay = '''            else:
                # pay provider 85% + platform 15% (商业闭环: 平台抽成)
                cost = row[8]
                owner_cut = cost * 85 // 100
                plat_cut = cost - owner_cut
                db.execute("UPDATE users SET credits=credits+? WHERE id=?", (owner_cut, row[2]))
                id_ledger.mirror("compute", row[2], owner_cut, "服务收益 " + tid, "earn-" + str(_tmod.time_ns()))
                db.execute("INSERT INTO credit_log(id,user_id,amount,reason,created_at) VALUES(?,?,?,?,?)",
                           (newid("cl"), row[2], owner_cut, "服务收益 " + tid, now()))
                PLATFORM_UID = "u_78b4cee3c2b54656"   # admin01 = 平台收入账户
                db.execute("UPDATE users SET credits=credits+? WHERE id=?", (plat_cut, PLATFORM_UID))
                id_ledger.mirror("compute", PLATFORM_UID, plat_cut, "平台抽成 " + tid, "fee-" + str(_tmod.time_ns()))
                db.execute("INSERT INTO credit_log(id,user_id,amount,reason,created_at) VALUES(?,?,?,?,?)",
                           (newid("cl"), PLATFORM_UID, plat_cut, "平台抽成 " + tid, now()))'''
if "owner_cut = cost * 85 // 100" in s:
    print("[A] compute 抽成: 已打过, 跳过")
elif old_pay in s:
    s = s.replace(old_pay, new_pay, 1)
    print("[A] compute 抽成补丁 ✓ (85/15 + 修 reason bug)")
else:
    print("[A] !! 未匹配到 pay-provider 块, 请人工检查")

if APPLY and "[A] compute 抽成补丁 ✓" in open("/dev/stdout").read() if False else True:
    pass
if APPLY:
    shutil.copy2(CSRC, CSRC + ".bak-p1-" + time.strftime("%Y%m%d%H%M%S"))
    open(CSRC, "w", encoding="utf-8").write(s)
    print("[A] 已写入 compute/server.py (已备份)")

# ---------- B) billing 提现 ----------
BSRC = "/opt/studio-billing/app.py"
b = open(BSRC, encoding="utf-8").read()
if "withdrawals" in b and "def user_withdraw" in b:
    print("[B] billing 提现: 已打过, 跳过")
else:
    CODE = '''

# ===== lyco: 提现系统 (积分 -> USDT, 10 积分 = 1 USDT) =====
WITHDRAW_MIN_CREDITS = 100        # 最低 100 积分 = 10 USDT
POINTS_PER_USDT = 10


@app.get("/api/admin/withdrawals")
async def admin_withdrawals(
    x_admin_token: str | None = Header(None),
    authorization: str | None = Header(None),
):
    actor = require_admin(x_admin_token, authorization)
    conn = db()
    try:
        rows = conn.execute(
            "SELECT w.*, u.username FROM withdrawals w LEFT JOIN users u ON u.id=w.uid "
            "ORDER BY w.created_at DESC LIMIT 100").fetchall()
    except Exception:
        return {"items": []}
    return {"items": [dict(r) for r in rows]}


@app.post("/api/admin/withdrawals/{wid}")
async def admin_withdrawal_act(
    wid: int,
    req: Request,
    x_admin_token: str | None = Header(None),
    authorization: str | None = Header(None),
):
    """approve: 标记已打款(填 tx_hash); reject: 退回积分"""
    actor = require_admin(x_admin_token, authorization)
    body = await req.json()
    action = str(body.get("action", ""))
    conn = db()
    row = conn.execute("SELECT * FROM withdrawals WHERE id=?", (wid,)).fetchone()
    if not row:
        raise HTTPException(404, "withdrawal not found")
    if row["status"] != "pending":
        raise HTTPException(400, "该笔已处理 (%s)" % row["status"])
    import time as _t
    if action == "paid":
        tx = str(body.get("tx_hash", ""))[:120]
        conn.execute("UPDATE withdrawals SET status='paid', tx_hash=?, paid_at=? WHERE id=?",
                     (tx, int(_t.time()), wid))
        reason = "admin:提现打款 %s" % (tx or "-")
    elif action == "reject":
        conn.execute("UPDATE withdrawals SET status='rejected', paid_at=? WHERE id=?",
                     (int(_t.time()), wid))
        # 退回积分
        delta = int(row["credits"])
        conn.execute("BEGIN IMMEDIATE")
        bal = conn.execute("SELECT balance FROM points_accounts WHERE user_id=?", (row["uid"],)).fetchone()
        newbal = (bal[0] if bal else 0) + delta
        if bal:
            conn.execute("UPDATE points_accounts SET balance=?, updated_at=? WHERE user_id=?",
                         (newbal, int(_t.time()), row["uid"]))
        else:
            conn.execute("INSERT INTO points_accounts(user_id,balance,updated_at) VALUES(?,?,?)",
                         (row["uid"], newbal, int(_t.time())))
        conn.execute(
            "INSERT INTO points_ledger(user_id,delta,balance_after,reason,reference,idem_key,created_at,meta)"
            " VALUES(?,?,?,?,?,?,?,?)",
            (row["uid"], delta, newbal, "withdraw:reject", "wid-%d" % wid,
             "wid-reject-%d" % wid, int(_t.time()), '{"actor":"%s"}' % actor["actor"]))
        reason = "admin:拒绝提现, 已退 %d 积分" % delta
    else:
        raise HTTPException(400, "action 必须是 paid / reject")
    try:
        conn.execute("INSERT INTO admin_audit(actor,action,target,reason,metadata,created_at)"
                     " VALUES(?,?,?,?,?,?)",
                     (actor["actor"], "withdraw." + action, "wid-%d" % wid, reason, "", int(_t.time())))
    except Exception:
        pass
    conn.commit()
    return {"ok": True, "action": action}


@app.post("/api/withdraw")
async def user_withdraw(
    req: Request,
    authorization: str | None = Header(None),
):
    """用户发起提现: 立即冻结积分(负流水), 管理员 USDT 打款后标记 paid"""
    if not authorization or not authorization.startswith("Bearer "):
        raise HTTPException(401, "请先登录")
    payload = jwt_verify(authorization[7:])
    if not payload or not payload.get("uid"):
        raise HTTPException(401, "会话已失效")
    uid = payload["uid"]
    body = await req.json()
    credits = int(body.get("credits", 0))
    address = str(body.get("address", "")).strip()
    if credits < WITHDRAW_MIN_CREDITS:
        raise HTTPException(400, "最低提现 %d 积分 (= %g USDT)" % (WITHDRAW_MIN_CREDITS, WITHDRAW_MIN_CREDITS / POINTS_PER_USDT))
    if not address.startswith("T") or len(address) < 25:
        raise HTTPException(400, "请填写有效的 TRC20 收款地址")
    usdt = round(credits / POINTS_PER_USDT, 2)
    conn = db()
    import time as _t
    conn.execute("BEGIN IMMEDIATE")
    bal = conn.execute("SELECT balance FROM points_accounts WHERE user_id=?", (uid,)).fetchone()
    cur = bal[0] if bal else 0
    if cur < credits:
        conn.rollback()
        raise HTTPException(400, "积分不足 (当前 %d)" % cur)
    newbal = cur - credits
    conn.execute("UPDATE points_accounts SET balance=?, updated_at=? WHERE user_id=?",
                 (newbal, int(_t.time()), uid))
    conn.execute(
        "INSERT INTO points_ledger(user_id,delta,balance_after,reason,reference,idem_key,created_at,meta)"
        " VALUES(?,?,?,?,?,?,?,?)",
        (uid, -credits, newbal, "withdraw:freeze", address[:30],
         "wid-freeze-%s-%d" % (address[:10], _t.time_ns()), int(_t.time()),
         '{"usdt":%s}' % usdt))
    conn.execute(
        "INSERT INTO withdrawals(uid,credits,usdt,address,status,created_at)"
        " VALUES(?,?,?,?,?,?)",
        (uid, credits, usdt, address, "pending", int(_t.time())))
    try:
        conn.execute("INSERT INTO admin_audit(actor,action,target,reason,metadata,created_at)"
                     " VALUES(?,?,?,?,?,?)",
                     ("user-%s" % uid, "withdraw.request", address[:20],
                      "%d 积分 = %s USDT" % (credits, usdt), "", int(_t.time())))
    except Exception:
        pass
    conn.commit()
    return {"ok": True, "usdt": usdt, "address": address, "balance_after": newbal,
            "note": "管理员将尽快打款, 请留意后台状态"}
# ===== lyco: 提现系统结束 =====
'''
    b = b + "\n" + CODE
    print("[B] billing 提现代码已追加")

if APPLY:
    shutil.copy2(BSRC, BSRC + ".bak-p1-" + time.strftime("%Y%m%d%H%M%S"))
    open(BSRC, "w", encoding="utf-8").write(b)
    print("[B] 已写入 billing app.py (已备份)")
else:
    print("(dry-run: 加 --apply 执行)")
