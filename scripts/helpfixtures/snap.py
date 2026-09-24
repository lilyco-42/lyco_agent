"""把 help-parse --json 输出固化成逐字快照（一行一条 full_cmd）。

用法: python3 snap.py <cli> <json文件> <输出目录>

CI 的 `help-parse 池回归` job 对每个 fixture 调一次，产出 snapshots/<cli>.txt，
与已提交快照逐字 diff —— 条数相同但命令被换掉的内容级回归也逃不掉。

基线**永远人工固化**：改 help_parse 导致 diff 红，必须逐条核对差异确属改进后，
从 artifact 下载 snapshots/ 覆盖提交；绝不让 CI 自动写基线，
否则回归会被「自动更新基线」掩盖（2026-09-24 定下的纪律）。
"""
import json
import sys


def main():
    if len(sys.argv) != 4:
        print(__doc__)
        sys.exit(2)
    cli, jf, outdir = sys.argv[1], sys.argv[2], sys.argv[3]
    with open(jf, encoding="utf-8") as fh:
        j = json.load(fh)
    acts = j.get("actions") or []
    with open(f"{outdir}/{cli}.txt", "w", encoding="utf-8") as fh:
        for a in acts:
            fh.write((a.get("full_cmd") or a.get("cmd") or "") + "\n")
    print(f"{cli} {len(acts)}")


if __name__ == "__main__":
    main()
