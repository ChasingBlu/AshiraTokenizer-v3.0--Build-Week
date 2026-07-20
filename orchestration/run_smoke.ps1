param(
    [Parameter(Mandatory = $true)]
    [string]$Corpus,
    [string]$Output
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot
$Exe = Join-Path $RepoRoot "target\release\ashira_tokenizer_v2.exe"

if ([string]::IsNullOrWhiteSpace($Output)) {
    $Output = Join-Path $RepoRoot "runs\smoke_out"
}

if (!(Test-Path $Exe)) {
    throw "Trainer binary not found: $Exe. Run orchestration/build_release.ps1 first."
}

New-Item -ItemType Directory -Force -Path $Output | Out-Null

& $Exe `
  --corpus $Corpus `
  --output $Output `
  --vocab-size 320 `
  --min-freq 2 `
  --accelerator cpu

if ($LASTEXITCODE -ne 0) {
    throw "Smoke run failed"
}

Write-Host "[SMOKE] PASS"
