# ============================================================
#  lyco 一键安装 —— lycore.exe + mpkg 包，零依赖（不需要 python / node / uv）
#  用法（PowerShell，普通用户权限即可）:
#    powershell -ExecutionPolicy ByPass -c "irm https://lain42.top/lyco-dl/bootstrap.ps1 | iex"
# ============================================================
$ErrorActionPreference = "Stop"
$Base = "https://lain42.top/lyco-dl"
$BinDir = "$env:LOCALAPPDATA\lyco\bin"
$PackDir = "$env:LOCALAPPDATA\lyco\packs"

Write-Host "== lyco 一键安装 ==" -ForegroundColor Cyan
Write-Host "目标: $BinDir"

# 1) lycore.exe（Rust 单文件，无任何运行时依赖）
New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
Write-Host "[1/3] 下载 lycore.exe ..."
curl.exe -fsSL "$Base/lycore.exe" -o "$BinDir\lycore.exe"
if ($LASTEXITCODE -ne 0) { throw "lycore.exe 下载失败，请检查网络" }

# 2) mpkg 记忆包（机房自举等）
Write-Host "[2/3] 下载 mpkg 包 ..."
New-Item -ItemType Directory -Force -Path $PackDir | Out-Null
curl.exe -fsSL "$Base/packs.zip" -o "$env:TEMP\lyco-packs.zip"
if ($LASTEXITCODE -eq 0) {
    Expand-Archive -Force -Path "$env:TEMP\lyco-packs.zip" -DestinationPath $PackDir
    Remove-Item "$env:TEMP\lyco-packs.zip" -Force
}

# 3) 用户级 PATH 永久生效
Write-Host "[3/3] 写入用户 PATH ..."
$UserPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($UserPath -notlike "*lyco\bin*") {
    [Environment]::SetEnvironmentVariable("PATH", "$BinDir;$UserPath", "User")
}
$env:PATH = "$BinDir;$env:PATH"

# 冒烟：能定身 = 装好了
& lycore mpkg-id "$PackDir\lab-python-env"
if ($LASTEXITCODE -eq 0) {
    Write-Host ""
    Write-Host "✅ lycore 就位（新开终端可直接用 lycore 命令）" -ForegroundColor Green
    Write-Host "   机房自举:  lycore mpkg-verify $PackDir\lab-python-env"
    Write-Host "   查看命令:  lycore （无参数）"
} else {
    Write-Warning "安装后冒烟未过 —— 请把上面的报错原样反馈"
}
