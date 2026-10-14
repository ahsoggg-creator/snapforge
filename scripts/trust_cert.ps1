# Makes THIS computer trust the "ahsoggg" certificate so signed SnapForge builds show a known publisher.
# Do it only if you trust the author: the certificate goes into your own Trusted Root and Trusted Publishers
# stores (Windows asks you to confirm the root). Compare the printed fingerprint with the one in the README.
#
#   powershell -ExecutionPolicy Bypass -File scripts\trust_cert.ps1 [-Cer certs\ahsoggg.cer]
param([string]$Cer = (Join-Path (Split-Path -Parent $PSScriptRoot) "certs\ahsoggg.cer"))

$ErrorActionPreference = "Stop"
$c = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 (Resolve-Path $Cer)
Write-Host "Certificate: $($c.Subject)"
Write-Host "SHA-256 fingerprint: $((Get-FileHash $Cer -Algorithm SHA256).Hash)"
Import-Certificate -FilePath $Cer -CertStoreLocation Cert:\CurrentUser\Root | Out-Null
Import-Certificate -FilePath $Cer -CertStoreLocation Cert:\CurrentUser\TrustedPublisher | Out-Null
Write-Host "Done."
