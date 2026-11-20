# Run a local Natter server that also serves the web build of the app.
#   powershell -File scripts\dev.ps1            (http://localhost:8080)
# Build the app first: cd crates\app; trunk build --release
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
if (-not $env:NATTER_DATA) { $env:NATTER_DATA = "$root\natter-data" }
if (-not $env:NATTER_BIND) { $env:NATTER_BIND = "127.0.0.1:8080" }
if (-not $env:NATTER_PUBLIC_IP) { $env:NATTER_PUBLIC_IP = "127.0.0.1" }
if (-not $env:NATTER_REGISTRATION) { $env:NATTER_REGISTRATION = "open" }
$env:NATTER_WEB_DIR = "$root\crates\app\dist"
if (-not $env:RUST_LOG) { $env:RUST_LOG = "natter_server=info" }
cargo run --release -p natter-server
