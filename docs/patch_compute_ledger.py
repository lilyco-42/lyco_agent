#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""patch_compute_ledger.py v2 —— 逐行精准给 compute/server.py 接统一账本
按"唯一标记行"在其后插入镜像调用, 缩进自动对齐; 幂等(有标记不重复打)。
"""
import shutil
import time

SRC = "/opt/compute/server.py"
BAK = SRC + ".bak-ledger-" + time.strftime("%Y%m%d%H%M%S")

shutil.copy2(SRC, BAK)
lines = open(SRC, encoding="utf-8").read().split("\n")

if "import id_ledger" in "\n".join(lines):
    print("已打过补丁, 跳过")
    raise SystemExit(0)

out = []
n = 0


def indent_of(line):
    return line[: len(line) - len(line.lstrip())]


def stmt_indent(lines, i):
    """i 是续行(如 INSERT 的第二行); 向上找语句起始行, 返回其缩进"""
    j = i
    while j > 0 and not lines[j].lstrip().startswith(("db.execute", "cur.execute")):
        j -= 1
    return indent_of(lines[j])


for i, line in enumerate(lines):
    out.append(line)

    # 1) import 注入点
    if line.startswith("def now(): return int(time.time())"):
        out.append('import sys as _sys, time as _tmod')
        out.append('_sys.path.insert(0, "/opt/lain42")')
        out.append('import id_ledger')
        n += 1

    # 2) create_task 扣费 (credit_log 写入 -cost 的续行之后)
    if 'newid("cl"), u[0], -cost, "调用服务 " + row[3], now())' in line:
        ind = stmt_indent(lines, i)
        out.append(ind + 'id_ledger.mirror("compute", u[0], -cost, "调用服务 " + row[3], "task-debit-" + str(_tmod.time_ns()))')
        n += 1

    # 3) add_credit 加分
    if 'newid("cl"), user_id, amount, reason, now())' in line:
        ind = stmt_indent(lines, i)
        out.append(ind + 'id_ledger.mirror("compute", user_id, amount, reason, "compute-credit-" + str(_tmod.time_ns()))')
        n += 1

    # 4) 退款两处 (row[8] 加回)
    if 'db.execute("UPDATE users SET credits=credits+? WHERE id=?", (row[8], row[3]))' in line:
        ind = indent_of(line)
        out.append(ind + 'id_ledger.mirror("compute", row[3], row[8], "任务退款", "refund-" + str(_tmod.time_ns()))')
        n += 1
    if 'db.execute("UPDATE users SET credits=credits+? WHERE id=?", (row[8], row[2]))' in line:
        ind = indent_of(line)
        out.append(ind + 'id_ledger.mirror("compute", row[2], row[8], "任务退款", "refund-" + str(_tmod.time_ns()))')
        n += 1

open(SRC, "w", encoding="utf-8").write("\n".join(out))
print("备份:", BAK)
print("打补丁处数:", n)
print("mirror 调用数:", "\n".join(out).count("id_ledger.mirror("))
