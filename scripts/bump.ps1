# Set a new version everywhere and commit it. Pushing to main then publishes the release.
#   powershell -ExecutionPolicy Bypass -File scripts\bump.ps1 0.2.0
param([Parameter(Mandatory = $true)][string]$Version)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Use a version like 0.2.0" }
if (-not (Select-String -Path CHANGELOG.md -Pattern "## \[$Version\]" -Quiet)) {
    throw "Add a '## [$Version]' section to CHANGELOG.md first - it becomes the release notes."
}
$toml = Get-Content Cargo.toml -Raw
$toml = $toml -replace '(\[workspace\.package\]\s*\r?\nversion = )"[^"]+"', "`$1`"$Version`""
[IO.File]::WriteAllText("$PWD\Cargo.toml", $toml)
cargo update --workspace --offline | Out-Null
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "Release $Version"
Write-Host "Now: git push origin main  (the Release workflow builds and publishes v$Version)"
