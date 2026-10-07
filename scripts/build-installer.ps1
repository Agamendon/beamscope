# Builds installer\BeamScope-Setup.exe (Velopack, per-user install, no admin needed).
#
# Needs on the BUILD machine only: Rust, Node.js, .NET 8 SDK and vpk:
#   winget install Microsoft.DotNet.SDK.8
#   dotnet tool install -g vpk
#
# usage:  powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$app = Join-Path $root "beamscope"
$tauri = Join-Path $app "src-tauri"
$stage = Join-Path $root "build\pack"
$out = Join-Path $root "build\releases"
$env:PATH += ";$env:USERPROFILE\.dotnet\tools;C:\Program Files\dotnet"

$version = (Get-Content (Join-Path $tauri "tauri.conf.json") -Raw | ConvertFrom-Json).version
Write-Host "== BeamScope $version"

# 1. Release exe with the web UI embedded (no bundle: Velopack does the packaging).
Push-Location $app
try {
  npm install --no-audit --no-fund
  if ($LASTEXITCODE) { throw "npm install failed" }
  npm run tauri build -- --no-bundle
  if ($LASTEXITCODE) { throw "tauri build failed" }
} finally { Pop-Location }

# 2. Stage the files that get installed.
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item (Join-Path $tauri "target\release\beamscope.exe") $stage
# FTDI's D3XX user-mode DLL for the BladeCam2. Shipping it next to the exe means the camera works
# even if DataRay's program folder is missing; the BladeCam2 USB driver itself still has to be
# installed (it comes with DataRay).
$ftd = Join-Path ${env:ProgramFiles} "DataRay\FTD3XX.dll"
if (Test-Path $ftd) { Copy-Item $ftd (Join-Path $stage "ftd3xx.dll") }
else { Write-Warning "FTD3XX.dll not found in DataRay's folder; installer will rely on the target PC's copy" }

# 3. Pack. We ship only the full Setup.exe (no update feed), so start from a clean output folder;
#    otherwise vpk refuses to re-pack a version it has already packed.
Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
#     --framework webview2 installs the WebView2 runtime on PCs that lack it (rare on Win 11).
vpk pack `
  --packId BeamScope `
  --runtime win-x64 `
  --packVersion $version `
  --packTitle BeamScope `
  --packAuthors "BeamScope" `
  --packDir $stage `
  --mainExe beamscope.exe `
  --icon (Join-Path $tauri "icons\icon.ico") `
  --framework webview2 `
  --shortcuts "Desktop,StartMenuRoot" `
  --outputDir $out
if ($LASTEXITCODE) { throw "vpk pack failed" }

# 4. The one file people need, committed to the repo.
$dest = Join-Path $root "installer"
New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item (Join-Path $out "BeamScope-win-Setup.exe") (Join-Path $dest "BeamScope-Setup.exe") -Force
Write-Host "== installer\BeamScope-Setup.exe ($([math]::Round((Get-Item (Join-Path $dest 'BeamScope-Setup.exe')).Length / 1MB, 1)) MB)"
