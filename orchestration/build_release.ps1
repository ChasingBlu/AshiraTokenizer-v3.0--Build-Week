$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$Cargo = Get-Command cargo -CommandType Application -ErrorAction Stop

Write-Host "[BUILD] Using cargo command: $($Cargo.Source)"
Push-Location $RepoRoot
try {
    & $Cargo.Source build --release
    if ($LASTEXITCODE -ne 0) {
        throw "Release build failed"
    }
}
finally {
    Pop-Location
}

Write-Host "[BUILD] PASS"
