$ErrorActionPreference = 'Stop'

$running = Get-Process -Name 'rustcode-tauri' -ErrorAction SilentlyContinue
if ($running) {
  $running | Stop-Process -Force
  Start-Sleep -Milliseconds 400
}

Set-Location (Join-Path $PSScriptRoot '..\..\src-tauri')
& ..\gui\node_modules\.bin\tauri.CMD dev
