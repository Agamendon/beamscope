# Builds the BeamScope installer (Velopack: per-user install, no admin needed).
# GitHub Actions runs this on every version tag (see .github/workflows/build.yml); you normally
# don't need to run it yourself.
#
# Local use needs Rust, Node.js 20+, the .NET 8 SDK and vpk:
#   winget install Microsoft.DotNet.SDK.8
#   dotnet tool install -g vpk --version 1.2.161
#
# usage:  powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1 [-Version 1.2.3]
#   -Version  overrides the version from beamscope\src-tauri\tauri.conf.json (CI passes the tag).
# Output:  build\releases\BeamScope-win-Setup.exe (+ portable zip and Velopack update files)
param([string]$Version)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$app = Join-Path $root "beamscope"
$tauri = Join-Path $app "src-tauri"
$stage = Join-Path $root "build\pack"
$out = Join-Path $root "build\releases"
$env:PATH += ";$env:USERPROFILE\.dotnet\tools;C:\Program Files\dotnet"

$confPath = Join-Path $tauri "tauri.conf.json"
if ($Version) {
  # Stamp the version into the app (Tauri reads it from tauri.conf.json at build time).
  $conf = Get-Content $confPath -Raw | ConvertFrom-Json
  $conf.version = $Version
  # No byte-order mark: Windows PowerShell's "utf8" adds one, which JSON parsers may reject.
  [IO.File]::WriteAllText($confPath, ($conf | ConvertTo-Json -Depth 32), (New-Object Text.UTF8Encoding $false))
} else {
  $Version = (Get-Content $confPath -Raw | ConvertFrom-Json).version
}
Write-Host "== BeamScope $Version"

# 1. Release exe with the web UI embedded (no bundle: Velopack does the packaging).
Push-Location $app
try {
  if ($env:CI) { npm ci --no-audit --no-fund } else { npm install --no-audit --no-fund }
  if ($LASTEXITCODE) { throw "npm install failed" }
  npm run tauri build -- --no-bundle
  if ($LASTEXITCODE) { throw "tauri build failed" }
} finally { Pop-Location }

# 2. Stage the files that get installed.
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item (Join-Path $tauri "target\release\beamscope.exe") $stage
# FTDI's D3XX DLL for the BladeCam2 (see vendor\ftdi\README.md).
Copy-Item (Join-Path $root "vendor\ftdi\FTD3XX.dll") (Join-Path $stage "ftd3xx.dll")

# 3. Pack. Each build is a full standalone installer, so start from a clean output folder
#    (vpk also refuses to re-pack a version it has already packed).
#    --framework webview2 installs the WebView2 runtime on PCs that lack it (rare on Win 11).
Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
vpk pack `
  --packId BeamScope `
  --runtime win-x64 `
  --packVersion $Version `
  --packTitle BeamScope `
  --packAuthors "BeamScope" `
  --packDir $stage `
  --mainExe beamscope.exe `
  --icon (Join-Path $tauri "icons\icon.ico") `
  --framework webview2 `
  --shortcuts "Desktop,StartMenuRoot" `
  --outputDir $out
if ($LASTEXITCODE) { throw "vpk pack failed" }

$setup = Join-Path $out "BeamScope-win-Setup.exe"
Write-Host "== $setup ($([math]::Round((Get-Item $setup).Length / 1MB, 1)) MB)"
