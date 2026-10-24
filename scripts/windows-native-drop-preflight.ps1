# Read-only drop-specific additions. No shell creation, input, elevation or settings writes.
param([Parameter(Mandatory)][string]$Receipt)
$ErrorActionPreference='Stop'
$base = "$Receipt.base.json"
& pwsh -NoProfile -NonInteractive -File "$PSScriptRoot/windows-native-preflight.ps1" -Receipt $base | Out-Null
$r=Get-Content -LiteralPath $base -Raw | ConvertFrom-Json -AsHashtable
try {
 Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ItdaDropProbe {
 [StructLayout(LayoutKind.Sequential)] struct LAST {public uint Size,Time;}
 [DllImport("user32.dll")] static extern bool GetLastInputInfo(ref LAST v);
 [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int k);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c,string t);
 [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
 [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint a,bool i,uint p);
 [DllImport("advapi32.dll")] static extern bool OpenProcessToken(IntPtr p,uint a,out IntPtr t);
 [DllImport("advapi32.dll")] static extern bool GetTokenInformation(IntPtr t,int c,IntPtr v,uint n,out uint needed);
 [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthorityCount(IntPtr s);
 [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthority(IntPtr s,uint n);
 [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr p);
 public static uint Pid(long h){uint p;GetWindowThreadProcessId(new IntPtr(h),out p);return p;}
 public static uint Idle(){LAST v=new LAST();v.Size=(uint)Marshal.SizeOf(typeof(LAST));if(!GetLastInputInfo(ref v))throw new Exception("Idle unreadable");return unchecked((uint)Environment.TickCount-v.Time);}
 public static uint Integrity(uint pid){IntPtr p=OpenProcess(0x1000,false,pid),t=IntPtr.Zero,b=IntPtr.Zero;try{if(p==IntPtr.Zero||!OpenProcessToken(p,8,out t))throw new Exception("Token unavailable");uint needed;GetTokenInformation(t,25,IntPtr.Zero,0,out needed);if(needed==0)throw new Exception("IL unavailable");b=Marshal.AllocHGlobal((int)needed);if(!GetTokenInformation(t,25,b,needed,out needed))throw new Exception("IL unreadable");IntPtr sid=Marshal.ReadIntPtr(b);byte count=Marshal.ReadByte(GetSidSubAuthorityCount(sid));return unchecked((uint)Marshal.ReadInt32(GetSidSubAuthority(sid,(uint)count-1)));}finally{if(b!=IntPtr.Zero)Marshal.FreeHGlobal(b);if(t!=IntPtr.Zero)CloseHandle(t);if(p!=IntPtr.Zero)CloseHandle(p);}}
}
'@
 $shell=[ItdaDropProbe]::FindWindow('Shell_TrayWnd',$null).ToInt64()
 $shellPid=if($shell){[ItdaDropProbe]::Pid($shell)}else{0}
 $p=if($shellPid){Get-Process -Id $shellPid -ErrorAction Stop}else{$null}
 $r.checks.explorer=@{shell_hwnd="$shell";pid=$shellPid;session_id=if($p){$p.SessionId}else{$null};name=if($p){$p.ProcessName}else{$null};integrity=if($p){[ItdaDropProbe]::Integrity($shellPid)}else{$null}}
 if(-not $p -or $p.ProcessName -ne 'explorer' -or $p.SessionId -ne $r.checks.session.id){$r.unmet+= 'same-session-existing-explorer-shell'}
 $r.checks.runner_integrity=[ItdaDropProbe]::Integrity($PID)
 $r.checks.integrity_match=($r.checks.runner_integrity -eq $r.checks.explorer.integrity)
 # Mismatch is a prerequisite, not permission to elevate a shell or change UAC.
 if(-not $r.checks.integrity_match){$r.unmet+='runner-explorer-integrity-mismatch'}
 $foreground=[ItdaDropProbe]::GetForegroundWindow().ToInt64()
 $r.checks.foreground=@{hwnd="$foreground";pid=if($foreground){[ItdaDropProbe]::Pid($foreground)}else{0}}
 $r.checks.idle_ms=[ItdaDropProbe]::Idle()
 $r.checks.pressed=@(1,2,4,5,6,16,17,18,91,92 | Where-Object {([ItdaDropProbe]::GetAsyncKeyState($_) -band 0x8000) -ne 0})
 if($r.checks.idle_ms -lt 5000 -or $r.checks.idle_ms -gt [int]::MaxValue){$r.unmet+='idle-at-least-five-seconds'}
 if($r.checks.pressed.Count){$r.unmet+='no-buttons-or-modifiers-down'}
 if($r.status -ne 'probe-error'){$r.status=if($r.unmet.Count){'prerequisite-unavailable'}else{'eligible-for-owned-drop'}}
} catch {$r.status='probe-error';$r.error=$_.Exception.Message}
$r.recorded_at=[DateTime]::UtcNow.ToString('o')
$r.scope='Read-only same-job Explorer/session/IL/input prerequisites; no native input or app launch'
[IO.File]::WriteAllText([IO.Path]::GetFullPath($Receipt),($r|ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($false))
if($r.status -eq 'probe-error'){exit 1}
if($r.status -eq 'prerequisite-unavailable'){exit 2}
