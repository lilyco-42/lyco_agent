# lab-python-env —— 机房/重置环境一键自举

## 解决什么问题

机房每次开机全被重置，装 uv → 配镜像 → 建工程全是重复劳动。本包把这套流程
固化成 **可回放、可验证、content-addressed** 的 mpkg 记忆包，一条命令跑完，
退出码说话（0 = 环境就绪，2 = 某步失败，1 = 前置缺失）。

## 用法（机房新机器上）

```
lycore mpkg-verify packs/lab-python-env
```

就这一条。它按顺序幂等执行：

| step | 行为 | 幂等策略 |
|---|---|---|
| 1 | 装 uv（astral 官方脚本） | `command -v uv` 已在 PATH 则跳过 |
| 2 | 配 aliyun PyPI 镜像 + copy 链接模式（写 `%APPDATA%\uv\uv.toml`） | 文件已存在则**不覆盖**个人配置 |
| 3 | `{{work}}/pyqt6-app` 里 `uv init --bare` | 每次回放在全新临时工作区，无冲突 |
| 4 | `uv add pyqt6 --link-mode=copy` | 同上 |

verify 全过 = 环境**确定性地**就绪，不是"看起来装好了"。

## 素材说明

- `uv.toml` —— 打包携带的镜像配置模板（aliyun + `link-mode = "copy"`，
  机房盘符常禁 symlink/junction，默认硬链接会炸）。
- 需要别的包（numpy/requests/...）？改 step 4 的 `uv add` 参数即可，
  改完 `mpkg-id` 重定身 —— 内容寻址，改一个字节就是新包，不会污染旧包。

## 已知边界

- `requirements.os = ["windows"]`：step 1/2 用了 `%APPDATA%` 与 PowerShell
  一行流，仅 Windows 机房。Linux 机房把这两步换成 sh 等价物即可。
- step 4 需要一个 Python 解释器：教学机房一般自带；裸机首次 `uv add` 会
  自动下载托管 CPython（走 GitHub，若被墙可设 `UV_PYTHON_INSTALL_MIRROR`）。
- 首次装 uv 后的 PATH：每步显式 `export PATH="$HOME/.local/bin:$PATH"`，
  不依赖安装脚本改的注册表 PATH（当前进程不生效）。
