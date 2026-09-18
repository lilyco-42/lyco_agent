#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""id_ledger.py —— 统一积分账本镜像模块 (同机直写 billing.db)

其他服务(compute/proxy-panel)在扣/加积分时调用 mirror():
  · 自动经 identity_links 把服务内 external_id 映射到统一 uid
  · points_ledger 写入(不可变流水) + points_accounts 余额更新
  · idem_key 幂等(防重放) + BEGIN IMMEDIATE(防并发双扣)
"""
import sqlite3
import threading
import time

BILLING = "/opt/studio-billing/billing.db"
_LOCK = threading.Lock()


def _conn():
    return sqlite3.connect(BILLING, timeout=15)


def _uid(cur, service, ext_id):
    row = cur.execute("select uid from identity_links where service=? and external_id=?",
                      (service, str(ext_id))).fetchone()
    return row[0] if row else None


def mirror(service, ext_id, delta, reason, idem_key):
    """把一笔 delta 镜像进统一账本; 返回新余额, 未绑定返回 None"""
    with _LOCK:
        c = _conn()
        try:
            uid = _uid(c, service, ext_id)
            if not uid:
                return None
            c.execute("BEGIN IMMEDIATE")
            row = c.execute("select balance from points_accounts where user_id=?", (uid,)).fetchone()
            bal = row[0] if row else 0
            new = bal + delta
            if row:
                c.execute("update points_accounts set balance=?, updated_at=? where user_id=?",
                          (new, int(time.time()), uid))
            else:
                c.execute("insert into points_accounts(user_id,balance,updated_at) values(?,?,?)",
                          (uid, new, int(time.time())))
            c.execute("insert or ignore into points_ledger(user_id,delta,balance_after,reason,"
                      "reference,idem_key,created_at,meta) values(?,?,?,?,?,?,?,?)",
                      (uid, delta, new, reason, "%s:%s" % (service, ext_id), idem_key,
                       int(time.time()), '{"via":"%s"}' % service))
            c.commit()
            return new
        finally:
            c.close()


def balance(service, ext_id):
    c = _conn()
    try:
        uid = _uid(c, service, ext_id)
        if not uid:
            return None
        row = c.execute("select balance from points_accounts where user_id=?", (uid,)).fetchone()
        return row[0] if row else 0
    finally:
        c.close()
