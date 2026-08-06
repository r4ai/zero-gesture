param(
    [Parameter(Mandatory = $true)]
    [string]$Path
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$thumbprint = $env:P05C_CERT_THUMBPRINT
if ([string]::IsNullOrWhiteSpace($thumbprint)) {
    throw "P05C_CERT_THUMBPRINT must identify the disposable acceptance signer."
}

$certificate = Get-Item -LiteralPath "Cert:\CurrentUser\My\$thumbprint"
$signature = Set-AuthenticodeSignature `
    -LiteralPath $Path `
    -Certificate $certificate `
    -HashAlgorithm SHA256

if (
    $null -eq $signature.SignerCertificate -or
    $signature.SignerCertificate.Thumbprint -ine $thumbprint
) {
    throw "Authenticode signing did not use the disposable acceptance identity: $Path"
}
