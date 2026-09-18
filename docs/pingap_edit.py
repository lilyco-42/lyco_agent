#!/usr/bin/env python3
# 在 pingap.toml 中加入 ai.lain42.top -> new-api(:3000) 的反代 + 证书
import re, sys

P = "/etc/pingap.toml"
s = open(P).read()

assert "[upstreams.newapi_backend]" not in s, "upstream 已存在"
assert "[locations.ai_gateway]" not in s, "location 已存在"
assert "[certificates.ai_gateway]" not in s, "cert 已存在"

# 1) upstream + location 插入到 [locations.compute] 之前
up_loc = '''[upstreams.newapi_backend]
addrs = ["127.0.0.1:3000"]

[locations.ai_gateway]
enable_reverse_proxy_headers = true
host = "ai.lain42.top"
path = "/"
upstream = "newapi_backend"

'''
marker = "[locations.compute]"
assert marker in s, "找不到 [locations.compute] 锚点"
s = s.replace(marker, up_loc + marker, 1)

# 2) 把 ai_gateway 加入 [servers.https] 的 locations 列表
assert s.count('    "signal",\n]') == 1, "signal 列表锚点不唯一"
s = s.replace('    "signal",\n]', '    "signal",\n    "ai_gateway",\n]', 1)

# 3) 证书块(内联 fullchain + key)
fullchain = open("/root/.acme.sh/ai.lain42.top_ecc/fullchain.cer").read().strip()
key = open("/root/.acme.sh/ai.lain42.top_ecc/ai.lain42.top.key").read().strip()
cert_block = (
    '[certificates.ai_gateway]\n'
    'domains = "ai.lain42.top"\n'
    'is_default = false\n'
    'tls_cert = """\n' + fullchain + '\n"""\n'
    'tls_key = """\n' + key + '\n"""\n\n'
)
s = s.rstrip("\n") + "\n\n" + cert_block
open(P, "w").write(s)
print("pingap.toml 已更新 (upstream/location/cert 三处)")
