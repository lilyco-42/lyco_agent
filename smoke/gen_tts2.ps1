param([string]$Dir = "D:\Code\rust\lyco_agent\smoke")
# 已弃用: PowerShell 5.1 按 ANSI 读 UTF-8 无 BOM 文件, 中文文本会念成 mojibake。
# 中文 TTS 生成请用 bash 直接调: D:/app/scoop/apps/python/current/python.exe -m edge_tts ...
# 保留本文件仅作教训记录。
$py = "D:\app\scoop\apps\python\current\python.exe"
$lines = @(
  @{T='等待操作'; F='n1'},
  @{T='首先 cargo new hello world 建立项目'; F='n2'},
  @{T='然后 cd hello world 进入项目目录'; F='n3'},
  @{T='最后 cargo run 运行程序 输出 hello world'; F='n4'}
)
foreach ($l in $lines) {
  & $py -m edge_tts --voice zh-CN-XiaoxiaoNeural --text $l.T --write-media "$Dir\$($l.F).mp3" 2>$null
  if ($LASTEXITCODE -ne 0) { Write-Output "FAIL $($l.F)"; exit 1 }
}
Write-Output "edge-tts done"
