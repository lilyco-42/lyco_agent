#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
new-api 渠道池一键配置脚本 (lain42.top:3000)
用法:
  python3 newapi_provision.py --url http://127.0.0.1:3000 \
      --user root --pass '<你的后台密码>' \
      [--dry-run]   # 只打印将要添加的渠道, 不实际调用

渠道 type 码 (来自 new-api constant 包, v1.0.0-rc.27):
  Zhipu=16  Ali/通义=17  Tencent/混元=23  DeepSeek=43
  SiliconFlow=40  Moonshot=25  OpenRouter=20  OpenAI=1

把下面 PROVIDERS 里对应 key 填上即可。空 key 的渠道会被跳过。
"""
import argparse, json, sys, urllib.request, urllib.error

CHANNEL_TYPES = {
    "zhipu": 16, "ali": 17, "tencent": 23, "deepseek": 43,
    "siliconflow": 40, "moonshot": 25, "openrouter": 20, "openai": 1,
}

# ===== 在此填入你的上游 key (空字符串的渠道会被跳过) =====
PROVIDERS = [
    # 智谱 GLM: GLM-4.7-Flash 永久免费, 国内直连, OpenAI 兼容
    {"name": "智谱 GLM", "type": "zhipu", "key": "",
     "base_url": "https://open.bigmodel.cn", "models": "*"},
    # 阿里云百炼 / 通义: 有免费额度, 国内直连
    {"name": "阿里通义", "type": "ali", "key": "",
     "base_url": "https://dashscope.aliyuncs.com", "models": "*"},
    # 腾讯混元: 有免费额度, 国内直连
    {"name": "腾讯混元", "type": "tencent", "key": "",
     "base_url": "https://hunyuan.tencentcloudapi.com", "models": "*"},
    # DeepSeek: 平台注册送额度, 国内直连
    {"name": "DeepSeek", "type": "deepseek", "key": "",
     "base_url": "https://api.deepseek.com", "models": "*"},
    # 硅基流动: 注册送 2000万 token, 国内直连, 延迟低
    {"name": "硅基流动", "type": "siliconflow", "key": "",
     "base_url": "https://api.siliconflow.cn", "models": "*"},
    # 月之暗面 Kimi: 有免费额度
    {"name": "月之暗面 Kimi", "type": "moonshot", "key": "",
     "base_url": "https://api.moonshot.cn", "models": "*"},
    # OpenRouter: 一个 key 调几十个模型 (免费档有限)
    {"name": "OpenRouter", "type": "openrouter", "key": "",
     "base_url": "https://openrouter.ai/api", "models": "*"},
]
# ===============================================================


def req(url, method="GET", token=None, data=None):
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = "Bearer " + token
    body = json.dumps(data).encode() if data is not None else None
    r = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(r, timeout=20) as resp:
            return resp.status, json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read().decode())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://127.0.0.1:3000")
    ap.add_argument("--user", default="root")
    ap.add_argument("--pass", dest="password", required=True)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    active = [p for p in PROVIDERS if p["key"]]
    if not active:
        print("[!] PROVIDERS 里没有任何已填 key 的渠道。请先在脚本顶部填入至少一个上游 key。")
        sys.exit(1)

    print("=== 登录 new-api (%s) ===" % args.url)
    st, login = req(args.url + "/api/user/login", "POST",
                    data={"username": args.user, "password": args.password})
    if not login.get("success"):
        print("[x] 登录失败:", login.get("message"))
        sys.exit(1)
    token = login["data"]["access_token"]
    print("[✓] 登录成功, token 长度", len(token))

    for p in active:
        payload = {
            "name": p["name"],
            "type": CHANNEL_TYPES[p["type"]],
            "key": p["key"],
            "base_url": p["base_url"],
            "models": p["models"],
            "model_mapping": "",
            "groups": "default",
            "status": 1,
            "priority": 10,
            "weight": 1,
            "auto_ban": 0,
        }
        if args.dry_run:
            print("[dry] 将添加:", payload["name"], "type=%d" % payload["type"], "base=%s" % p["base_url"])
            continue
        st, resp = req(args.url + "/api/channel/", "POST", token, payload)
        ok = resp.get("success")
        print(("[✓]" if ok else "[x]"), "添加", p["name"], "->", resp.get("message"), "(http %d)" % st)
        if ok and resp.get("data", {}).get("id"):
            cid = resp["data"]["id"]
            st2, t = req(args.url + "/api/channel/test", "POST", token, {"id": cid})
            print("    测试:", ("[✓] 通过" if t.get("success") else "[!] " + str(t.get("message"))), "(http %d)" % st2)

    if not args.dry_run:
        st, lst = req(args.url + "/api/channel/?p=1&page_size=100", "GET", token)
        total = lst.get("data", {}).get("total", 0)
        print("\n=== 渠道池现状: %d 个 ===" % total)


if __name__ == "__main__":
    main()
