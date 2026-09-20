# v17/v17b：「不会的 CLI 靠读 `--help` 自学」— 已实测，方向对但形态错

> 需求（用户 2026-09-21）：「提高泛用性，即使不会的 CLI，也能学习 `--help` 学习等」
>
> 两轮实验已跑完，**结论明确且可执行**。本文档回答：这条路能不能走、卡在哪、下一步该做什么。
> 所有数字取自 `/workspace/v17_help_results.json` 与 `/workspace/v17b_results.json`。

---

## 0. 裁决摘要

**方向对，形态错。**

| 判据 | 结论 |
| --- | --- |
| 「读 `--help`」有没有用？ | ✅ **有用**。相对无注入，base +24.3pp、v13 +5.0pp |
| 能否替代人工 schema？ | ❌ **不能**。base `help`=23.2% vs `hand`=100%（**−76.8pp**） |
| 让模型自己提炼 help 成动作表？ | ❌ **不行**。base 仅 +8.2pp；**v13 反而 −18.3pp**（它是路由器，不是整理器） |
| 根因 | 🔴 **0.6B 容量不足以「读散文 → 提炼结构化动作表」**，不是 prompt 问题 |

**正解**：把「提炼」从模型挪到 **Rust 侧确定性解析器**（`lycore help-parse`）。
模型只做它擅长的「对齐已结构化的动作表」，而动作表的生成是**纯字符串处理**，不需要神经网络。

---

## 1. v17：裸灌 `--help`（单阶段）

### 设置
唯一变量 = 注入内容。三个 CLI（docker/kubectl/npm）有手写 schema 可对照，
另加 **git / cargo / jq** 三个 **训练与既往评测都未出现**的 CLI 作真泛化探针。

### 读数（exec %）

| CLI | tier | hand | help | plain | help−hand |
| --- | --- | --- | --- | --- | --- |
| docker | contaminated(29) | **100.0** | 10.0 | 0.0 | −90.0 |
| kubectl | clean(7) | **100.0** | 22.2 | 11.1 | −77.8 |
| npm | contaminated(35) | **100.0** | 37.5 | 12.5 | −62.5 |
| git | UNSEEN | — | **100.0** | 25.0 | — |
| cargo | UNSEEN | — | 0.0 | 0.0 | — |
| jq | UNSEEN | — | 25.0 | 0.0 | — |

**base06b 均值**：hand=**100.0** / help=**23.2** / plain=**7.9**
**v13 均值**：hand=52.3 / help=**33.7** / plain=15.4

### help vs plain（衡量「读手册本身有没有信息增益」）

| 模型 | 均值增益 | 逐 CLI |
| --- | --- | --- |
| base06b | **+24.3pp** | docker +10 / kubectl +11 / npm +25 / **git +75** / cargo 0 / jq +25 |
| v13 | +5.0pp | docker +30 / kubectl 0 / npm +25 / git +25 / **cargo −50** / jq 0 |

### 四类失败模式（逐条人工核对得出，**这是本实验最有价值的产出**）

| 代号 | 现象 | 实例 |
| --- | --- | --- |
| **F1 掉前缀** | help 里子命令是缩进列出的 → 模型照抄裸命令 | gold=`docker ps` → pred=**`ps`** |
| **F2 复述描述** | 把 help 当散文读，输出说明文字 | gold=`cargo build` → pred=**`Run the build process`** |
| **F3 换工具** | 已有域习惯压过 help（v13 特有） | gold=`git status` → pred=**`gh status`** |
| **F4 编参数** | 漏掉实体/加了不存在的 flag | gold=`npm install express` → pred=**`npm install`** |

> **F1 是最大杀手**：base 的 docker 失败里 6/9 是掉前缀，plain 臂同样 7/10 掉前缀。
> 这说明 0.6B 在长上下文里**倾向于模仿 help 的视觉缩进**，而非遵守"输出完整命令"的指令。

---

## 2. v17b：两阶段「提炼 → 执行」

### 假设
把「理解 3.4KB 散文」与「产出 1 条命令」拆开：
阶段 1 让模型把 help 压成紧凑动作表（**压缩任务**，理论上 0.6B 能做），
阶段 2 用动作表当 schema（与人工 schema 同形态）。

### 读数（exec %）

| CLI | hand | help | **help2** | plain | h2−hand |
| --- | --- | --- | --- | --- | --- |
| docker | 100.0 | 10.0 | **0.0** | 0.0 | −100.0 |
| kubectl | 100.0 | 22.2 | **44.4** | 11.1 | −55.6 |
| npm | 100.0 | 37.5 | **50.0** | 12.5 | −50.0 |
| git | — | 100.0 | 75.0 | 25.0 | — |
| cargo | — | 0.0 | 0.0 | 0.0 | — |
| jq | — | 25.0 | 0.0 | 0.0 | — |

