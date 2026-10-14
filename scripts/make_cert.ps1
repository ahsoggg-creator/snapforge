# Creates (once) a self-signed code-signing certificate "ahsoggg" in YOUR certificate store and exports its
# public part to certs\ahsoggg.cer (safe to commit). The private key stays in the store; a password-protected
# backup is written only if you ask for one:
#
#   powershell -ExecutionPolicy Bypass -File scripts\make_cert.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\make_cert.ps1 -BackupPfx D:\safe\ahsoggg.pfx
#
# A self-signed certificate proves "signed by the same author", not "trusted by Windows": SmartScreen keeps
# warning until the certificate is trusted on that machine (scripts\trust_cert.ps1).
param([string]$BackupPfx = "")

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$subject = "CN=ahsoggg"

$cert = Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert |
    Where-Object { $_.Subject -eq $subject -and $_.NotAfter -gt (Get-Date).AddDays(30) } |
    Sort-Object NotAfter -Descending | Select-Object -First 1
if (-not $cert) {
    $cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject $subject -KeyAlgorithm RSA -KeyLength 3072 `
        -HashAlgorithm SHA256 -KeyExportPolicy Exportable -NotAfter (Get-Date).AddYears(5) `
        -CertStoreLocation Cert:\CurrentUser\My
    Write-Host "Created certificate $($cert.Thumbprint)"
} else {
    Write-Host "Using the existing certificate $($cert.Thumbprint)"
}

New-Item -ItemType Directory -Force (Join-Path $root "certs") | Out-Null
$cer = Join-Path $root "certs\ahsoggg.cer"
Export-Certificate -Cert $cert -FilePath $cer | Out-Null
Write-Host "Public certificate: $cer"
Write-Host ("SHA-256 fingerprint: " + (Get-FileHash $cer -Algorithm SHA256).Hash)

if ($BackupPfx -ne "") {
    $pw = Read-Host "Password for the .pfx backup" -AsSecureString
    Export-PfxCertificate -Cert $cert -FilePath $BackupPfx -Password $pw | Out-Null
    Write-Host "Private key backup: $BackupPfx (keep it secret; *.pfx is git-ignored)"
}
