# lyco 分发管线（小白版）

目标：像 codex / claude code 桌面端一样「下载 → 就能用」，唯一区别是
lyco_agent 的模型是本地/自托管（lyco_router），记忆包（mpkg）本地拉取复用，
**不需要任何 API token，也不需要 python / node / uv 前置知识**。

## 组成

| 文件 | 作用 |
|---|---|
| `bootstrap.ps1` | 小白一键脚本：下载 lycore.exe → 写用户 PATH → 展开 mpkg 包 → 冒烟 |
| `index.html` | 分发点落地页 |
| （CI 产物）`lycore.exe` | Rust 单文件二进制，唯一运行时是 Windows 本身 |
| （CI 产物）`packs.zip` | `packs/` 目录打 zip，随脚本展开 |

## 分发点

`https://lain42.top/lyco-dl/`（pingap directory 插件 → `/var/www/lyco-dl`）

小白入口（PowerShell 一行）：

```
powershell -ExecutionPolicy ByPass -c "irm https://lain42.top/lyco-dl/bootstrap.ps1 | iex"
```

## 服务器侧（lain42.top，root@）

- pingap 配置：`/etc/pingap.toml` 里 `[locations.lyco_dl]` + `[plugins.lyco_dir]`，
  `lyco_dl` 在 locations 数组中排在 `lyco` **之前**（防前缀误吞）；改完 autoreload 生效。
- 文件根：`/var/www/lyco-dl/`（lycore.exe、packs.zip、bootstrap.ps1、index.html）。
- 备份：`/etc/pingap.toml.bak-*`（改动前的时间戳备份，回滚直接覆盖还原）。

## 更新流程（发新版本时）

1. CI（build.yml）跑 `Build & Test` → 下载 artifact `lycore-x86_64-pc-windows-msvc`
2. `packs/` 打 zip（保留顶层 `lab-python-env/` 目录结构）
3. scp 上服务器 `/var/www/lyco-dl/`（覆盖 lycore.exe / packs.zip）
4. 冒烟：`curl -fsSL https://lain42.top/lyco-dl/lycore.exe -o /dev/null -w '%{http_code} %{size_download}\n'`
