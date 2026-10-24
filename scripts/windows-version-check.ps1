# Check the built Windows exe's resources: the version Windows shows (and that remote setup
# reads with the same expression to refuse a downgrade), plus the icon and manifest.
#   pwsh scripts/windows-version-check.ps1 [target/release/itda.exe]
param([string]$Exe = 'target/release/itda.exe')
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot)
$exe = (Resolve-Path -LiteralPath $Exe).Path
$cargo = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE) { throw 'cargo metadata failed' }
$version = ($cargo.packages | Where-Object name -eq 'itda').version
if (-not $version) { throw 'No itda package version' }

# remote_setup.rs reads exactly this on a Windows target.
$product = [string](Get-Item -LiteralPath $exe).VersionInfo.ProductVersion
if ($product -ne $version) { throw "ProductVersion '$product' is not the package version '$version'" }
$info = (Get-Item -LiteralPath $exe).VersionInfo
if ($info.FileVersion -ne $version) { throw "FileVersion '$($info.FileVersion)' is not '$version'" }
if ($info.ProductName -ne 'Itda' -or $info.OriginalFilename -ne 'itda.exe') { throw 'Unexpected product or file name' }
$numbers = ($version -split '[-+]')[0] -split '\.'
if ($info.FileMajorPart -ne [int]$numbers[0] -or $info.FileMinorPart -ne [int]$numbers[1] -or $info.FileBuildPart -ne [int]$numbers[2]) {
    throw "Numeric file version $($info.FileMajorPart).$($info.FileMinorPart).$($info.FileBuildPart) is not $version"
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ItdaResources {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, ExactSpelling=true, SetLastError=true)]
    static extern IntPtr LoadLibraryExW(string path, IntPtr file, uint flags);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool FreeLibrary(IntPtr module);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, ExactSpelling=true, SetLastError=true)]
    static extern IntPtr FindResourceW(IntPtr module, IntPtr name, IntPtr type);
    // LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE: read resources, run nothing.
    public static bool Has(string path, int type, int id) {
        IntPtr m = LoadLibraryExW(path, IntPtr.Zero, 0x22);
        if (m == IntPtr.Zero) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        try { return FindResourceW(m, new IntPtr(id), new IntPtr(type)) != IntPtr.Zero; }
        finally { FreeLibrary(m); }
    }
}
'@
foreach ($r in @(@{ name = 'icon group'; type = 14; id = 1 }, @{ name = 'manifest'; type = 24; id = 1 }, @{ name = 'version'; type = 16; id = 1 })) {
    if (-not [ItdaResources]::Has($exe, $r.type, $r.id)) { throw "Missing $($r.name) resource" }
}
"itda.exe ${version}: ProductVersion/FileVersion match; icon, manifest and version resources present"