**均值**：base06b `help2`=31.5%（vs help 23.2，**+8.2pp**）
v13 `help2`=15.4%（vs help 33.7，**−18.3pp** ← 提炼步骤整个失效）

### 为什么 v13 反而更差：它是**命令路由器**，不是**手册整理器**

v13 的阶段 1 输出（原样）：
```
<完整命令含子命令> :<一句话说明>
<完整命令含子命令> :<一句话说明>
```
即它**照抄了 prompt 里的格式模板**（因为训练时它的行为模式是「照着 system prompt 的格式输出一条命令」），
完全没做提炼。→ 这是**能力错配**：路由器权重不适合做文档理解。

### base06b 的提炼质量 = help2 成败的唯一决定因素

| CLI | 提炼产物 | 质量 | help2 |
| --- | --- | --- | --- |
| git | `git status : Show the working tree status` | ✅ 带完整前缀 | 75.0 |
| kubectl | `kubectl get : Display one or many resources` | ✅ 带前缀 | **44.4** |
| npm | `npm install <foo> : add the <foo> dependency` | ✅ 带前缀 | **50.0** |
| docker | `run : 创建并运行…` / `ps : 显示…` | ❌ **掉前缀** + 混入写操作 | **0.0** |
| cargo | `cargo:build : 编译当前包` | ❌ **冒号格式**（照抄 help 的 `cargo-build` 节标题） | **0.0** |
| jq | `jq -n : use null as the single input value` | ⚠️ 全是 flag，无「子命令」概念 | 0.0 |

**结论：`help2` 的表现完全被提炼保真度绑定。**
凡提炼带对了 CLI 前缀，help2 就显著好于 help 和 plain；凡提炼错了（docker/cargo），help2 直接归零。

---

## 3. 根因与正解

### 根因
「从非结构化散文里抽出结构化的 (命令, 说明) 对」是**信息抽取**任务。
0.6B 在**长上下文 + 需要遵守格式约束 + 需要判断只读/写**三重压力下，抽取质量不可靠。

**这是容量问题，不是 prompt 问题** —— 证据：base06b 的 kubectl/npm/git 提炼正确，
docker/cargo 提炼错误，同一个 prompt、同一个模型，差别只在于 **help 文本的排版风格**
（docker 是缩进式 `  ps`，cargo 是 `cargo-build` 节标题式）。

### 正解：把提炼挪出模型 → `lycore help-parse`（纯 Rust，确定性）

「提炼」根本不需要神经网络：

```rust
// lycore/src/help_parse.rs — 零依赖、可测、确定性
//
// 输入: `docker --help` 的原始输出
// 输出: [HelpAction { full_cmd: "docker ps", desc: "列出正在运行的容器", readonly: true }]

pub fn parse_help(cli: &str, raw: &str) -> Vec<HelpAction>;
```

三类 help 版式的确定性解析规则（覆盖实测全部 6 个 CLI）：

| 版式 | 实例 | 解析规则 |
| --- | --- | --- |
| **缩进式** | `docker`（`  ps        列出…`） | 剥离前导空白 → 补回 `cli ` 前缀 → 按 2+ 空格切分命令与说明 |
| **节标题式** | `cargo`（`cargo-build  编译当前包`） | 识别 `^<cli>-[a-z]+` 单行 → 转成 `cargo build` |
| **前缀式** | `kubectl`/`npm`/`git` | 行首已含 `cli xxx` → 剥前缀后重新解析子命令+参数 |
| **flag 表** | `jq`（纯 `--long  说明`，无子命令） | 回退：抽「有说明的长选项」当作动作 |
| **逗号列表** | `npm`（`All commands:` 段） | 行内全为合法子命令名 + ≥3 项 + 含逗号 → 逐项展开 |

**只读过滤**：用「动词黑名单」确定性排除写操作，
**不交给模型判断**（这与 Base 臂实证的 reject 0% 完全同源：写/读判定必须是规则，不是概率）。

判定顺序（v17c 定稿，白名单优先但主词黑名单压过）：

1. **短语黑名单**：`config set` / `remote add` / `submodule add` 这类需两词同时出现
2. **锚动词 + 裸位置参数**：`git config user.name x`（写）vs `git config --list`（读）
3. **主词黑名单**：CLI 后第一个 token 是危险动词 → 直接判写（挡住 `npm token list` 被白名单的 `list` 救回）
4. **动词白名单** → 只读
5. **动词黑名单** → 非只读
6. 都没有 → 默认只读

口径是「**是否可能损坏用户数据/影响他人**」，不是「是否产生写 IO」：
`cargo build`（只写自己的 `target/`）算**只读**；`docker rm` 不算。
`init`/`new`/`create`/`bisect` 算写（往磁盘/远端落新东西，不该当无害查询）。

