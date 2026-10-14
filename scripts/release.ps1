# Checks, builds and packages the release files into dist\.
#
#   powershell -ExecutionPolicy Bypass -File scripts\release.ps1                  # check + build + package
#   powershell -ExecutionPolicy Bypass -File scripts\release.ps1 -Publish -Notes docs\release-notes\v0.1.0.md
#
# Files: SnapForge-Setup.exe (installer), SnapForge.exe (portable; also what the updater downloads),
# snapforge-cli.exe, SHA256SUMS.txt. Executables are signed only if the local certificate exists
# (scripts\make_cert.ps1); CI builds are unsigned.
param([switch]$Publish, [string]$Notes = "")

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$tag = "v$version"
Write-Host "SnapForge $tag"

function Step($what, [scriptblock]$run) {
    Write-Host "== $what"
    & $run
    if ($LASTEXITCODE -ne 0) { throw "$what failed" }
}

Step "cargo fmt --check" { cargo fmt --all --check }
Step "cargo clippy" { cargo clippy --workspace --all-targets -- -D warnings }
Step "cargo test" { cargo test --workspace }
Step "cargo build --release" { cargo build --release -p snapforge-app -p snapforge-cli }

$dist = Join-Path $root "dist"
if (Test-Path $dist) { Remove-Item -LiteralPath $dist -Recurse -Force }
New-Item -ItemType Directory $dist | Out-Null
Copy-Item target\release\SnapForge.exe (Join-Path $dist "SnapForge.exe")
Copy-Item target\release\snapforge-cli.exe (Join-Path $dist "snapforge-cli.exe")

$hasCert = Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert -ErrorAction SilentlyContinue |
    Where-Object { $_.Subject -eq "CN=ahsoggg" }
if ($hasCert) {
    & (Join-Path $PSScriptRoot "sign.ps1") (Join-Path $dist "SnapForge.exe") (Join-Path $dist "snapforge-cli.exe")
} else {
    Write-Warning "no code-signing certificate on this machine - the files are UNSIGNED"
}
# the installer is the same executable under another name (it detects "Setup" in its file name)
Copy-Item (Join-Path $dist "SnapForge.exe") (Join-Path $dist "SnapForge-Setup.exe")

$lines = foreach ($name in "SnapForge-Setup.exe", "SnapForge.exe", "snapforge-cli.exe") {
    $h = (Get-FileHash (Join-Path $dist $name) -Algorithm SHA256).Hash.ToLower()
    "$h  $name"
}
Set-Content -LiteralPath (Join-Path $dist "SHA256SUMS.txt") -Value ($lines -join "`n") -Encoding ascii
Get-ChildItem $dist | Format-Table Name, Length

if ($Publish) {
    if ($Notes -eq "") { throw "-Notes <file> is required with -Publish" }
    $assets = Get-ChildItem $dist | ForEach-Object { $_.FullName }
    gh release create $tag @assets --title "SnapForge $version" --notes-file $Notes
    if ($LASTEXITCODE -ne 0) { throw "gh release failed" }
}
