# Read-only environment probe, not an Itda/native UI test. Writes only the requested receipt.
# Never switches/unlocks a desktop, installs a runtime, launches Itda or sends OS input.
param([Parameter(Mandatory = $true)][string]$Receipt)
$ErrorActionPreference = 'Stop'
$report = [ordered]@{
    schema_version = 1
    recorded_at = [DateTime]::UtcNow.ToString('o')
    status = 'probe-error'
    scope = 'Windows CI native-test prerequisites only; no app launch or user-journey acceptance'
    source = [ordered]@{
        checkout = $env:GITHUB_SHA
        pr_head = $env:ITDA_PR_HEAD
        run_id = $env:GITHUB_RUN_ID
        run_attempt = $env:GITHUB_RUN_ATTEMPT
        runner_image = $env:ImageOS
        runner_image_version = $env:ImageVersion
    }
    checks = [ordered]@{}
    unmet = @()
    references = @(
        'https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution',
        'https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-openinputdesktop',
        'https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/ne-wtsapi32-wts_connectstate_class'
    )
}
$exitCode = 1
try {
    if (-not $IsWindows) { throw 'This probe requires Windows PowerShell Core on Windows.' }
    Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class ItdaNativePrerequisites {
    [DllImport("user32.dll", SetLastError=true)]
    public static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll", SetLastError=true)]
    public static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, ExactSpelling=true, SetLastError=true)]
    public static extern bool GetUserObjectInformationW(IntPtr handle, int index, StringBuilder value, uint bytes, out uint needed);
    [DllImport("user32.dll", SetLastError=true)]
    public static extern IntPtr GetThreadDesktop(uint threadId);
    [DllImport("user32.dll")]
    public static extern IntPtr GetProcessWindowStation();
    [DllImport("kernel32.dll")]
    public static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")]
    public static extern int GetSystemMetrics(int metric);
    [DllImport("user32.dll")]
    public static extern uint GetDpiForSystem();
    [DllImport("user32.dll")]
    public static extern IntPtr GetThreadDpiAwarenessContext();
    [DllImport("user32.dll")]
    public static extern int GetAwarenessFromDpiAwarenessContext(IntPtr context);
    [DllImport("wtsapi32.dll", CharSet=CharSet.Unicode, ExactSpelling=true, SetLastError=true)]
    public static extern bool WTSQuerySessionInformationW(IntPtr server, uint session, int info, out IntPtr buffer, out uint bytes);
    [DllImport("wtsapi32.dll")]
    public static extern void WTSFreeMemory(IntPtr buffer);
}
'@
    function DesktopName([IntPtr]$handle) {
        if ($handle -eq [IntPtr]::Zero) { return $null }
        $name = [Text.StringBuilder]::new(256)
        [uint32]$needed = 0
        if (-not [ItdaNativePrerequisites]::GetUserObjectInformationW($handle, 2, $name, 512, [ref]$needed)) {
            throw "Cannot read desktop name (Win32 $([Runtime.InteropServices.Marshal]::GetLastWin32Error()))."
        }
        return $name.ToString()
    }
    $session = [Diagnostics.Process]::GetCurrentProcess().SessionId
    $report.checks.session = [ordered]@{ id = $session; user_interactive = [Environment]::UserInteractive }
    if ($session -eq 0 -or -not [Environment]::UserInteractive) { $report.unmet += 'interactive-nonzero-session' }

    # OpenInputDesktop can return the restore desktop even in a disconnected session.
    # Require WTSActive as well; a Default name alone is not evidence of availability.
    [IntPtr]$buffer = [IntPtr]::Zero
    [uint32]$bytes = 0
    try {
        $ok = [ItdaNativePrerequisites]::WTSQuerySessionInformationW([IntPtr]::Zero, $session, 8, [ref]$buffer, [ref]$bytes)
        if (-not $ok -or $buffer -eq [IntPtr]::Zero -or $bytes -lt 4) {
            $report.checks.session.connect_state = $null
            $report.unmet += 'readable-session-connection-state'
        } else {
            $state = [Runtime.InteropServices.Marshal]::ReadInt32($buffer)
            $report.checks.session.connect_state = $state
            if ($state -ne 0) { $report.unmet += 'actively-connected-session' }
        }
    } finally {
        if ($buffer -ne [IntPtr]::Zero) { [ItdaNativePrerequisites]::WTSFreeMemory($buffer) }
    }
    $desktop = [ItdaNativePrerequisites]::OpenInputDesktop(0, $false, 1) # DESKTOP_READOBJECTS only
    $openError = if ($desktop -eq [IntPtr]::Zero) { [Runtime.InteropServices.Marshal]::GetLastWin32Error() } else { $null }
    try {
        $inputName = DesktopName $desktop
        $threadName = DesktopName ([ItdaNativePrerequisites]::GetThreadDesktop([ItdaNativePrerequisites]::GetCurrentThreadId()))
        $stationName = DesktopName ([ItdaNativePrerequisites]::GetProcessWindowStation())
        $report.checks.desktop = [ordered]@{ input = $inputName; thread = $threadName; window_station = $stationName; open_input_error = $openError }
        if ($inputName -ne 'Default' -or $threadName -ne 'Default') { $report.unmet += 'default-input-and-thread-desktop' }
        if ($stationName -ne 'WinSta0') { $report.unmet += 'interactive-window-station' }
    } finally {
        # GetThreadDesktop returns a borrowed handle; only the opened input handle is closed.
        if ($desktop -ne [IntPtr]::Zero) { [void][ItdaNativePrerequisites]::CloseDesktop($desktop) }
    }
    $width = [ItdaNativePrerequisites]::GetSystemMetrics(0)
    $height = [ItdaNativePrerequisites]::GetSystemMetrics(1)
    $awareness = [ItdaNativePrerequisites]::GetAwarenessFromDpiAwarenessContext([ItdaNativePrerequisites]::GetThreadDpiAwarenessContext())
    $report.checks.screen = [ordered]@{ width = $width; height = $height; system_dpi = [ItdaNativePrerequisites]::GetDpiForSystem(); thread_awareness = $awareness; metric_space = 'Calling PowerShell DPI context; not certified physical pixels' }
    if ($width -lt 1024 -or $height -lt 768) { $report.unmet += 'screen-at-least-1024x768' }
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try {
        $principal = [Security.Principal.WindowsPrincipal]::new($identity)
        $report.checks.elevated_admin_token = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    } finally { $identity.Dispose() }

    # Microsoft documents the EdgeUpdate pv value for Evergreen Runtime detection.
    # Explicit views cover per-machine 32-bit and per-user registrations without writes.
    $versions = @()
    foreach ($hive in @([Microsoft.Win32.RegistryHive]::LocalMachine, [Microsoft.Win32.RegistryHive]::CurrentUser)) {
        foreach ($view in @([Microsoft.Win32.RegistryView]::Registry32, [Microsoft.Win32.RegistryView]::Registry64)) {
            $baseKey = [Microsoft.Win32.RegistryKey]::OpenBaseKey($hive, $view)
            $key = $null
            try {
                $key = $baseKey.OpenSubKey('SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}', $false)
                $value = if ($key) { [string]$key.GetValue('pv', '') } else { '' }
                $version = $null
                if ([Version]::TryParse($value, [ref]$version) -and $version -gt [Version]'0.0.0.0') {
                    $versions += [ordered]@{ hive = $hive.ToString(); view = $view.ToString(); version = $value }
                }
            } finally {
                if ($key) { $key.Dispose() }
                $baseKey.Dispose()
            }
        }
    }
    $report.checks.webview2_runtime = $versions
    if (-not $versions.Count) { $report.unmet += 'installed-webview2-runtime' }
    $report.status = if ($report.unmet.Count) { 'prerequisite-unavailable' } else { 'eligible-for-idle-pilot' }
    $exitCode = if ($report.unmet.Count) { 2 } else { 0 }
} catch {
    $report.error = $_.Exception.Message
} finally {
    $fullPath = [IO.Path]::GetFullPath($Receipt)
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($fullPath))
    [IO.File]::WriteAllText($fullPath, ($report | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
    Write-Output "Native prerequisite status: $($report.status); receipt: $fullPath"
}
exit $exitCode
