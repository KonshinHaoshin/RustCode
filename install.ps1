# RustCode release installer (Windows PowerShell)

param(
    [string]$InstallPath = "$env:LOCALAPPDATA\rustcode",
    [switch]$AddToPath = $false
)

$repo = "KonshinHaoshin/claude-code-rust"
$targetDir = Join-Path $InstallPath "bin"
$exePath = Join-Path $targetDir "rustcode.exe"

New-Item -ItemType Directory -Path $targetDir -Force | Out-Null

try {
    $release = Invoke-WebRequest -Uri "https://api.github.com/repos/$repo/releases/latest" -UseBasicParsing
    $version = (($release.Content | ConvertFrom-Json).tag_name)
} catch {
    $version = "v0.1.0"
}

$downloadUrl = "https://github.com/$repo/releases/download/$version/rustcode-windows-x86_64.exe"

Write-Host "Downloading RustCode from $downloadUrl"
Invoke-WebRequest -Uri $downloadUrl -OutFile $exePath -UseBasicParsing

if ($AddToPath) {
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($userPath -notlike "*$targetDir*") {
        [Environment]::SetEnvironmentVariable("Path", "$targetDir;$userPath", "User")
        $env:Path = "$targetDir;$env:Path"
    }
}

Write-Host
Write-Host "Installed: $exePath" -ForegroundColor Green
Write-Host "Run: rustcode --help"
