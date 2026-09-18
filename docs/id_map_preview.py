#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""id_map_preview.py —— 统一身份迁移预览 (只读)
按 email 自动合并 / 无 email 的列出人工决定项 / 余额对账基准
"""
import collections
import sqlite3

B = sqlite3.connect("/opt/studio-billing/billing.db")
C = sqlite3.connect("/var/lib/compute/compute.db")
P = sqlite3.connect("/opt/proxy-panel/proxpanel.db")

accts = []  # (service, ext_id, username, email, note)
for r in B.execute("select id,username,email from users"):
    accts.append(("billing", str(r[0]), r[1] or "", (r[2] or "").strip().lower(), ""))
for r in C.execute("select id,username from users"):
    accts.append(("compute", r[0], r[1] or "", "", "credits"))
for r in P.execute("select id,username,email from users"):
    tl, tu = P.execute("select traffic_limit,traffic_used from users where id=?", (r[0],)).fetchone()
    accts.append(("proxy-panel", str(r[0]), r[1] or "", (r[2] or "").strip().lower(),
                  "流量 %s/%s" % (tu, tl)))

by_email = collections.defaultdict(list)
no_email = []
for a in accts:
    (by_email[a[3]] if a[3] else no_email).append(a)

print("========== A) 按 email 自动合并的账号组 ==========")
for e, g in sorted(by_email.items()):
    if len(g) > 1:
        print("  ● %s  (合并 %d 账号):" % (e, len(g)))
        for x in g:
            print("      [%s] %s (%s)" % (x[0], x[2], x[1]))

print("\n========== B) 单服务账号 (各自成户) ==========")
for e, g in sorted(by_email.items()):
    if len(g) == 1:
        print("  · %-30s [%s] %s" % (e or "(空)", g[0][0], g[0][2]))

print("\n========== C) 无 email —— 需人工归属 ==========")
for a in no_email:
    extra = ""
    if a[0] == "compute":
        extra = "credits=%d" % C.execute("select credits from users where id=?", (a[1],)).fetchone()[0]
    elif a[0] == "proxy-panel":
        tl, tu = P.execute("select traffic_limit,traffic_used from users where id=?", (a[1],)).fetchone()
        extra = "流量 %s/%s 字节" % (tu, tl)
    print("  ○ [%s] %-18s (%s)  %s" % (a[0], a[2], a[1], extra))

print("\n========== D) 余额对账基准 ==========")
tot_c = C.execute("select sum(credits) from users").fetchone()[0]
tot_b = B.execute("select sum(balance) from points_accounts").fetchone()[0]
print("  compute credits 合计 : %d" % tot_c)
print("  billing points 合计  : %d" % tot_b)
print("  统一积分 = billing 余额 + compute credits × 汇率 (汇率待定)")
print("\n账号总数: %d (billing %d / compute %d / proxy-panel %d)"
      % (len(accts), len(B.execute("select 1 from users").fetchall()),
         len(C.execute("select 1 from users").fetchall()),
         len(P.execute("select 1 from users").fetchall())))
