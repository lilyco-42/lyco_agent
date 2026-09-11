# -*- coding: utf-8 -*-
"""lilyco_bridge.py — lilyco 框架 ↔ lyco agent 生态桥

能力: lilyco app --schema → OpenAI tools → lyco_chat 工具注册
      用户问 → 模型 tool_call(imgcompress) → 执行 lilyco app → 结果回填

这是 lilyco(工具生成框架) ↔ lyco(智能协助助手) 的生态连接:
  lilyco 生成 CLI 工具 → --schema 自动注册为 lyco 工具 → agent 可调用
  新 lilyco 应用 = 新 agent 能力 (零训练, 零代码)
"""
import json
import subprocess
import sys
from pathlib import Path


def schema_to_tool(schema_exe):
    """lilyco app --schema → OpenAI tool 定义"""
    r = subprocess.run([schema_exe, "--schema"], capture_output=True, text=True, timeout=30)
    if r.returncode != 0 or not r.stdout.strip().startswith("{"):
        return None
    schema = json.loads(r.stdout)
    name = schema["name"].lower()
    properties, required = {}, []
    for arg in schema.get("args", []):
        prop = {"type": "string", "description": arg.get("about", "")}
        kind = arg.get("kind", {}).get("type", "text")
        if kind == "number":
            prop["type"] = "number"
        elif kind == "flag":
            prop["type"] = "boolean"
        elif kind == "enum":
            prop["type"] = "string"
            prop["enum"] = arg["kind"].get("values", [])
        properties[arg["name"]] = prop
        if arg.get("required"):
            required.append(arg["name"])
    return {"type": "function", "function": {
        "name": name, "description": schema.get("about", ""),
        "parameters": {"type": "object", "properties": properties,
                       "required": required}}}


def register_tools(schema_exes, tools_openai_path):
    """批量注册 lilyco 工具到 lyco_chat 的 tools_openai.json"""
    existing = json.load(open(tools_openai_path, encoding="utf-8"))
    existing_names = {t["function"]["name"] for t in existing}
    added = []
    for exe in schema_exes:
        tool = schema_to_tool(exe)
        if tool and tool["function"]["name"] not in existing_names:
            existing.append(tool)
            added.append(tool["function"]["name"])
    json.dump(existing, open(tools_openai_path, "w", encoding="utf-8"),
              ensure_ascii=False, indent=1)
    return added


def execute_lilyco(schema_exe, arguments):
    """执行 lilyco app: arguments → CLI flags → run → 返回结果"""
    cmd = [schema_exe]
    for k, v in arguments.items():
        if isinstance(v, bool):
            if v:
                cmd.append(f"--{k}")
        else:
            cmd.extend([f"--{k}", str(v)])
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
    return {"ok": r.returncode == 0, "output": r.stdout[-500:] if r.stdout else "",
            "error": r.stderr[-200:] if r.returncode != 0 else ""}


if __name__ == "__main__":
    tools_path = "D:/Code/rust/lyco_chat/tools_openai.json"
    added = register_tools(
        ["D:/Code/rust/lilyco/target/release/imgpress.exe"], tools_path)
    print(f"注册 lilyco 工具: {added}")
    print(f"tools_openai.json 总工具数: {len(json.load(open(tools_path, encoding='utf-8')))}")
