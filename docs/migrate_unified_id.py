#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""migrate_unified_id.py —— lain42 统一身份+积分迁移 (P2)

决策(2026-09-18 用户拍板):
  · 汇率: 10 积分 = 1 USDT
  · 主键: email (唯一) + 内部自增 id
  · 积分 = 网站流通货币 (所有服务统一积分计价; compute credits 1:1 等于积分)

做法(非破坏):
  1. 在 billing.db 新增 identity_links / service_keys, 并给 orders 增加 credits/purpose 列
  2. 按 email 合并账号 (billing 为主, 其余服务挂 identity_links)
  3. 无 email 的账号: 按 --bind 参数人工归属, 否则挂到"待认领"占位用户
  4. 余额迁移: compute credits 1:1 计入 points_ledger (reason='migrate:compute', 幂等)
  5. 对账: 迁移前后总额必须一致

用法:
  python3 migrate_unified_id.py                 # dry-run (只打印, 不改库)
  python3 migrate_unified_id.py --apply         # 执行 (自动备份)
"""
import argparse
import shutil
import sqlite3
import time

BILLING = "/opt/studio-billing/billing.db"
COMPUTE = "/var/lib/compute/compute.db"
PROXY = "/opt/proxy-panel/proxpanel.db"
RATE = 10.0          # 10 积分 = 1 USDT

APPLY = "--apply" in __import__("sys").argv

# 无 email 账号的人工归属: external_id -> email (留空则挂到占位用户)
BINDINGS = {
    ("compute", "u_d1846342ac964fc2"): 6,   # lilyco42 -> billing lilyco42 (同名)
    ("compute", "u_78b4cee3c2b54656"): 5,   # admin01 -> billing admin
}


def norm(email):
    return (email or "").strip().lower()


def main():
    print("=== 汇率: 1 USDT = %.0f 积分 | 模式: %s ===" % (RATE, "APPLY" if APPLY else "DRY-RUN"))
    b = sqlite3.connect(BILLING)
    c = sqlite3.connect(COMPUTE)
    p = sqlite3.connect(PROXY)

    # ---------- 1) 建表 (非破坏) ----------
    ddl = [
        """CREATE TABLE IF NOT EXISTS identity_links(
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             uid INTEGER NOT NULL,
             service TEXT NOT NULL,
             external_id TEXT NOT NULL,
             external_username TEXT,
             email TEXT,
             linked_at INTEGER,
             UNIQUE(service, external_id))""",
        """CREATE TABLE IF NOT EXISTS service_keys(
             service TEXT PRIMARY KEY, api_key TEXT NOT NULL,
             enabled INTEGER DEFAULT 1, created_at INTEGER)""",
        """CREATE TABLE IF NOT EXISTS usdt_orders(
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             order_no TEXT UNIQUE, uid INTEGER, amount_usdt REAL,
             credits INTEGER, purpose TEXT, status TEXT,
             tx_hash TEXT, created_at INTEGER, paid_at INTEGER)""",
    ]
    if APPLY:
        for d in ddl:
            b.execute(d)
        lcols=[x[1] for x in b.execute('pragma table_info(points_ledger)')]
        if 'idem_key' not in lcols:
            b.execute('ALTER TABLE points_ledger ADD COLUMN idem_key TEXT')
            b.execute('CREATE UNIQUE INDEX IF NOT EXISTS idx_ledger_idem ON points_ledger(idem_key)')
        # 给 orders 补列 (统一订单语义)
        cols = [x[1] for x in b.execute("pragma table_info(orders)")]
        if "credits" not in cols:
            b.execute("ALTER TABLE orders ADD COLUMN credits INTEGER")
        if "purpose" not in cols:
            b.execute("ALTER TABLE orders ADD COLUMN purpose TEXT")
        if "uid" not in cols:
            b.execute("ALTER TABLE orders ADD COLUMN uid INTEGER")
        b.commit()
        print("[1] 建表/补列 ✓ (identity_links / service_keys / usdt_orders + orders.credits,purpose,uid)")
    else:
        print("[1] (dry-run 跳过建表)")

    # ---------- 2) 收集账号 ----------
    billing_users = list(b.execute("select id,username,email,founder from users"))
    compute_users = list(c.execute("select id,username,credits from users"))
    proxy_users = list(p.execute("select id,username,email from users"))
    print("[2] 账号: billing %d / compute %d / proxy-panel %d"
          % (len(billing_users), len(compute_users), len(proxy_users)))

    # email -> billing uid (billing 有 email 的优先作主账号)
    email2uid = {}
    for uid, un, em, fo in billing_users:
        e = norm(em)
        if e:
            email2uid.setdefault(e, uid)

    # ---------- 3) 合并计划 ----------
    print("\n[3] 合并计划:")
    links = []          # (uid, service, ext_id, ext_username, email)
    for uid, un, em, fo in billing_users:
        e = norm(em)
        target = email2uid.get(e, uid) if e else uid
        links.append((target, "billing", str(uid), un, e))
        flag = "" if target == uid else "  <= 并入 uid=%d" % target
        print("   billing  uid=%-3d %-12s %-28s -> uid=%-3d%s" % (uid, un, e or "(无email)", target, flag))
    for pid, pun, pem in proxy_users:
        e = norm(pem)
        key = ("proxy-panel", str(pid))
        if key in BINDINGS:
            e = norm(BINDINGS[key])
        if e and e in email2uid:
            target = email2uid[e]; note = "  <= email 匹配"
        else:
            target = None
            note = "  !! 无 email/未绑定 -> 挂占位用户"
        links.append((target, "proxy-panel", str(pid), pun, e))
        print("   proxy    id=%-3d %-14s %-28s -> %s%s" % (pid, pun, e or "(无email)",
              ("uid=%d" % target) if target else "待认领", note))
    for cid, cun, cr in compute_users:
        key = ("compute", cid)
        bv = BINDINGS.get(key)
        e = norm(bv) if isinstance(bv, str) else ""
        if isinstance(bv, int):
            target = bv; note = "  <= 人工绑定 uid=%d" % bv
        elif e and e in email2uid:
            target = email2uid[e]; note = "  <= 人工绑定(email)"
        else:
            target = None; note = "  !! 无 email/未绑定 -> 挂占位用户 (credits=%d)" % cr
        links.append((target, "compute", cid, cun, e))
        print("   compute  %-10s %-28s -> %s%s" % (cun, e or "(无email)",
              ("uid=%d" % target) if target else "待认领", note))

    # ---------- 4) 余额迁移 + 对账 ----------
    print("\n[4] 余额迁移 (compute credits 1:1 -> 统一积分):")
    mig = []
    for cid, cun, cr in compute_users:
        if cr:
            mig.append(("compute:" + cid, cun, cr))
            print("   + %-10s %4d 积分  (reason=migrate:compute, ref=%s)" % (cun, cr, cid))
    old_total = b.execute("select coalesce(sum(balance),0) from points_accounts").fetchone()[0]
    add_total = sum(x[2] for x in mig)
    print("   迁移前 billing 余额合计 : %d" % old_total)
    print("   本次迁入               : %d" % add_total)
    print("   迁移后预期合计         : %d" % (old_total + add_total))
    print("\n   对账口径: sum(points_ledger.delta) == sum(points_accounts.balance)")

    if APPLY:
        bak = BILLING + ".bak-migrate-" + time.strftime("%Y%m%d%H%M%S")
        shutil.copy2(BILLING, bak)
        print("\n[5] 已备份 -> %s" % bak)

        # 占位用户 (待认领)
        ph = b.execute("select id from users where username='unclaimed'").fetchone()
        if not ph:
            from hashlib import sha256
            import secrets as _s
            salt = _s.token_hex(8)
            h = sha256(("unclaimed" + salt).encode()).hexdigest()
            cur = b.execute("insert into users(username,email,pass_salt,pass_hash,founder,created_at)"
                            " values(?,?,?,?,?,?)", ("unclaimed", "", salt, h, 0, int(time.time())))
            b.commit()
            ph_uid = cur.lastrowid
            print("    占位用户 unclaimed uid=%d 已创建" % ph_uid)
        else:
            ph_uid = ph[0]

        for uid, svc, ext, ext_un, em in links:
            real = uid or ph_uid
            ex = b.execute("select 1 from identity_links where service=? and external_id=?",
                           (svc, ext)).fetchone()
            if not ex:
                b.execute("insert into identity_links(uid,service,external_id,external_username,email,linked_at)"
                          " values(?,?,?,?,?,?)", (real, svc, ext, ext_un, em, int(time.time())))
        b.commit()
        print("    identity_links 写入 %d 条 ✓" % len(links))

        # compute credits -> ledger (幂等: idem_key)
        for ref, un, cr in mig:
            key = "migrate:compute:" + ref
            if b.execute("select 1 from points_ledger where idem_key=?", (key,)).fetchone():
                continue
            # 找目标 uid
            row = b.execute("select uid from identity_links where service='compute' and external_id=?",
                            (ref.split(":", 1)[1],)).fetchone()
            tuid = row[0] if row else ph_uid
            bal = b.execute("select balance from points_accounts where user_id=?", (tuid,)).fetchone()
            newbal = (bal[0] if bal else 0) + cr
            if bal:
                b.execute("update points_accounts set balance=?, updated_at=? where user_id=?",
                          (newbal, int(time.time()), tuid))
            else:
                b.execute("insert into points_accounts(user_id,balance,updated_at) values(?,?,?)",
                          (tuid, newbal, int(time.time())))
            b.execute("insert into points_ledger(user_id,delta,balance_after,reason,reference,idem_key,created_at,meta)"
                      " values(?,?,?,?,?,?,?,?)",
                      (tuid, cr, newbal, "migrate:compute", ref, key, int(time.time()),
                       '{"src":"compute.db"}'))
        b.commit()
        print("    points_ledger 迁入 %d 条 ✓" % len(mig))

        # 对账
        a = b.execute("select coalesce(sum(balance),0) from points_accounts").fetchone()[0]
        l = b.execute("select coalesce(sum(delta),0) from points_ledger").fetchone()[0]
        print("\n[6] 对账: accounts=%d  ledger=%d  %s" % (a, l, "一致 ✓" if a == l else "不一致 ✗ 需排查"))
    else:
        print("\n(这是 dry-run。确认无误后加 --apply 执行, 会自动备份)")


if __name__ == "__main__":
    main()
