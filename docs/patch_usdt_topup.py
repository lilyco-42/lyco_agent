#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""patch_usdt_topup.py —— 商业化: USDT 充值自动到账 (统一入口)

A) billing app.py (追加):
   POST /api/usdt/order  创建充值订单 (Bearer) {amount_usdt} -> 返回 order_no + 收款地址
   GET  /api/usdt/orders 我的充值订单
   充值到账由独立 watcher 完成: 积分 = amount_usdt × 10, ledger 幂等(usdt:tx)
B) /opt/lain42/usdt_watcher.py: TronGrid 轮询 -> 按金额匹配 pending usdt_orders
   -> 标记 paid + 积分入账 (BEGIN IMMEDIATE, idem=usdt:tx:order)
C) systemd: lain42-usdt-watcher.service
幂等; 自动备份。
"""
import shutil
import time

APPLY = "--apply" in __import__("sys").argv
ok = []

# ===== A) billing =====
BS = "/opt/studio-billing/app.py"
b = open(BS, encoding="utf-8").read()
if "/api/usdt/order" not in b:
    code = '''

# ===== lyco: USDT 充值 (统一入口, 10 积分 = 1 USDT) =====
USDT_WALLET = "TKL9TuXXHu9oubcH36SndwKXwp3W1fxzUJ"
USDT_MIN = 1.0


@app.post("/api/usdt/order")
async def usdt_order_create(req: Request, authorization: str | None = Header(None)):
    user = auth_user(authorization)
    body = await req.json()
    try:
        amount = round(float(body.get("amount_usdt", 0)), 3)
    except Exception:
        raise HTTPException(400, "amount_usdt 必须是数字")
    if amount < USDT_MIN:
        raise HTTPException(400, "最低充值 %g USDT" % USDT_MIN)
    credits = int(amount * 10)
    import secrets as _s, time as _t
    order_no = "U" + _s.token_hex(6).upper()
    conn = db()
    conn.execute(
        "INSERT INTO usdt_orders(order_no, uid, amount_usdt, credits, purpose, status, created_at)"
        " VALUES(?,?,?,?,?,?,?)",
        (order_no, user["id"], amount, credits, "topup", "pending", int(_t.time())))
    conn.commit()
    return {"ok": True, "order_no": order_no, "amount_usdt": amount, "credits": credits,
            "wallet": USDT_WALLET,
            "note": "向该 TRC20 地址转账正好 %g USDT, 系统确认后自动到账" % amount}


@app.get("/api/usdt/orders")
async def usdt_order_mine(authorization: str | None = Header(None)):
    user = auth_user(authorization)
    conn = db()
    try:
        rows = conn.execute(
            "SELECT order_no, amount_usdt, credits, status, tx_hash, created_at, paid_at"
            " FROM usdt_orders WHERE uid=? ORDER BY id DESC LIMIT 50",
            (user["id"],)).fetchall()
    except Exception:
        return {"items": []}
    return {"items": [dict(r) for r in rows]}
# ===== lyco: USDT 充值结束 =====
'''
    b = b + "\n" + code
    ok.append(("A 充值订单 API", True))
else:
    ok.append(("A 充值订单 API", "已打过"))

# ===== B) watcher =====
W = "/opt/lain42/usdt_watcher.py"
WATCHER = '''#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""lain42 统一 USDT 充值 watcher —— 唯一入口
轮询 TronGrid 确认转账 -> 按金额匹配 pending usdt_orders(billing) -> paid + 积分入账。
幂等: ledger idem_key = usdt:{tx}:{order_no}; 同一 tx 只匹配一单。
"""
import json
import sqlite3
import time
import urllib.request

BILLING = "/opt/studio-billing/billing.db"
WALLET = "TKL9TuXXHu9oubcH36SndwKXwp3W1fxzUJ"
CONTRACT = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"
GRID = "https://api.trongrid.io"
RATE = 10


def log(m):
    print(time.strftime("[%F %T]"), m, flush=True)


def fetch_transfers():
    url = "%s/v1/accounts/%s/transactions/trc20?limit=50&only_confirmed=true&contract_address=%s" % (
        GRID, WALLET, CONTRACT)
    req = urllib.request.Request(url, headers={"accept": "application/json"})
    with urllib.request.urlopen(req, timeout=15) as r:
        return json.load(r).get("data", [])


