#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""patch_sms.py —— 商业化改造: 接入阿里云短信验证 (备用登录 + 手机绑定)

新增 (全部追加, 不改老代码):
  POST /api/sms/send    发送验证码 (60s 冷却 / 每号每日 10 条上限)
  POST /api/login/phone 手机号+验证码登录 (备用登录/找回)
  POST /api/sms/bind    已登录用户绑定手机
配置 (环境变量, 未配置时接口返回 503):
  BILLING_SMS_AK / BILLING_SMS_SECRET / BILLING_SMS_SIGN / BILLING_SMS_TEMPLATE
依赖: pip install alibabacloud-dysmsapi20170525 alibabacloud-tea-openapi
"""
SRC = "/opt/studio-billing/app.py"
src = open(SRC, encoding="utf-8").read()
if "api/sms/send" in src:
    print("已打过, 跳过")
    raise SystemExit(0)

# admin 用户列表补 phone 列
src = src.replace(
    'SELECT u.id, u.username, u.email, u.founder,',
    'SELECT u.id, u.username, u.email, u.phone, u.founder,', 1)

CODE = '''

# ===== lyco: 阿里云短信验证 (备用登录 + 手机绑定) =====
import random as _rnd

SMS_AK = os.environ.get("BILLING_SMS_AK", "")
SMS_SECRET = os.environ.get("BILLING_SMS_SECRET", "")
SMS_SIGN = os.environ.get("BILLING_SMS_SIGN", "")
SMS_TEMPLATE = os.environ.get("BILLING_SMS_TEMPLATE", "")


def _sms_ready() -> bool:
    return bool(SMS_AK and SMS_SECRET and SMS_SIGN and SMS_TEMPLATE)


def _ensure_sms_tables():
    conn = db()
    cols = [c[1] for c in conn.execute("pragma table_info(users)")]
    if "phone" not in cols:
        conn.execute("ALTER TABLE users ADD COLUMN phone TEXT")
    conn.execute(
        """CREATE TABLE IF NOT EXISTS sms_codes(
             phone TEXT PRIMARY KEY, code TEXT, expires_at INTEGER,
             used INTEGER DEFAULT 0, created_at INTEGER)""")
    conn.commit()


def _sms_send(phone: str, code: str):
    from alibabacloud_dysmsapi20170525.client import Client as _Client
    from alibabacloud_dysmsapi20170525 import models as _sms_models
    from alibabacloud_tea_openapi import models as _open_models
    cfg = _open_models.Config(access_key_id=SMS_AK, access_key_secret=SMS_SECRET)
    cfg.endpoint = "dysmsapi.aliyuncs.com"
    cli = _Client(cfg)
    req = _sms_models.SendSmsRequest(
        phone_numbers=phone, sign_name=SMS_SIGN, template_code=SMS_TEMPLATE,
        template_param='{"code":"%s"}' % code)
    resp = cli.send_sms(req)
    bk = resp.body
    if getattr(bk, "code", "") != "OK":
        raise HTTPException(502, "短信发送失败: %s" % getattr(bk, "message", "未知错误"))


def _check_sms_code(conn, phone: str, code: str) -> bool:
    import time as _t
    row = conn.execute("SELECT code, expires_at, used FROM sms_codes WHERE phone=?",
                       (phone,)).fetchone()
    return bool(row and not row[2] and row[1] >= int(_t.time()) and row[0] == code)


@app.post("/api/sms/send")
async def sms_send(req: Request):
    if not _sms_ready():
        raise HTTPException(503, "短信服务未配置")
    body = await req.json()
    phone = str(body.get("phone", "")).strip()
    if not re.fullmatch(r"1\\d{10}", phone):
        raise HTTPException(400, "手机号格式错误")
    import time as _t
    conn = db()
    _ensure_sms_tables()
    last = conn.execute("SELECT created_at FROM sms_codes WHERE phone=?", (phone,)).fetchone()
    now_ = int(_t.time())
    if last and now_ - last[0] < 60:
        raise HTTPException(429, "发送太频繁, 请 60 秒后再试")
    day = conn.execute("SELECT COUNT(*) FROM sms_codes WHERE phone=? AND created_at>?",
                       (phone, now_ - 86400)).fetchone()[0]
    if day >= 10:
        raise HTTPException(429, "今日发送次数已达上限")
    code = "%06d" % _rnd.randint(0, 999999)
    _sms_send(phone, code)
    conn.execute(
        "INSERT INTO sms_codes(phone,code,expires_at,used,created_at) VALUES(?,?,?,0,?)"
        " ON CONFLICT(phone) DO UPDATE SET code=excluded.code, expires_at=excluded.expires_at,"
        " used=0, created_at=excluded.created_at",
        (phone, code, now_ + 300, now_))
    conn.commit()
    return {"ok": True, "note": "验证码 5 分钟内有效"}


@app.post("/api/login/phone")
async def login_phone(req: Request):
    """备用登录: 手机号 + 短信验证码 (需已绑定手机)"""
    if not _sms_ready():
        raise HTTPException(503, "短信服务未配置")
    body = await req.json()
    phone = str(body.get("phone", "")).strip()
    code = str(body.get("code", "")).strip()
    conn = db()
    _ensure_sms_tables()
    if not _check_sms_code(conn, phone, code):
        raise HTTPException(400, "验证码错误或已过期")
    conn.execute("UPDATE sms_codes SET used=1 WHERE phone=?", (phone,))
    u = conn.execute("SELECT id, username, token, founder FROM users WHERE phone=?", (phone,)).fetchone()
    conn.commit()
    if not u:
        raise HTTPException(404, "该手机号未绑定账号, 请先用账号密码登录后绑定手机")
    token = jwt_sign({"uid": u["id"], "un": u["username"], "exp": time.time() + 30 * 86400})
    return {"ok": True, "token": token, "username": u["username"], "founder": bool(u["founder"])}


@app.post("/api/sms/bind")
async def sms_bind(req: Request, authorization: str | None = Header(None)):
    """已登录用户绑定手机 (作为备用登录/找回方式)"""
    if not _sms_ready():
        raise HTTPException(503, "短信服务未配置")
    user = auth_user(authorization)
    body = await req.json()
    phone = str(body.get("phone", "")).strip()
    code = str(body.get("code", "")).strip()
    conn = db()
    _ensure_sms_tables()
    if not _check_sms_code(conn, phone, code):
        raise HTTPException(400, "验证码错误或已过期")
    other = conn.execute("SELECT id FROM users WHERE phone=? AND id<>?", (phone, user["id"])).fetchone()
    if other:
        raise HTTPException(400, "该手机号已被其他账号绑定")
    conn.execute("UPDATE sms_codes SET used=1 WHERE phone=?", (phone,))
    conn.execute("UPDATE users SET phone=? WHERE id=?", (phone, user["id"]))
    conn.commit()
    return {"ok": True, "phone": phone}
# ===== lyco: 短信验证结束 =====
'''

open(SRC, "w", encoding="utf-8").write(src + "\n" + CODE)
print("已追加短信验证代码到", SRC)
