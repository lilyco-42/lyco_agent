#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""patch_billing_admin_ui2.py —— 让管理页在 pingap 反代(/studio/api/)下正常工作
1. HTML 里的 fetch 改相对路径 (穿透 /studio/api/ 重写)
2. 加 3 个别名路由 (直连 :4700/admin 时也能用)
幂等。
"""
SRC = "/opt/studio-billing/app.py"
s = open(SRC, encoding="utf-8").read()
n = 0

if 'api("/api/admin/users")' in s:
    s = s.replace('api("/api/admin/users")', 'api("admin/users")'); n += 1
if 'api("/api/admin/ledger?n=12")' in s:
    s = s.replace('api("/api/admin/ledger?n=12")', 'api("admin/ledger?n=12")'); n += 1
if '"/api/admin/points/adjust"' in s:
    s = s.replace('"/api/admin/points/adjust"', '"admin/points/adjust"'); n += 1

if "admin_users_alias" not in s:
    aliases = '''

@app.get("/admin/users")
async def admin_users_alias(x_admin_token: str | None = Header(None),
                            authorization: str | None = Header(None)):
    return await admin_users_list(x_admin_token=x_admin_token, authorization=authorization)


@app.get("/admin/ledger")
async def admin_ledger_alias(n: int = 12, x_admin_token: str | None = Header(None),
                             authorization: str | None = Header(None)):
    return await admin_ledger_tail(n=n, x_admin_token=x_admin_token, authorization=authorization)


@app.post("/admin/points/adjust")
async def admin_adjust_alias(req: Request, x_admin_token: str | None = Header(None),
                             authorization: str | None = Header(None)):
    return await admin_points_adjust(req, x_admin_token=x_admin_token, authorization=authorization)
'''
    marker = "# ===== lyco: 管理界面结束 ====="
    s = s.replace(marker, aliases + "\n" + marker)
    n += 1

open(SRC, "w", encoding="utf-8").write(s)
print("改动处数:", n)
