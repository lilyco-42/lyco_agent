Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
$s.SelectVoice(($s.GetInstalledVoices() | Where-Object { $_.VoiceInfo.Culture.Name -like 'zh*' } | Select-Object -First 1).VoiceInfo.Name)
$dir = "D:\Code\rust\lyco_agent\smoke"
$lines = @(
  @{T='等待操作'; F='t1.wav'},
  @{T='首先 cargo new hello world 建立项目'; F='t2.wav'},
  @{T='然后 cd hello world 进入项目目录'; F='t3.wav'},
  @{T='最后 cargo run 运行程序 输出 hello world'; F='t4.wav'}
)
foreach ($l in $lines) {
  $s.SetOutputToWaveFile("$dir\$($l.F)")
  $s.Speak($l.T)
}
$s.Dispose()
Write-Output "TTS done"
