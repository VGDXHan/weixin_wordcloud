# 以管理员身份清理旧的 dev 进程并重新启动 tauri dev
$ErrorActionPreference = 'SilentlyContinue'

Write-Host '=== 清理旧的 dev 进程 ===' -ForegroundColor Cyan
taskkill /F /IM weixin-wordcloud.exe 2>$null | Out-Null

# 杀掉与 tauri/vite 相关的 node / cargo 进程（旧的 dev 树）
Get-CimInstance Win32_Process |
  Where-Object { $_.CommandLine -match 'tauri|vite' -and $_.Name -match 'node|cargo|weixin' } |
  ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }

# 释放 1420 端口
$owner = (Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue).OwningProcess
if ($owner) { Stop-Process -Id $owner -Force -ErrorAction SilentlyContinue }

Start-Sleep -Seconds 1

Set-Location 'D:\Mainpage\weixin_wordcloude'
Write-Host '=== 启动 tauri dev（管理员）===' -ForegroundColor Green
npm run tauri dev
