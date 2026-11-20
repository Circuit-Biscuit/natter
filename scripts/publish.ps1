# One-time setup: creates the private GitHub repository under the organization and pushes main.
# GitHub Actions then builds the installers and publishes release v<version>.
#
#   powershell -ExecutionPolicy Bypass -File scripts\publish.ps1
#
# Needs the GitHub CLI logged in (gh auth login).
param(
    [string]$Org = "Circuit-Biscuit",
    [string]$Name = "natter"
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)
gh auth status | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Log in first: gh auth login" }
gh repo create "$Org/$Name" --private --source . --remote origin --push `
    --description "Team chat, voice and video - fast like Discord, at home on your own Nextcloud. Rust server + desktop app."
if ($LASTEXITCODE -ne 0) { throw "Creating the repository failed." }
Write-Host ""
Write-Host "Published: https://github.com/$Org/$Name"
Write-Host "Builds: https://github.com/$Org/$Name/actions"
