# Verify a signed release before generating SHA256SUMS.txt.
param(
    [Parameter(Mandatory = $true)]
    [string]$Publisher,
    [string]$Exe = 'target/release/itda.exe'
)

$ErrorActionPreference = 'Stop'
$path = (Resolve-Path -LiteralPath $Exe).Path
$signature = Get-AuthenticodeSignature -LiteralPath $path
if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
    throw "Itda Authenticode signature is $($signature.Status): $($signature.StatusMessage)"
}
if ($signature.SignatureType -ne 'Authenticode') {
    throw 'The executable needs an embedded Authenticode signature'
}
$actual = $signature.SignerCertificate.GetNameInfo(
    [System.Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false
)
if ($actual -cne $Publisher) {
    throw "Unexpected Authenticode publisher '$actual'"
}
if ($null -eq $signature.TimeStamperCertificate) {
    throw 'The signature needs a trusted timestamp'
}

"Verified embedded Authenticode signature and timestamp for $actual"
