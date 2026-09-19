# Search HuggingFace for NL->command (CLI/shell) datasets. Local run (no GPU).
import json, ssl, urllib.request, urllib.parse

ctx = ssl.create_default_context()

def api_search(kw, limit=15):
    q = urllib.parse.urlencode({"search": kw, "limit": limit, "sort": "downloads", "direction": "-1"})
    url = "https://huggingface.co/api/datasets?" + q
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    try:
        with urllib.request.urlopen(req, timeout=30, context=ctx) as r:
            return json.load(r)
    except Exception as e:
        print(f"  !! search '{kw}' failed: {e}")
        return []

for kw in ["bash", "shell command", "command line", "cli", "powershell", "terminal command", "nl2bash"]:
    print(f"\n=== {kw} ===")
    rows = api_search(kw)
    if not rows:
        continue
    for d in rows:
        print(f"{d.get('downloads', 0):>9}  {d.get('id')}")
