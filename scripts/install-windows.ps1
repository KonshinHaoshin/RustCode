#!/usr/bin/env pwsh

param(
    [string]$InstallDir = "$env:LOCALAPPDATA\rustcode"
)

$sourceDir = Resolve-Path "$PSScriptRoot\.."
$binDir = Join-Path $InstallDir "bin"

Write-Host "==========================================="
Write-Host "RustCode - Windows Source Install"
Write-Host "==========================================="
Write-Host "Install dir: $InstallDir"
Write-Host

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "Error: cargo is required. Install Rust from https://rustup.rs/" -ForegroundColor Red
    exit 1
}

New-Item -ItemType Directory -Path $binDir -Force | Out-Null

Set-Location $sourceDir
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Host "Error: cargo build --release failed" -ForegroundColor Red
    exit 1
}

Copy-Item "$sourceDir\target\release\rustcode.exe" "$binDir\rustcode.exe" -Force

Write-Host
Write-Host "Installed: $binDir\rustcode.exe" -ForegroundColor Green
Write-Host "Add to PATH if needed:"
Write-Host "  $binDir"
Write-Host
Write-Host "Quick start:"
Write-Host "  rustcode --help"
Write-Host "  rustcode config set provider deepseek"
Write-Host "  rustcode config set api_key ""your-api-key"""
