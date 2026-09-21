# help-parse 盲区探测器械（L1 外部 CLI 池）

## 为什么存在

`lycore help-parse` 的职责是**把任意 CLI 的 `--help` 确定性解析成动作表**（v17 正解：提炼不用模型）。
它的质量只能用**外部陌生 CLI** 检验 —— 池内 6 个 CLI（docker/git/cargo/npm/kubectl/jq）
是「按手上有什么样本」攒的，而攒的人一直用同一类 CLI，于是**系统性漏掉他不知道的那一类**。

同一失效模式已实测发生**两次**：

| 发现 | 池内是否显形 | 原因 |
|---|---|---|
| clap long-help 版式（描述在下一行） | ❌ 完全不显形 | 池内 6 个 CLI 无一是 clap derive 输出 |
| flag 值占位符丢失（`--warmup <NUM>` → `--warmup`） | ❌ 完全不显形 | 池内 flag **全是裸 flag**，本来就不带值 |

**结论：池内回归「全绿」只证明「对池内那类 CLI 是对的」，不证明泛用性。**

## 三个脚本

| 脚本 | 作用 |
|---|---|
| `collect.py` | 采集全部目标 CLI 的 `--help` 到 `fixtures/<cli>.txt`；`--parse` 再跑一遍 help-parse |
| `regress.py` | **固定口径回归比对**：`--baseline` 建基线，无参数则比对并标红池内回归 |
| `diffcheck.py` | **逐字 diff**：`save-before` / `save-after` / `compare`，比的是 `full_cmd`+`desc`+`example` 全文，不是条数 |

### 固定口径（必须统一，否则数字不可比）

```bash
lycore help-parse --cli <name> --help-text <file> --top 9999 --include-writes --json
```

- `--include-writes`：不过滤写操作（否则只看到只读子集，条数对不上）
- `--top 9999`：不截断
- 主数字取 `total_parsed`（含写操作的全部解析动作）

### 四层池

| 层 | 内容 | 计分 |
|---|---|---|
| **L0** | 现有池 docker/git/cargo/npm/kubectl/jq | **不计分**（已被研究过，会污染） |
| **L1** | 外部陌生：fd/rg/hyperfine/zoxide/bat/starship/oha | **主分数** |
| **L2** | 自研 lilyco（`lfiles` 等） | **不计分**（⚠️ 作者偏差：help 是我们自己写的） |
| **L3** | 冷门/异形（待补） | 计分 |

## 用法

```bash
PY=C:/Users/liuqi/.workbuddy/binaries/python/versions/3.13.12/python.exe
cd scripts/helpfixtures

$PY collect.py              # 采集 help 样本（没装的 CLI 会标 not-installed）
$PY collect.py --parse      # 看当前解析条数
$PY regress.py --baseline   # 建基线（改动前跑一次）
$PY regress.py              # 改动后比对（池内回归会 exit 1）

# 逐字 diff（更严格，推荐每次改 help_parse 都跑）
$PY diffcheck.py save-before   # 改前（需先 git stash + build）
$PY diffcheck.py save-after    # 改后
$PY diffcheck.py compare       # 逐字比对，池内差异数必须 = 0
```

## 判据（两条硬线）

1. **池内逐字差异必须 = 0** —— 补盲区不得误伤已正确的旧形态。
2. **L1 只能变好** —— 条数增加或内容修正（占位符补齐 / desc 去污染）。

违反第 1 条的改动一律回退重做。
