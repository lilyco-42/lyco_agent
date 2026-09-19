#!/usr/bin/env python3
# lyco_mcp_client.py — lyco_agent 侧对接 lilyco 的 MCP 客户端（胶水层）
#
# 对接依据（读 lilyco-mcp/src/lib.rs 得到，非猜）：
#   · 传输 = 换行分隔的 JSON-RPC 2.0 over stdio；每个进程一个请求/响应一行
#   · initialize 的 params.capabilities.{sampling,roots} 决定服务端能否反向请求；
#     不发 sampling 就不会被要求做 sampling
#   · notifications/initialized 等「无 id」消息是通知，绝不能回响应
#   · tools/list  -> {"tools":[{"name","description","inputSchema"}]}
#   · tools/call  params {"name","arguments"}；执行前由服务端 CommandSchema::validate_args 校验
#     带 params._meta.progressToken 时，执行期间会流出 notifications/progress
#   · 服务端可能反向发起请求（id 形如 "srv-N"）→ 本客户端一律回 method not found，避免挂死
#
# 用法：
#   python lyco_mcp_client.py --self-test          # 用内置 mock server 自测（无需 lilyco 二进制）
#   python lyco_mcp_client.py --cmd "lffmpeg --mcp" --list
#   python lyco_mcp_client.py --cmd "lffmpeg --mcp" --call compress '{"input":"a.mp4","output":"b.mp4"}'
import sys, json, subprocess, threading, queue, argparse, shlex, time

PROTO = "2024-11-05"


class McpClient:
    def __init__(self, cmd, on_progress=None, timeout=60.0):
        self.cmd = cmd if isinstance(cmd, list) else shlex.split(cmd)
        self.on_progress = on_progress
        self.timeout = timeout
        self._id = 0
        self._lock = threading.Lock()
        self._pending = {}           # id -> queue
        self.proc = subprocess.Popen(
            self.cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True, bufsize=1, encoding="utf-8")
        self._t = threading.Thread(target=self._reader, daemon=True)
        self._t.start()

    # ---- 内部：读循环 ----
    def _reader(self):
        for line in self.proc.stdout:
            line = line.strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except Exception:
                continue
            has_id = "id" in msg and msg["id"] is not None
            is_req = "method" in msg
            if has_id and not is_req:                       # 我们对服务端请求的响应
                q = self._pending.pop(msg["id"], None)
                if q:
                    q.put(msg)
            elif is_req and has_id:                         # 服务端反向请求 → 拒绝，防挂死
                self._write({"jsonrpc": "2.0", "id": msg["id"],
                             "error": {"code": -32601, "message": f"method not found: {msg['method']}"}})
            else:                                           # 通知（含 progress）
                if msg.get("method") == "notifications/progress" and self.on_progress:
                    try:
                        self.on_progress(msg.get("params", {}))
                    except Exception:
                        pass

    def _write(self, obj):
        assert self.proc.stdin
        self.proc.stdin.write(json.dumps(obj, ensure_ascii=False) + "\n")
        self.proc.stdin.flush()

    def request(self, method, params=None):
        with self._lock:
            self._id += 1
            rid = self._id
        q = queue.Queue()
        self._pending[rid] = q
        self._write({"jsonrpc": "2.0", "id": rid, "method": method, "params": params or {}})
        try:
            msg = q.get(timeout=self.timeout)
        except queue.Empty:
            self._pending.pop(rid, None)
            raise TimeoutError(f"MCP {method} 超时（{self.timeout}s）")
        if "error" in msg:
            raise RuntimeError(f"MCP {method} 错误: {msg['error']}")
        return msg.get("result", {})

    def notify(self, method, params=None):
        self._write({"jsonrpc": "2.0", "method": method, "params": params or {}})

    # ---- 语义封装 ----
    def initialize(self):
        r = self.request("initialize", {
            "protocolVersion": PROTO,
            "capabilities": {"roots": {"listChanged": False}},   # 不声明 sampling → 服务端不会来要
            "clientInfo": {"name": "lyco_agent-mcp-client", "version": "0.1"},
        })
        self.notify("notifications/initialized")
        return r

    def list_tools(self):
        return self.request("tools/list").get("tools", [])

    def call_tool(self, name, arguments, progress_token=None, timeout=None):
        params = {"name": name, "arguments": arguments or {}}
        if progress_token is not None:
            params["_meta"] = {"progressToken": progress_token}
        old, self.timeout = self.timeout, (timeout or self.timeout)
        try:
            return self.request("tools/call", params)
        finally:
            self.timeout = old

    def close(self):
        try:
            self.proc.terminate()
        except Exception:
            pass