def settle(conn, order, tx):
    """订单到账: 标记 paid + 积分入账 (幂等)"""
    cur = conn.execute("BEGIN IMMEDIATE")
    row = conn.execute("SELECT status FROM usdt_orders WHERE order_no=?", (order["order_no"],)).fetchone()
    if not row or row[0] != "pending":
        conn.rollback()
        return False
    uid = order["uid"]
    credits = int(order["credits"])
    bal = conn.execute("SELECT balance FROM points_accounts WHERE user_id=?", (uid,)).fetchone()
    newbal = (bal[0] if bal else 0) + credits
    if bal:
        conn.execute("UPDATE points_accounts SET balance=?, updated_at=? WHERE user_id=?",
                     (newbal, int(time.time()), uid))
    else:
        conn.execute("INSERT INTO points_accounts(user_id,balance,updated_at) VALUES(?,?,?)",
                     (uid, newbal, int(time.time())))
    conn.execute(
        "INSERT OR IGNORE INTO points_ledger(user_id,delta,balance_after,reason,reference,idem_key,created_at,meta)"
        " VALUES(?,?,?,?,?,?,?,?)",
        (uid, credits, newbal, "usdt:topup", order["order_no"],
         "usdt:%s:%s" % (tx["tx"], order["order_no"]), int(time.time()),
         json.dumps({"tx": tx["tx"], "usdt": tx["amount"]}, ensure_ascii=False)))
    conn.execute("UPDATE usdt_orders SET status='paid', tx_hash=?, paid_at=? WHERE order_no=?",
                 (tx["tx"], int(time.time()), order["order_no"]))
    conn.commit()
    log("到账: %s %d 积分 (%g USDT, tx %s...)" % (order["order_no"], credits, tx["amount"], tx["tx"][:14]))
    return True


def main():
    log("usdt watcher 启动, wallet=" + WALLET)
    seen = set()
    while True:
        try:
            conn = sqlite3.connect(BILLING, timeout=15)
            pending = conn.execute(
                "SELECT order_no, uid, amount_usdt, credits, created_at FROM usdt_orders"
                " WHERE status='pending' AND created_at > ?",
                (int(time.time()) - 86400 * 3,)).fetchall()
            if pending:
                amounts = {round(r[2], 3): {"order_no": r[0], "uid": r[1], "credits": r[3],
                                            "created_at": r[4]} for r in pending}
                for tx in fetch_transfers():
                    to = tx.get("to", "")
                    ttype = (tx.get("token_info", {}) or {}).get("address", "")
                    if to != WALLET or ttype != CONTRACT:
                        continue
                    txid = tx.get("transaction_id", "")
                    if txid in seen:
                        continue
                    amount = round(float(tx.get("value", 0)) / 1e6, 3)
                    m = amounts.get(amount)
                    if m and tx.get("block_timestamp", 0) / 1000 >= m["created_at"] - 300:
                        settle(conn, m, {"tx": txid, "amount": amount})
                        seen.add(txid)
                    else:
                        seen.add(txid)   # 无人认领的转账也记_seen, 避免重复扫
            conn.close()
        except Exception as e:
            log("poll error: " + str(e)[:200])
        time.sleep(15)


if __name__ == "__main__":
    main()
'''

# ===== C) systemd =====
UNIT = '''[Unit]
Description=lain42 unified USDT topup watcher
After=network-online.target

[Service]
Type=simple
ExecStart=/usr/bin/python3 /opt/lain42/usdt_watcher.py
Restart=always
RestartSec=5
StandardOutput=journal

[Install]
WantedBy=multi-user.target
'''

print("=== 补丁清单 ===")
for name, st in ok:
    print("  %-24s %s" % (name, "✓" if st is True else st))
if APPLY:
    shutil.copy2(BS, BS + ".bak-usdt-" + time.strftime("%Y%m%d%H%M%S"))
    open(BS, "w", encoding="utf-8").write(b)
    open(W, "w", encoding="utf-8").write(WATCHER)
    open("/etc/systemd/system/lain42-usdt-watcher.service", "w").write(UNIT)
    print("已写入 billing + watcher + systemd 单元 (已备份)")
else:
    print("(dry-run: 加 --apply 执行)")
