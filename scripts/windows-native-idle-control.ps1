# Only controls the pilot's copied executable / private WebView2 profile.
param(
    [Parameter(Mandatory)][ValidateSet('startup', 'inspect', 'query-end', 'close', 'cleanup')][string]$Operation,
    [Parameter(Mandatory)][string]$Root,
    [int]$AppPid = 0,
    [string]$CreationTicks = '',
    [string]$Hwnd = '0',
    [int]$DebugPort = 0
)
$ErrorActionPreference = 'Stop'
$ownedRoot = [IO.Path]::GetFullPath($Root).TrimEnd('\')
$temporaryParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
if ([IO.Path]::GetDirectoryName($ownedRoot) -ne $temporaryParent -or
    [IO.Path]::GetFileName($ownedRoot) -notmatch '^itda-native-idle-[a-zA-Z0-9-]+$' -or
    ((Get-Item -LiteralPath $ownedRoot).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'Pilot workspace ownership mismatch'
}
if ($Operation -eq 'startup') {
    $value = (Get-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name Itda -ErrorAction SilentlyContinue).Itda
    $hash = if ($null -ne $value) { [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes([string]$value))).ToLowerInvariant() } else { $null }
    [ordered]@{ exists = ($null -ne $value); value_sha256 = $hash } | ConvertTo-Json -Compress
    exit 0
}
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class ItdaIdleWindow {
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll", SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr h, uint m, UIntPtr w, IntPtr l, uint f, uint ms, out UIntPtr value);
    [DllImport("user32.dll", SetLastError=true)] static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, ExactSpelling=true)] static extern int GetClassNameW(IntPtr h, StringBuilder value, int count);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, ExactSpelling=true)] static extern int GetWindowTextW(IntPtr h, StringBuilder value, int count);
    public static string ClassName(long value) { var s=new StringBuilder(256); GetClassNameW(new IntPtr(value),s,s.Capacity); return s.ToString(); }
    public static string Title(long value) { var s=new StringBuilder(512); GetWindowTextW(new IntPtr(value),s,s.Capacity); return s.ToString(); }
    static IntPtr Own(uint pid, long value) {
        var h = new IntPtr(value); uint actual;
        if (pid == 0 || h == IntPtr.Zero || GetWindowThreadProcessId(h, out actual) == 0 || actual != pid)
            throw new InvalidOperationException("Pilot HWND does not belong to candidate PID");
        if (ClassName(value)!="itda_home" || !System.Text.RegularExpressions.Regex.IsMatch(Title(value),@"^Itda \(profile [0-9a-f]{16}\)$"))
            throw new InvalidOperationException("Pilot HWND is not the isolated home window");
        return h;
    }
    public static long QueryEnd(uint pid, long h) {
        UIntPtr value;
        if (SendMessageTimeoutW(Own(pid,h),0x11,UIntPtr.Zero,IntPtr.Zero,2,2000,out value) == IntPtr.Zero)
            throw new InvalidOperationException("Owned shutdown query failed or timed out");
        return checked((long)value.ToUInt64());
    }
    public static void Close(uint pid, long h) {
        if (!PostMessageW(Own(pid,h),0x10,IntPtr.Zero,IntPtr.Zero))
            throw new InvalidOperationException("Owned window close failed");
    }
}
'@
$exe = Join-Path $ownedRoot 'itda.exe'
$app = if ($AppPid -gt 0) { Get-Process -Id $AppPid -ErrorAction SilentlyContinue } else { $null }
if ($app) {
    if ($app.Path -ne $exe) { throw 'Candidate PID executable path changed' }
    $actualTicks = $app.StartTime.ToUniversalTime().Ticks.ToString()
    if ($CreationTicks -and $actualTicks -ne $CreationTicks) { throw 'Candidate PID creation time changed' }
}
$views = @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object {
    $_.CommandLine -and $_.CommandLine.Contains($ownedRoot + '\', [StringComparison]::OrdinalIgnoreCase)
})
if ($Operation -eq 'cleanup') {
    $forced = $false
    if ($app) { Stop-Process -Id $app.Id -Force; $forced = $true }
    foreach ($view in $views) {
        # Re-read before stopping a child, so a reused PID must still name this profile.
        $current = Get-CimInstance Win32_Process -Filter "ProcessId=$($view.ProcessId)" -ErrorAction SilentlyContinue
        if ($current.Name -eq 'msedgewebview2.exe' -and $current.CommandLine -and $current.CommandLine.Contains($ownedRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
            Stop-Process -Id $current.ProcessId -Force -ErrorAction SilentlyContinue
        }
    }
    $until = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $remaining = @(Get-CimInstance Win32_Process | Where-Object {
            ($_.ExecutablePath -eq $exe) -or ($_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -and $_.CommandLine.Contains($ownedRoot + '\', [StringComparison]::OrdinalIgnoreCase))
        })
        if (-not $remaining.Count) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $until)
    if ($remaining.Count) { throw 'Owned candidate/profile processes remain' }
    [ordered]@{ forced_candidate = $forced; remaining_owned_processes = 0 } | ConvertTo-Json -Compress
    exit 0
}
if (-not $app) { [ordered]@{ alive = $false } | ConvertTo-Json -Compress; exit 0 }
if ($Operation -eq 'inspect') {
    $app.Refresh()
    $handle = $app.MainWindowHandle
    $listeners = if ($DebugPort -gt 0) { @(Get-NetTCPConnection -LocalPort $DebugPort -State Listen -ErrorAction SilentlyContinue | Select-Object LocalAddress, OwningProcess) } else { @() }
    [ordered]@{
        alive = $true; pid = $app.Id; creation_ticks = $actualTicks; session_id = $app.SessionId
        hwnd = $handle.ToInt64().ToString(); visible = ($handle -ne [IntPtr]::Zero -and [ItdaIdleWindow]::IsWindowVisible($handle))
        window_class = [ItdaIdleWindow]::ClassName($handle.ToInt64()); window_title = [ItdaIdleWindow]::Title($handle.ToInt64())
        private_udf = (Test-Path -LiteralPath (Join-Path $ownedRoot 'data\Itda\WebView2'))
        webview_pids = @($views | ForEach-Object { [int]$_.ProcessId }); listeners = @($listeners)
    } | ConvertTo-Json -Depth 5 -Compress
    exit 0
}
if (-not $CreationTicks -or $Hwnd -notmatch '^[1-9][0-9]*$') { throw 'Pinned process identity and HWND required' }
if ($Operation -eq 'query-end') {
    [ordered]@{ response = [ItdaIdleWindow]::QueryEnd($app.Id, [long]$Hwnd) } | ConvertTo-Json -Compress
} elseif ($Operation -eq 'close') {
    [ItdaIdleWindow]::Close($app.Id, [long]$Hwnd)
    [ordered]@{ posted = $true } | ConvertTo-Json -Compress
}
