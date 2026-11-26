# Android release in one go: builds the signed universal APK (GitHub, sideloading, self-hosters) and the app
# bundle (Google Play), attaches the APK to the GitHub draft release, and uploads the bundle to Google Play.
#
#   powershell -ExecutionPolicy Bypass -File scripts\android-release.ps1                  # Play internal testing
#   powershell -ExecutionPolicy Bypass -File scripts\android-release.ps1 -SkipPlay        # APK only
#   ... -Track alpha   ... -Status draft   ... -SkipGitHub   ... -SkipBuild (reuse target\android\)
#
# Run it right after pushing a version bump (scripts\bump.ps1), while the Release workflow still has the draft
# open: the APK must be on the draft before it's published, so it lands in SHA256SUMS too.
#
# Local settings, never in git: fastlane\.env with KEY=VALUE lines, read here and by fastlane:
#   ANDROID_HOME, NDK_HOME, JAVA_HOME     Android SDK, NDK (27+) and a JDK 17+
#   SUPPLY_JSON_KEY=<key file>            a Play service account key, or, without key files:
#   PLAY_SERVICE_ACCOUNT=<email>          a service account to impersonate with your gcloud sign-in
#                                         (gcloud auth application-default login)
# Signing: crates\desktop\gen\android\keystore.properties (see docs/DEPLOY.md, Releasing).
param(
    [string]$Track = "internal",
    [string]$Status = "completed",
    [switch]$SkipGitHub,
    [switch]$SkipPlay,
    [switch]$SkipBuild
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

function Invoke-Native([string]$What, [scriptblock]$Cmd) {
    & $Cmd
    if ($LASTEXITCODE) { throw "$What failed (exit code $LASTEXITCODE)" }
}

if (Test-Path fastlane\.env) {
    foreach ($line in Get-Content fastlane\.env) {
        $name, $value = $line -split '=', 2
        $name = $name.Trim()
        if ($null -ne $value -and $name -and -not $name.StartsWith('#')) { Set-Item "env:$name" $value.Trim().Trim('"') }
    }
}
if (-not $env:SUPPLY_JSON_KEY -and -not $env:PLAY_SERVICE_ACCOUNT -and -not $SkipPlay) {
    throw "Set SUPPLY_JSON_KEY or PLAY_SERVICE_ACCOUNT in fastlane\.env (see the top of this script)"
}
$env:LC_ALL = "en_US.UTF-8"; $env:LANG = "en_US.UTF-8"   # fastlane insists

$version = [regex]::Match((Get-Content Cargo.toml -Raw), '(?m)^version = "([^"]+)"').Groups[1].Value
$v = $version.Split('.') | ForEach-Object { [int]$_ }
$versionCode = $v[0] * 1000000 + $v[1] * 1000 + $v[2]   # Tauri's own formula, so the APK and the bundle agree
$out = "target\android"
$apk = "$out\Laklak_$version.apk"
$aab = "$out\Laklak_$version.aab"
Write-Host "Laklak $version (versionCode $versionCode)"

if (-not $SkipBuild) {
    foreach ($name in "ANDROID_HOME", "NDK_HOME", "JAVA_HOME") {
        if (-not (Test-Path ([Environment]::GetEnvironmentVariable($name) + ""))) { throw "Set $name (in fastlane\.env)" }
    }
    if (-not (Test-Path crates\desktop\gen\android\keystore.properties)) { throw "No keystore.properties: the build would be unsigned" }
    # The UI first: the Android build bundles crates\app\dist as it is (tauri.android.conf.json skips trunk).
    Push-Location crates\app
    try { Invoke-Native "trunk build" { trunk build --release } } finally { Pop-Location }
    Push-Location crates\desktop
    try { Invoke-Native "Android build" { cargo tauri android build --apk --aab --target aarch64 --target armv7 } }
    finally {
        Pop-Location
        # The build rewrites this generated file with Android-only entries; keep the committed one.
        & { $ErrorActionPreference = "Continue"; git checkout -- crates/desktop/gen/schemas/acl-manifests.json 2>$null }
    }
    New-Item -ItemType Directory -Force $out | Out-Null
    $built = "crates\desktop\gen\android\app\build\outputs"
    Copy-Item "$built\apk\universal\release\app-universal-release.apk" $apk
    Copy-Item "$built\bundle\universalRelease\app-universal-release.aab" $aab
}
if (-not (Test-Path $apk) -or -not (Test-Path $aab)) { throw "Missing $apk or $aab" }

if (-not $SkipGitHub) {
    $draft = & { $ErrorActionPreference = "Continue"; gh release view "v$version" --json isDraft --jq .isDraft 2>$null }
    if ($draft -eq "true") {
        Invoke-Native "gh release upload" { gh release upload "v$version" $apk --clobber }
        Write-Host "APK attached to the draft v$version."
    } elseif ($draft -eq "false") {
        Write-Warning "v$version is already published, so the APK isn't attached (its checksum would be missing). If it must be: gh release upload v$version $apk"
    } else {
        Write-Warning "No GitHub release v$version yet: push the version bump first, then run this again with -SkipBuild."
    }
}

if (-not $SkipPlay) {
    # "What's new" on Google Play (500 characters at most): the headline of each "New"/"Changed"/"Fixed" item.
    $section = [regex]::Match((Get-Content CHANGELOG.md -Raw -Encoding UTF8), "(?s)## \[$([regex]::Escape($version))\][^\n]*\n(.*?)(?=\n## \[|$)").Groups[1].Value
    $more = "`nAll changes: github.com/Circuit-Biscuit/laklak/releases"
    $notes = ""
    foreach ($m in [regex]::Matches($section, '(?m)^- \*\*(.+?)\*\*')) {
        $item = [char]0x2022 + " " + ($m.Groups[1].Value -replace '[`*_]', '').TrimEnd('.', ':') + "`n"
        if (($notes + $item + $more).Length -gt 500) { break }
        $notes += $item
    }
    $notes = ($notes + $more.Trim()).Trim()
    $changelogs = "fastlane\metadata\android\en-US\changelogs"
    New-Item -ItemType Directory -Force $changelogs | Out-Null
    [IO.File]::WriteAllText("$PWD\$changelogs\$versionCode.txt", $notes)
    Invoke-Native "fastlane" { fastlane android play "aab:$aab" "track:$Track" "status:$Status" }   # signs in by itself (Fastfile)
    Write-Host "Bundle $versionCode is on Google Play's $Track track ($Status)."
}
