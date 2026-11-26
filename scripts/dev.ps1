# Run a local Laklak server that also serves the web build of the app.
#   powershell -File scripts\dev.ps1            (http://localhost:8080)
# Build the app first: cd crates\app; trunk build --release
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
if (-not $env:LAKLAK_DATA) { $env:LAKLAK_DATA = "$root\laklak-data" }
if (-not $env:LAKLAK_BIND) { $env:LAKLAK_BIND = "127.0.0.1:8080" }
if (-not $env:LAKLAK_PUBLIC_IP) { $env:LAKLAK_PUBLIC_IP = "127.0.0.1" }
if (-not $env:LAKLAK_REGISTRATION) { $env:LAKLAK_REGISTRATION = "open" }
$env:LAKLAK_WEB_DIR = "$root\crates\app\dist"
if (-not $env:RUST_LOG) { $env:RUST_LOG = "laklak_server=info" }
cargo run --release -p laklak-server
