# Build the Windows release exe: target/release/itda.exe
#   pwsh scripts/windows-release.ps1
# Keeps the build machine's paths (user folder, user name) out of the binary: panic messages
# and dependency file names would otherwise carry them. The last matching prefix wins.
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot)
$env:CARGO_ENCODED_RUSTFLAGS = "--remap-path-prefix=$env:USERPROFILE=~`u{1f}--remap-path-prefix=$PWD=itda"
cargo build --release
if ($LASTEXITCODE) { exit $LASTEXITCODE }
$exe = Get-Item target/release/itda.exe
'{0} ({1:N2} MB)' -f $exe.FullName, ($exe.Length / 1MB)