产出的动作表**仍然是注入给模型的那份 schema**，只是**生成者从人换成了确定性代码**。

### v17c：解析器落地实测（6 个 CLI，零人工）

`lycore help-parse --cli <name>` 的真机输出（`--top 8`，白名单重排后）：

| CLI | 解析总数 | 只读数 | top-6 实际内容 |
| --- | --- | --- | --- |
| docker | 57 | 43 | ps / logs / inspect / stats / info / images |
| kubectl | 43 | 32 | get / logs / version / describe / diff / config |
| npm | 68 | 49 | ls / get / version / diff / test / config |
| git | 24 | 11 | status / log / show / branch / diff / bisect |
| cargo | 16 | 7 | test / check / build / doc / run / bench |
| jq | 29 | 29 | --null-input / --raw-input / --slurp / … |

**v17c 修掉的四个隐藏缺陷**（每一个都曾让整条 CLI 失效）：

| 缺陷 | 症状 | 修法 |
| --- | --- | --- |
| ANSI 颜色码 | `cargo --help` 全臂 0%（命令名被 `\x1b[92m` 淹没） | 解析前 `strip_ansi` |
| 别名叫法 | `cargo build, b` 产出不可执行命令 | `split_cmd_and_args` 剥别名 |
| 前缀式被误压 | `kubectl get` 被当别名叫法压成 `kubectl` | **顺序修正**：`normalize_alias` 只用于无前缀行 |
| 键未归一 | `kubectl get` 与 `kubectl get pods` 同时入库 | `push_unique` 先归一空白再算 key |

**新增单测 13 项全绿**（`cargo test --lib help_parse`）：含 5 类版式、只读判定的 31 条断言、
别名/参数保留、逗号列表正反例、去重、Usage 行不混入。

---

## 4. v18 设计（下一步）

```
用户说「用 jq 把 config.json 的 name 抽出来」
  ↓ lycore help-parse（确定性，<1ms，零 GPU）
      读 `jq --help` → 抽出 [jq .name <file>, jq . <file>, ...] → 只读过滤
  ↓ 注入 v13（或 base）
      输出 `jq .name config.json`
  ↓ T1 门（读操作 → 放行）
  ↓ brush 执行
```

**验收标准**（对齐本次实测的上界）：
1. `help-parse` 在 6 个 CLI 上**产出动作表的保真度 ≥ 人工 schema 的 90%**（可逐条 diff 验证）；
2. 用 `help-parse` 产出的 schema 跑评测，**hparse ≥ hand 臂的 90%**（base06b 目标 ≥ 90%）；
3. **cargo / jq 这两个 v17 全臂 0% 的 CLI 必须出分**（它们是「版式难」的代表）。

四臂设计（`a10_v18_helpparse.py`，唯一变量 = 注入内容）：
`hand`（人工 schema，上界）／`help`（原始直灌，v17 失败臂）／
**`hparse`（★ help-parse 产物，零人工+零模型）**／`plain`（无注入，下界）。

**为什么这次能成**：v17b 已证明「前缀带对 → 分数就上」。`help-parse` 就是**保证前缀一定带对**的那个组件。
模型不再需要做信息抽取，只需要做它已经被验证擅长的「对齐 + 补槽位」。

---

## 5. 附带确认的两件事

### 5.1 「读手册」对真陌生 CLI 有效（这是泛用性的正面证据）
`git` 在 base06b 上：`help` 臂 **100.0%**（8/8 全对），`plain` 只有 25.0%。
git 在 v13 训练语料与既往 zs-cli 评测中**均未出现**。
→ **「给手册就能用」在最有希望的那类 CLI（help 排版规整、命令名与意图有强语言关联）上成立。**

### 5.2 失败模式补完了 T1 门的设计依据
F4「编参数」在本次大量出现（`npm install` 漏掉 `express`）。
→ T1 门除了拒危险命令，还需**校验必需参数是否存在**（空参调用是静默失效，用户最难察觉）。
这条与 `t1gate.rs` 现有的「参数不拼 shell」是同一层的加固。

---

## 6. 复现

```bash
cd p2-lyco_ops/cloudstudio
export CS_COOKIE="$(python _getck.py 2>/dev/null | head -1)"
export CS_JPS='https://<spaceKey>--jps.ap-shanghai2.cloudstudio.club'
# v17 裸灌
node cs_exec_train.mjs --file "D:/Code/p2-lyco_ops/cloudstudio/a10_v17_help.py"
# v17b 两阶段
node cs_exec_train.mjs --file "D:/Code/p2-lyco_ops/cloudstudio/a10_v17b_distill.py"
```

⚠️ **务必前台阻塞执行**（`nohup ... &` 在本 shell 会被回收）；单次约 5–8 分钟。
产物：`/workspace/v17_help_results.json`、`/workspace/v17b_results.json`。
