param(
    [Parameter(Mandatory = $true)]
    [string]$Corpus
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot
$Exe = Join-Path $RepoRoot "target\release\ashira_tokenizer_v2.exe"

if (!(Test-Path $Exe)) {
    throw "Trainer binary not found: $Exe. Run orchestration/build_release.ps1 first."
}

$A = Join-Path $RepoRoot "runs\det_a"
$B = Join-Path $RepoRoot "runs\det_b"
New-Item -ItemType Directory -Force -Path $A | Out-Null
New-Item -ItemType Directory -Force -Path $B | Out-Null

& $Exe --corpus $Corpus --output $A --vocab-size 300 --min-freq 2 --accelerator cpu | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Run A failed" }

& $Exe --corpus $Corpus --output $B --vocab-size 300 --min-freq 2 --accelerator cpu | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Run B failed" }

$mA = (Get-FileHash (Join-Path $A "merges.bin") -Algorithm SHA256).Hash
$mB = (Get-FileHash (Join-Path $B "merges.bin") -Algorithm SHA256).Hash
$vA = (Get-FileHash (Join-Path $A "vocab.bin") -Algorithm SHA256).Hash
$vB = (Get-FileHash (Join-Path $B "vocab.bin") -Algorithm SHA256).Hash

Write-Host "MERGES_EQ=$($mA -eq $mB)"
Write-Host "VOCAB_EQ=$($vA -eq $vB)"
Write-Host "MERGES_SHA=$mA"
Write-Host "VOCAB_SHA=$vA"

if ($mA -ne $mB -or $vA -ne $vB) {
    throw "Determinism check failed"
}

Write-Host "[DETERMINISM] PASS"