def extract_text(result):
    """MCP 结果里取文本（兼容 content 数组 / 纯 dict）"""
    if isinstance(result, dict) and "content" in result:
        parts = [c.get("text", "") for c in result["content"] if isinstance(c, dict) and c.get("type") == "text"]
        return "\n".join(parts) if parts else json.dumps(result, ensure_ascii=False)
    return json.dumps(result, ensure_ascii=False)


# ------------------------------------------------------------------ mock server
MOCK = r'''
import sys, json, time
def out(o): sys.stdout.write(json.dumps(o, ensure_ascii=False)+"\n"); sys.stdout.flush()
TOOLS=[{"name":"echo","description":"回显文本 [safety: read_only]",
        "inputSchema":{"type":"object","properties":{"text":{"type":"string","description":"要回显的文本"}},"required":["text"]}},
       {"name":"slow_add","description":"慢速相加（演示进度） [safety: confirm]",
        "inputSchema":{"type":"object","properties":{"a":{"type":"number"},"b":{"type":"number"}},"required":["a","b"]}}]
for line in sys.stdin:
    line=line.strip()
    if not line: continue
    try: req=json.loads(line)
    except Exception: out({"jsonrpc":"2.0","id":None,"error":{"code":-32700,"message":"parse error"}}); continue
    m=req.get("method"); rid=req.get("id")
    if m=="initialize":
        out({"jsonrpc":"2.0","id":rid,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},
             "serverInfo":{"name":"mock","version":"1.0"}}})
    elif m=="notifications/initialized": pass
    elif m=="ping": out({"jsonrpc":"2.0","id":rid,"result":{}})
    elif m=="tools/list": out({"jsonrpc":"2.0","id":rid,"result":{"tools":TOOLS}})
    elif m=="tools/call":
        p=req.get("params",{}); name=p.get("name"); args=p.get("arguments",{})
        tok=(p.get("_meta") or {}).get("progressToken")
        if name=="echo":
            if "text" not in args:
                out({"jsonrpc":"2.0","id":rid,"error":{"code":-32602,"message":"missing required arg: text"}}); continue
            out({"jsonrpc":"2.0","id":rid,"result":{"content":[{"type":"text","text":"echo: "+str(args["text"])}]}})
        elif name=="slow_add":
            for i in range(1,4):
                if tok is not None:
                    out({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":tok,"progress":i,"total":3}})
                time.sleep(0.05)
            out({"jsonrpc":"2.0","id":rid,"result":{"content":[{"type":"text","text":str(args.get("a",0)+args.get("b",0))}]}})
        else:
            out({"jsonrpc":"2.0","id":rid,"error":{"code":-32602,"message":"unknown tool: "+str(name)}})
    else:
        if rid is not None: out({"jsonrpc":"2.0","id":rid,"error":{"code":-32601,"message":"method not found: "+str(m)}})
'''


def self_test():
    import tempfile, os
    p = os.path.join(tempfile.gettempdir(), "_mock_mcp_server.py")
    open(p, "w", encoding="utf-8").write(MOCK)
    prog = []
    c = McpClient([sys.executable, "-u", p], on_progress=lambda prm: prog.append(prm))
    try:
        init = c.initialize()
        print("[1] initialize ->", json.dumps(init, ensure_ascii=False)[:120])
        tools = c.list_tools()
        print("[2] tools/list ->", [t["name"] for t in tools])
        r = c.call_tool("echo", {"text": "你好"})
        print("[3] call echo ->", extract_text(r))
        r2 = c.call_tool("slow_add", {"a": 2, "b": 3}, progress_token="tok-1")
        print("[4] call slow_add ->", extract_text(r2), "| progress 通知数 =", len(prog))
        try:
            c.call_tool("echo", {})
        except RuntimeError as e:
            print("[5] 缺参被服务端 validate 拦住 ->", str(e)[:80])
        try:
            c.call_tool("nope", {})
        except RuntimeError as e:
            print("[6] 未知工具 ->", str(e)[:60])
        print("SELF_TEST_OK")
    finally:
        c.close()


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--cmd", help="启动 MCP server 的命令，例如 'lffmpeg --mcp'")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--call")
    ap.add_argument("args", nargs="?")
    a = ap.parse_args()
    if a.self_test:
        self_test(); raise SystemExit(0)
    if not a.cmd:
        ap.error("需要 --cmd 或 --self-test")
    c = McpClient(a.cmd, on_progress=lambda p: print("  [progress]", p, file=sys.stderr))
    c.initialize()
    if a.list or not a.call:
        for t in c.list_tools():
            req = t["inputSchema"].get("required", [])
            print(f"- {t['name']}: {t['description']}  必填={req}")
    if a.call:
        res = c.call_tool(a.call, json.loads(a.args) if a.args else {})
        print(extract_text(res))
    c.close()
