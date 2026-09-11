# rewrite_grpo_v4 评测报告（2026-09-11）

模型：`models/qwen3_lyco_rewrite_v4.tar`（v4 训练产物，2026-09-10 09:05 下载）
环境：CloudStudio T4 15GB / torch 2.10 / transformers 5.1 / fp16（T4 无 bf16）
脚本：`tools/rewrite_grpo_v4_eval.py`（25 组口语 → lyv.lookup 命中 + 意图正确）
基线：v2 = 7/10（旧 10 组集，不可直接对比）；v3 = 64%（README 记录）

## 总分

**16/25 = 64%**（与 v3 持平，未提升）

## 分意图

| intent | 得分 | 说明 |
|---|---|---|
| rust.project.run | 5/5 | 满分 |
| py.pkg.install | 2/2 | v3 诊断的薄弱项已修复 |
| rust.project.create | 4/5 | 「开个新工程练练手」→ 误判 run |
| rust.project.build | 2/3 | 「项目怎么构建出来」→ 误判 create |
| git.push | 3/4 | 「保存我的修改」→ git.commit |
| fs.chdir | 0/4 | **3/4 是评测数据 bug**（见下），1 条真错 |
| node.pkg.install | 0/2 | 真薄弱：npm 相关全部 misc.talk |

## 关键发现：fs.chdir 0/4 主要是评测标签 bug

「怎么进入那个文件夹 / 切到项目目录里 / 到代码目录下去」三条，模型输出
（如何 进入 文件夹 / 切 项目 目录 / 到 代码 目录）全部被 lyv.lookup 正确召回，
但命中的 intent 是 **`cd.hello`**（知识包里 chdir 条目的实际命名），
而 v4 训练数据 ORAL_PATTERNS 期望写的是 **`fs.chdir`** —— 标签不一致，
完美模型也会判 0 分。真正错误只有「跳转到工程文件夹 → git.push」1 条。

**行动项（下一轮 v5）**：
1. 修正 chdir 样本的期望 intent 为包内实际值（cd.hello 或重命名包条目）。
2. 修正「保存我的修改」期望（git.commit 与 git.push 语义边界需在样本里区分）。
3. 补 node.pkg.install 知识包条目或标准查询词（npm 系当前召回落 misc.talk）。
4. build/create 边界再加 2-3 组对比样本（「构建出来」≠「新建」）。

## v5 结果（2026-09-11 当日跟进）

v5 = 变体扩充（原 v5 草案）+ 标签对齐（上述 1/2/4）合并，从 v4 底座训 300 步（T4 fp32+AMP，
9 分钟）。**评测 33/37 = 89%**（v4 同口径修正后 80%，+9pt）。

| intent | v5 |
|---|---|
| rust.project.run | 7/7 |
| git.commit | 4/4 |
| py.pkg.install | 3/3 |
| rust.project.create | 8/9 |
| rust.project.build | 4/5 |
| cd.hello | 4/5 |
| git.push | 3/4 |

剩余 4 条 FAIL：build/create 混淆 ×2（「项目怎么构建出来/项目文件怎么生成」）、
chdir 偶发 ×1、「代码提交到仓库」→git.commit（该条期望 git.push 本身语义可议，
commit 其实更贴切，v6 应修正样本而非模型）。
npm 类已按「包无内容」移出训练/评测集，属知识包缺口而非模型问题。
产物：`/workspace/qwen3_lyco_rewrite_v5`（云端）+ 本地归档 tar。

## 传输与复现备注

- v4.tar 1.5G：lain42 中继单流仅 0.25MB/s（阿里云出带宽 ~2Mbps 封顶），
  下载至 938MB 后工作区被「定时关机」停止；剩余 576MB 改 Jupyter 分块直传
  （6×100MB，~2.2MB/s，5.5 分钟），云端拼接后 SHA-256 与本地一致
  （`7a4973ab9450d9fc…`）。
- 踩坑：transformers 5.x `from_pretrained` 参数名 `dtype`；`device_map` 需 accelerate
  （pip install accelerate 1.15.0）；工作区重启后 /workspace 磁盘内容保留。
