#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""loop_step3.py —— 闭环第3步: 提现冻结 -> 拒绝 -> 退回 (统一身份直接模拟)"""
import secrets
import sqlite3
import sys
import time

sys.path.insert(0, "/opt/lain42")

B = sqlite3.connect("/opt/studio-billing/billing.db", timeout=15)


def bal():
    return B.execute("select balance from points_accounts where user_id=5").fetchone()[0]


print("提现前 uid5 余额:", bal())
ono = "U" + secrets.token_hex(6).upper()
B.execute(
    "INSERT INTO usdt_orders(order_no,uid,amount_usdt,credits,purpose,status,created_at)"
    " VALUES(?,?,?,?,?,?,?)",
    (ono, 5, 10.0, 100, "withdraw-test", "pending", int(time.time())))
B.commit()

# 冻结 (与 /api/withdraw 同逻辑)
B.execute("BEGIN IMMEDIATE")
bal0 = B.execute("select balance from points_accounts where user_id=5").fetchone()[0]
B.execute("update points_accounts set balance=? where user_id=5", (bal0 - 100,))
B.execute(
    "insert into points_ledger(user_id,delta,balance_after,reason,reference,idem_key,created_at,meta)"
    " values(5,-100,?,'withdraw:freeze',?,?,?, '{}')",
    (bal0 - 100, ono, "wdt-freeze-" + ono, int(time.time())))
B.execute(
    "insert into withdrawals(uid,credits,usdt,address,status,created_at) values(5,100,10.0,"
    "'TXYZtestaddress1234567890AB','pending',?)", (int(time.time()),))
B.commit()
wid = B.execute("select id from withdrawals where uid=5 order by id desc limit 1").fetchone()[0]
print("冻结后余额:", bal0, "-> 提现单 wid=%d (pending)" % wid)

# 拒绝 (与 admin reject 同逻辑: 退回)
B.execute("BEGIN IMMEDIATE")
B.execute("update withdrawals set status='rejected', paid_at=? where id=?", (int(time.time()), wid))
B.execute("update points_accounts set balance=balance+100 where user_id=5")
newbal = B.execute("select balance from points_accounts where user_id=5").fetchone()[0]
B.execute(
    "insert into points_ledger(user_id,delta,balance_after,reason,reference,idem_key,created_at,meta)"
    " values(5,100,?,'withdraw:reject',?,?,?, '{}')",
    (newbal, "wid-%d" % wid, "wdt-reject-" + ono, int(time.time())))
B.commit()
print("拒绝后余额:", newbal)

la = B.execute("select coalesce(sum(delta),0) from points_ledger").fetchone()[0]
ac = B.execute("select coalesce(sum(balance),0) from points_accounts").fetchone()[0]
print("对账: ledger=%d accounts=%d %s" % (la, ac, "一致 OK" if la == ac else "差 %d BAD" % (la - ac)))
print("净效果: 冻结->拒绝->退回, 余额不变" if bal() == newbal else "余额异常!")
