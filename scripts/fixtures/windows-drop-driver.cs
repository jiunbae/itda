using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
public class ItdaQualityWindow {
 public delegate bool CB(IntPtr h,IntPtr p);
 [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X,Y; }
 [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
 [StructLayout(LayoutKind.Sequential)] struct MONITORINFO { public uint Size; public RECT Monitor,Work; public uint Flags; }
 [StructLayout(LayoutKind.Sequential)] struct LASTINPUTINFO { public uint Size,Time; }
 [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int X,Y; public uint Data,Flags,Time; public UIntPtr Extra; }
 [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort Key,Scan; public uint Flags,Time; public UIntPtr Extra; }
 [StructLayout(LayoutKind.Explicit)] struct UNION { [FieldOffset(0)] public MOUSEINPUT Mouse; [FieldOffset(0)] public KEYBDINPUT Key; }
 [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint Type; public UNION Value; }
 [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h,int c);
 [DllImport("user32.dll")] static extern bool EnumWindows(CB cb,IntPtr p);
 [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,CB cb,IntPtr p);
 [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
 [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll")] static extern bool IsWindow(IntPtr h);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextLength(IntPtr h);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h,StringBuilder b,int size);
 [DllImport("user32.dll",CharSet=CharSet.Unicode,ExactSpelling=true)] static extern int GetWindowTextW(IntPtr h,StringBuilder b,int size);
 [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr value);
 [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int w,int hh,uint flags);
 [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h,out RECT r);
 [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h,ref POINT p);
 [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr h);
 [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr h,uint flags);
 [DllImport("user32.dll")] static extern bool GetMonitorInfo(IntPtr h,ref MONITORINFO info);
 [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT p);
 [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h,uint flags);
 [DllImport("user32.dll")] static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT p);
 [DllImport("user32.dll")] static extern uint SendInput(uint count,INPUT[] inputs,int size);
 [DllImport("user32.dll")] static extern bool GetLastInputInfo(ref LASTINPUTINFO info);
 [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a,uint b,bool attach);
 [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
 [StructLayout(LayoutKind.Sequential)] struct MSG { public IntPtr Window;public uint Message;public UIntPtr W;public IntPtr L;public uint Time;public POINT P;public uint Private; }
 [StructLayout(LayoutKind.Sequential)] struct MOUSEHOOK { public POINT P;public uint Data,Flags,Time;public UIntPtr Extra; }
 [StructLayout(LayoutKind.Sequential)] struct KEYHOOK { public uint Key,Scan,Flags,Time;public UIntPtr Extra; }
 delegate IntPtr HookProc(int code,IntPtr w,IntPtr l);
 [DllImport("user32.dll")] static extern IntPtr SetWindowsHookEx(int type,HookProc callback,IntPtr module,uint thread);
 [DllImport("user32.dll")] static extern bool UnhookWindowsHookEx(IntPtr hook);
 [DllImport("user32.dll")] static extern IntPtr CallNextHookEx(IntPtr hook,int code,IntPtr w,IntPtr l);
 [DllImport("user32.dll")] static extern bool PeekMessage(out MSG msg,IntPtr window,uint min,uint max,uint remove);
 [DllImport("user32.dll")] static extern bool TranslateMessage(ref MSG msg);
 [DllImport("user32.dll")] static extern IntPtr DispatchMessage(ref MSG msg);
 [DllImport("user32.dll")] static extern int GetSystemMetrics(int index);
 [DllImport("user32.dll")] static extern short GetAsyncKeyState(int key);
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] static extern IntPtr GetModuleHandle(string module);
 [DllImport("user32.dll")] static extern IntPtr OpenInputDesktop(uint f,bool inherit,uint access);
 [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr h);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern bool GetUserObjectInformation(IntPtr h,int index,StringBuilder b,uint size,out uint needed);
 [DllImport("wtsapi32.dll")] static extern bool WTSQuerySessionInformation(IntPtr server,uint session,int kind,out IntPtr data,out uint bytes);
 [DllImport("wtsapi32.dll")] static extern void WTSFreeMemory(IntPtr data);
 [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint a,bool i,uint p);
 [DllImport("advapi32.dll")] static extern bool OpenProcessToken(IntPtr p,uint a,out IntPtr t);
 [DllImport("advapi32.dll")] static extern bool GetTokenInformation(IntPtr t,int c,IntPtr v,uint n,out uint needed);
 [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthorityCount(IntPtr s);
 [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthority(IntPtr s,uint n);
 [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr p);
 public static uint Integrity(uint pid){IntPtr p=OpenProcess(0x1000,false,pid),t=IntPtr.Zero,b=IntPtr.Zero;try{if(p==IntPtr.Zero||!OpenProcessToken(p,8,out t))throw new Exception("Token unavailable");uint needed;GetTokenInformation(t,25,IntPtr.Zero,0,out needed);if(needed==0)throw new Exception("IL unavailable");b=Marshal.AllocHGlobal((int)needed);if(!GetTokenInformation(t,25,b,needed,out needed))throw new Exception("IL unreadable");IntPtr sid=Marshal.ReadIntPtr(b);byte count=Marshal.ReadByte(GetSidSubAuthorityCount(sid));return unchecked((uint)Marshal.ReadInt32(GetSidSubAuthority(sid,(uint)count-1)));}finally{if(b!=IntPtr.Zero)Marshal.FreeHGlobal(b);if(t!=IntPtr.Zero)CloseHandle(t);if(p!=IntPtr.Zero)CloseHandle(p);}}
 static void AssertDesktop(){
  IntPtr desktop=OpenInputDesktop(0,false,1),data=IntPtr.Zero;uint needed;
  try{StringBuilder name=new StringBuilder(256);if(desktop==IntPtr.Zero||!GetUserObjectInformation(desktop,2,name,512,out needed)||name.ToString()!="Default")throw new Exception("Default input desktop unavailable");uint bytes;if(!WTSQuerySessionInformation(IntPtr.Zero,(uint)System.Diagnostics.Process.GetCurrentProcess().SessionId,8,out data,out bytes)||data==IntPtr.Zero||bytes<4||Marshal.ReadInt32(data)!=0)throw new Exception("Interactive session is no longer active");}
  finally{if(desktop!=IntPtr.Zero)CloseDesktop(desktop);if(data!=IntPtr.Zero)WTSFreeMemory(data);}
 }
 static void Pump(int ms){DateTime end=DateTime.UtcNow.AddMilliseconds(ms);do{MSG m;while(PeekMessage(out m,IntPtr.Zero,0,0,1)){TranslateMessage(ref m);DispatchMessage(ref m);}Thread.Sleep(2);}while(DateTime.UtcNow<end);}
 public enum HookKind { Mouse,Key }
 public enum HookMessageClass { MouseMove,MouseButtonDown,MouseButtonUp,MouseWheel,OtherMouse,KeyDown,KeyUp,OtherKey }
 public enum ActivationRoute { NotAttempted,ForegroundRequest,OwnedStatusBarFallback }
 public class UnmatchedHookInfo {
  public HookKind Hook;public HookMessageClass MessageClass;
  public bool Injected,LowerILInjected,ExtraZero,ExtraEqual,ExtraLow32Equal,ExtraHigh32Zero,LastOwnTargetKnown,MousePointEqualLastOwnTarget;
  public int MsSinceLastTaggedSendAttempt,TaggedSendAttemptCount;
 }
 public class DragInfo { public string Outcome,ReleasePoint,Error;public bool Interfered,EmergencyEscaped,EmergencyMayHaveOfferedSource;public uint InputBefore,InputAfter,IdleBefore,TickBefore;public string FocusRestore,CursorRestore;public uint PriorForegroundPid;public long StartedUnixMs,ReleaseUnixMs,FirstExplorerUnixMs;public uint[] GuardBefore,GuardAfter;
  public ActivationRoute ActivationPath;public int TaggedEchoCount,TaggedMouseEchoCount,TaggedKeyEchoCount,TaggedSendAttemptCount,UnmatchedHookCount;public List<UnmatchedHookInfo> UnmatchedHooks=new List<UnmatchedHookInfo>();
 }
 // Attribution requires full marker equality; diagnostics never normalize it.
 // At most 8 coarse unmatched records, counts capped at 10000 and elapsed ms at
 // 60000 (-1 before any tagged send). No VK, scan, cursor, message ID, flags word
 // or raw extra-info value is retained. Targets stay private to this one Drag.
 class Attribution {
  readonly DragInfo info;readonly UIntPtr marker;bool sent,haveTarget;uint lastSendTick;POINT lastTarget;
  public Attribution(DragInfo value,UIntPtr tag){info=value;marker=tag;}
  public void BeforeSend(int count){sent=true;lastSendTick=unchecked((uint)Environment.TickCount);info.TaggedSendAttemptCount=Math.Min(10000,info.TaggedSendAttemptCount+count);}
  public void OwnTarget(int x,int y){lastTarget=new POINT{X=x,Y=y};haveTarget=true;}
  public bool SamePoint(POINT point){return haveTarget&&point.X==lastTarget.X&&point.Y==lastTarget.Y;}
  static HookMessageClass Classify(HookKind hook,IntPtr message){
   long m=message.ToInt64();
   if(hook==HookKind.Key){if(m==0x100||m==0x104)return HookMessageClass.KeyDown;if(m==0x101||m==0x105)return HookMessageClass.KeyUp;return HookMessageClass.OtherKey;}
   if(m==0x200)return HookMessageClass.MouseMove;
   if(m==0x201||m==0x204||m==0x207||m==0x20b)return HookMessageClass.MouseButtonDown;
   if(m==0x202||m==0x205||m==0x208||m==0x20c)return HookMessageClass.MouseButtonUp;
   if(m==0x20a||m==0x20e)return HookMessageClass.MouseWheel;return HookMessageClass.OtherMouse;
  }
  public void Observe(HookKind hook,IntPtr message,uint flags,UIntPtr extra,bool samePoint){
   if(extra==marker){info.TaggedEchoCount=Math.Min(10000,info.TaggedEchoCount+1);if(hook==HookKind.Mouse)info.TaggedMouseEchoCount=Math.Min(10000,info.TaggedMouseEchoCount+1);else info.TaggedKeyEchoCount=Math.Min(10000,info.TaggedKeyEchoCount+1);return;}
   // Every unmatched event still invalidates the run, even after the record cap.
   info.Interfered=true;info.UnmatchedHookCount=Math.Min(10000,info.UnmatchedHookCount+1);
   if(info.UnmatchedHooks.Count>=8)return;
   var record=new UnmatchedHookInfo();record.Hook=hook;record.MessageClass=Classify(hook,message);
   // MSLLHOOKSTRUCT: INJECTED=0x1. KBDLLHOOKSTRUCT: INJECTED=0x10.
   // Both use LOWER_IL_INJECTED=0x2. These flags diagnose; none waive failure.
   record.Injected=(flags&(hook==HookKind.Mouse?0x1u:0x10u))!=0;record.LowerILInjected=(flags&0x2)!=0;
   record.ExtraZero=extra==UIntPtr.Zero;record.ExtraEqual=extra==marker;record.ExtraLow32Equal=unchecked((uint)extra.ToUInt64())==unchecked((uint)marker.ToUInt64());record.ExtraHigh32Zero=(extra.ToUInt64()>>32)==0;
   record.LastOwnTargetKnown=haveTarget;record.MousePointEqualLastOwnTarget=hook==HookKind.Mouse&&samePoint;
   record.MsSinceLastTaggedSendAttempt=sent?(int)Math.Min(60000u,unchecked((uint)Environment.TickCount-lastSendTick)):-1;record.TaggedSendAttemptCount=info.TaggedSendAttemptCount;
   info.UnmatchedHooks.Add(record);
  }
 }
 public class LayoutInfo { public long File,Explorer; public int OriginX,OriginY,TargetX,TargetY; public RECT Work,Source,Destination;public uint SourcePid,ExplorerPid,SourceIntegrity,ExplorerIntegrity;public int Session;public string SourceStartedTicks,ExplorerStartedTicks; }
 // Read-only validation of the originally pinned home HWND. Process.MainWindowHandle
 // may switch to the file window; it is not an identity for a specific window.
 // The controller separately verifies canonical executable path and creation time.
 public static void AssertHomeWindow(uint pid,long value) {
  IntPtr h=new IntPtr(value);uint actual;
  if(pid==0||h==IntPtr.Zero||!IsWindow(h)||GetWindowThreadProcessId(h,out actual)==0||actual!=pid)throw new Exception("Pinned home HWND PID changed");
  StringBuilder cls=new StringBuilder(256),title=new StringBuilder(512);GetClassName(h,cls,cls.Capacity);GetWindowTextW(h,title,title.Capacity);
  if(cls.ToString()!="itda_home"||!System.Text.RegularExpressions.Regex.IsMatch(title.ToString(),@"^Itda \(profile [0-9a-f]{16}\)$")||!IsWindowVisible(h))throw new Exception("Pinned home HWND class/title/visibility changed");
 }
 public static LayoutInfo Layout(uint pid,long main,long explorer) {
  IntPtr old=SetThreadDpiAwarenessContext(new IntPtr(-4)); if(old==IntPtr.Zero)throw new Exception("PMv2 context unavailable");
  try {
   List<IntPtr> files=new List<IntPtr>();
   EnumWindows((h,p)=>{uint id;GetWindowThreadProcessId(h,out id);if(id==pid&&h.ToInt64()!=main&&IsWindowVisible(h)&&GetWindowTextLength(h)>0)files.Add(h);return true;},IntPtr.Zero);
   if(files.Count!=1)throw new Exception("Exactly one owned file window required");
   IntPtr file=files[0],target=new IntPtr(explorer);
   uint filePid,explorerPid;GetWindowThreadProcessId(file,out filePid);GetWindowThreadProcessId(target,out explorerPid);
   StringBuilder cls=new StringBuilder(256);GetClassName(target,cls,256);
   var fp=System.Diagnostics.Process.GetProcessById((int)filePid);var ep=System.Diagnostics.Process.GetProcessById((int)explorerPid);
   if(cls.ToString()!="CabinetWClass"||ep.ProcessName!="explorer"||fp.SessionId!=ep.SessionId||ep.SessionId!=System.Diagnostics.Process.GetCurrentProcess().SessionId)throw new Exception("Explorer PID/class/session mismatch");
   uint fi=Integrity(filePid),ei=Integrity(explorerPid);if(fi!=ei)throw new Exception("prerequisite-unavailable: candidate-Explorer IL mismatch");
   if(!IsWindowVisible(target))throw new Exception("Owned Explorer window not visible");
   MONITORINFO mi=new MONITORINFO();mi.Size=(uint)Marshal.SizeOf(typeof(MONITORINFO));
   if(!GetMonitorInfo(MonitorFromWindow(file,2),ref mi))throw new Exception("Work area unavailable");
   int width=mi.Work.Right-mi.Work.Left,height=mi.Work.Bottom-mi.Work.Top,half=(width-60)/2;
   if(half<440||height<640)throw new Exception("prerequisite-unavailable: monitor too small for two isolated drop windows");
   if(!SetWindowPos(file,IntPtr.Zero,mi.Work.Left+10,mi.Work.Top+20,half,Math.Min(850,height-40),0x14)||!SetWindowPos(target,IntPtr.Zero,mi.Work.Left+10+half,mi.Work.Top+20,half,Math.Min(850,height-40),0x14))throw new Exception("Owned window placement failed");
   Thread.Sleep(600);
   RECT source,destination;GetWindowRect(file,out source);GetWindowRect(target,out destination);
   if(source.Right-source.Left<440||destination.Right-destination.Left<420||source.Bottom-source.Top<600||destination.Bottom-destination.Top<600||source.Right>destination.Left||source.Left<mi.Work.Left||destination.Right>mi.Work.Right)throw new Exception("prerequisite-unavailable: measured windows overlap or exceed work area");
   List<IntPtr> views=new List<IntPtr>();
   EnumChildWindows(target,(h,p)=>{StringBuilder b=new StringBuilder(256);GetClassName(h,b,256);if(b.ToString()=="SHELLDLL_DefView"&&IsWindowVisible(h))views.Add(h);return true;},IntPtr.Zero);
   if(views.Count!=1)throw new Exception("Cannot identify unique Explorer folder view");
   RECT view;GetWindowRect(views[0],out view);
   POINT origin=new POINT();origin.Y=(int)(4*GetDpiForWindow(file)/96);
   if(!ClientToScreen(file,ref origin))throw new Exception("File client origin unavailable");
   LayoutInfo info=new LayoutInfo();info.File=file.ToInt64();info.Explorer=explorer;info.OriginX=origin.X;info.OriginY=origin.Y;info.TargetX=(view.Left+view.Right)/2;info.TargetY=(view.Top+view.Bottom)/2;info.SourcePid=filePid;info.ExplorerPid=explorerPid;info.SourceIntegrity=fi;info.ExplorerIntegrity=ei;info.Session=fp.SessionId;info.SourceStartedTicks=fp.StartTime.ToUniversalTime().Ticks.ToString();info.ExplorerStartedTicks=ep.StartTime.ToUniversalTime().Ticks.ToString();info.Work=mi.Work;info.Source=source;info.Destination=destination;return info;
  } finally {SetThreadDpiAwarenessContext(old);}
 }
 static void Mouse(uint flags,UIntPtr marker,Attribution attribution){INPUT i=new INPUT();i.Value.Mouse.Flags=flags;i.Value.Mouse.Extra=marker;attribution.BeforeSend(1);if(SendInput(1,new INPUT[]{i},Marshal.SizeOf(typeof(INPUT)))!=1)throw new Exception("Mouse input rejected");}
 static void Move(int x,int y,UIntPtr marker,Attribution attribution,uint buttons=0){int left=GetSystemMetrics(76),top=GetSystemMetrics(77),w=GetSystemMetrics(78),h=GetSystemMetrics(79);if(w<2||h<2)throw new Exception("Virtual desktop unavailable");INPUT i=new INPUT();i.Value.Mouse.X=(int)((long)(x-left)*65535/(w-1));i.Value.Mouse.Y=(int)((long)(y-top)*65535/(h-1));i.Value.Mouse.Flags=0xc001|buttons;i.Value.Mouse.Extra=marker;attribution.OwnTarget(x,y);attribution.BeforeSend(1);if(SendInput(1,new INPUT[]{i},Marshal.SizeOf(typeof(INPUT)))!=1)throw new Exception("Move input rejected");}
 static void Escape(UIntPtr marker,Attribution attribution){INPUT a=new INPUT();a.Type=1;a.Value.Key.Key=0x1b;a.Value.Key.Extra=marker;INPUT b=a;b.Value.Key.Flags=2;attribution.BeforeSend(2);if(SendInput(2,new INPUT[]{a,b},Marshal.SizeOf(typeof(INPUT)))!=2)throw new Exception("Cancel input rejected");}
 // Activate only the requested fixture source (or restore a previously saved
 // window), without typing/clicking on the old foreground window.
 static void ActivateWindow(IntPtr target){
  IntPtr foreground=GetForegroundWindow();if(foreground==IntPtr.Zero){SetForegroundWindow(target);return;}
  uint pid;uint thread=GetWindowThreadProcessId(foreground,out pid),current=GetCurrentThreadId();bool attached=false;
  try{if(thread!=current){attached=AttachThreadInput(current,thread,true);if(!attached)throw new Exception("Foreground input queue unavailable");}SetForegroundWindow(target);if(GetForegroundWindow()!=target)throw new Exception("Owned source could not become foreground");}
  finally{if(attached)AttachThreadInput(current,thread,false);}
 }
 static void AssertOwners(LayoutInfo layout){
  uint a,b;GetWindowThreadProcessId(new IntPtr(layout.File),out a);GetWindowThreadProcessId(new IntPtr(layout.Explorer),out b);
  if(a!=layout.SourcePid||b!=layout.ExplorerPid)throw new Exception("Fixture HWND PID changed");
  var source=System.Diagnostics.Process.GetProcessById((int)a);var target=System.Diagnostics.Process.GetProcessById((int)b);
  if(source.StartTime.ToUniversalTime().Ticks.ToString()!=layout.SourceStartedTicks||target.StartTime.ToUniversalTime().Ticks.ToString()!=layout.ExplorerStartedTicks||source.SessionId!=layout.Session||target.SessionId!=layout.Session)throw new Exception("Fixture process identity changed");
 }
 public static uint[] ProbeIdle(){LASTINPUTINFO v=new LASTINPUTINFO();v.Size=(uint)Marshal.SizeOf(typeof(LASTINPUTINFO));if(!GetLastInputInfo(ref v))throw new Exception("Idle unknown");uint tick=unchecked((uint)Environment.TickCount);return new uint[]{v.Time,tick,unchecked(tick-v.Time)};}
 // One bounded scenario. No retries or payload downloads are initiated here.
 public static DragInfo Drag(LayoutInfo layout,int x,int y,bool cancel){
  long source=layout.File,destination=layout.Explorer;int targetX=layout.TargetX,targetY=layout.TargetY;
  if(Environment.GetEnvironmentVariable("GITHUB_ACTIONS")!="true"||Environment.GetEnvironmentVariable("ITDA_ENABLE_OWNED_INPUT")!="true"||!System.Text.RegularExpressions.Regex.IsMatch(Environment.GetEnvironmentVariable("ITDA_EXPECTED_SOURCE_SHA")??"","^[0-9a-f]{40}$")||Environment.GetEnvironmentVariable("ITDA_EXPECTED_SOURCE_SHA")!=Environment.GetEnvironmentVariable("GITHUB_SHA"))throw new Exception("Native input gate is closed");
  // Run 40069501377 observed low32 equality without full equality; this does not
  // prove an OS-wide truncation rule. Use a nonzero positive 31-bit random tag
  // in UIntPtr, avoiding dependence on high-word or sign-bit preservation.
  // This is attribution, not a security boundary. Full equality remains required;
  // low32 equality never waives interference. Fail before native work if RNG fails.
  UIntPtr marker;
  using(var random=System.Security.Cryptography.RandomNumberGenerator.Create()){
   byte[] bytes=new byte[4];uint value=0;
   for(int attempt=0;attempt<8&&value==0;attempt++){random.GetBytes(bytes);value=BitConverter.ToUInt32(bytes,0)&0x7fffffffu;}
   if(value==0)throw new Exception("Nonzero input attribution marker unavailable");
   marker=new UIntPtr(value);
  }
  DragInfo info=new DragInfo();info.StartedUnixMs=DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();IntPtr old=SetThreadDpiAwarenessContext(new IntPtr(-4));if(old==IntPtr.Zero)throw new Exception("PMv2 context unavailable");
  bool down=false;POINT prior;bool havePrior=GetCursorPos(out prior);IntPtr file=new IntPtr(source),explorer=new IntPtr(destination),mouseHook=IntPtr.Zero,keyHook=IntPtr.Zero,priorForeground=IntPtr.Zero;uint priorPid=0;
  var attribution=new Attribution(info,marker);HookProc mouse=null,keyboard=null;
  try{
   AssertOwners(layout);AssertDesktop();
   LASTINPUTINFO idle=new LASTINPUTINFO();idle.Size=(uint)Marshal.SizeOf(typeof(LASTINPUTINFO));
   if(!havePrior||!GetLastInputInfo(ref idle))throw new Exception("Console cursor/idle state unavailable");
   info.InputBefore=idle.Time;info.TickBefore=unchecked((uint)Environment.TickCount);info.IdleBefore=unchecked(info.TickBefore-idle.Time);if(info.IdleBefore>Int32.MaxValue||info.IdleBefore<5000)throw new Exception("Console has recent input; no injected drag attempted");
   foreach(int key in new int[]{1,2,4,5,6,16,17,18,91,92})if((GetAsyncKeyState(key)&0x8000)!=0)throw new Exception("Input button/modifier already down");
   priorForeground=GetForegroundWindow();GetWindowThreadProcessId(priorForeground,out priorPid);info.PriorForegroundPid=priorPid;
   // Monitor only attribution, never record key values or cursor data. Every
   // injected event is tagged; any other event invalidates this run.
   mouse=(code,w,l)=>{if(code>=0){var value=(MOUSEHOOK)Marshal.PtrToStructure(l,typeof(MOUSEHOOK));attribution.Observe(HookKind.Mouse,w,value.Flags,value.Extra,attribution.SamePoint(value.P));}return CallNextHookEx(IntPtr.Zero,code,w,l);};
   keyboard=(code,w,l)=>{if(code>=0){var value=(KEYHOOK)Marshal.PtrToStructure(l,typeof(KEYHOOK));attribution.Observe(HookKind.Key,w,value.Flags,value.Extra,false);}return CallNextHookEx(IntPtr.Zero,code,w,l);};
   mouseHook=SetWindowsHookEx(14,mouse,GetModuleHandle(null),0);keyHook=SetWindowsHookEx(13,keyboard,GetModuleHandle(null),0);
   if(mouseHook==IntPtr.Zero||keyHook==IntPtr.Zero)throw new Exception("Input attribution hooks unavailable");
   // The desktop is idle and attribution is active. Raise only our two test
   // windows, then focus the source; never send raw input to the old foreground.
   if(!SetWindowPos(file,IntPtr.Zero,0,0,0,0,0x13))throw new Exception("Owned source could not be raised");
   info.ActivationPath=ActivationRoute.ForegroundRequest;try{ActivateWindow(file);}catch{}
   if(GetForegroundWindow()!=file){
    info.ActivationPath=ActivationRoute.OwnedStatusBarFallback;RECT bounds;if(!GetWindowRect(file,out bounds))throw new Exception("Owned source geometry unavailable");
    // Owned blank status-bar space, never a foreign foreground window or title-bar button.
    int ax=bounds.Right-32,ay=bounds.Bottom-18;AssertOwners(layout);AssertDesktop();
    if(GetAncestor(WindowFromPoint(new POINT{X=ax,Y=ay}),2)!=file)throw new Exception("Activation point is not owned");
    int activationEchoBefore=info.TaggedMouseEchoCount;Move(ax,ay,marker,attribution);Pump(30);if(info.Interfered)throw new Exception("External input during activation");
    if(info.TaggedMouseEchoCount<=activationEchoBefore)throw new Exception("Tagged activation mouse echo absent; no activation click attempted");
    if(GetAncestor(WindowFromPoint(new POINT{X=ax,Y=ay}),2)!=file)throw new Exception("Activation point changed");
    Mouse(2,marker,attribution);down=true;Mouse(4,marker,attribution);down=false;Pump(80);
    if(GetForegroundWindow()!=file)throw new Exception("Owned source activation refused");
   }
   if(!SetWindowPos(explorer,IntPtr.Zero,0,0,0,0,0x13))throw new Exception("Owned Explorer could not be raised");
   Pump(50);if(info.Interfered)throw new Exception("External console input during focus");
   if(GetAncestor(WindowFromPoint(new POINT{X=x,Y=y}),2)!=file||GetAncestor(WindowFromPoint(new POINT{X=targetX,Y=targetY}),2)!=explorer)throw new Exception("Drop coordinates do not belong to owned windows");
   int sourceEchoBefore=info.TaggedMouseEchoCount;Move(x,y,marker,attribution);Pump(80);if(info.Interfered||GetForegroundWindow()!=file)throw new Exception("Input/focus changed before mouse down");
   if(info.TaggedMouseEchoCount<=sourceEchoBefore)throw new Exception("Tagged source mouse echo absent; no drag mouse down attempted");
   Move(x,y,marker,attribution,2);down=true;
   for(int i=1;i<=120;i++){
    int nx=x+(targetX-x)*i/120,ny=y+(targetY-y)*i/120;
    AssertOwners(layout);IntPtr hit=GetAncestor(WindowFromPoint(new POINT{X=nx,Y=ny}),2);
    if(hit!=file&&hit!=explorer)throw new Exception("Drag path left owned windows");
    if(GetForegroundWindow()!=file&&GetForegroundWindow()!=explorer)throw new Exception("Foreground left owned windows");
    if(hit==explorer&&info.FirstExplorerUnixMs==0)info.FirstExplorerUnixMs=DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();AssertDesktop();Move(nx,ny,marker,attribution);Pump(12);if(info.Interfered)throw new Exception("External console input detected");
   }
   Pump(500);if(info.Interfered)throw new Exception("External console input detected");
   if(GetAncestor(WindowFromPoint(new POINT{X=targetX,Y=targetY}),2)!=explorer)throw new Exception("Drop target changed");
   if(cancel){int cancelEchoBefore=info.TaggedKeyEchoCount;Escape(marker,attribution);Pump(100);if(info.TaggedKeyEchoCount<=cancelEchoBefore)throw new Exception("Tagged cancel key echo absent");}

   if(info.Interfered||(GetForegroundWindow()!=file&&GetForegroundWindow()!=explorer))throw new Exception("Input/focus changed before release");
   AssertOwners(layout);AssertDesktop();info.ReleaseUnixMs=DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();Move(targetX,targetY,marker,attribution,4);down=false;Pump(50);info.ReleasePoint=cancel?"cancel":"target";info.Outcome=cancel?"cancelled-input":"drop-input";
  }catch(Exception e){info.Error=e.Message;}
  finally{
   if(down){
    // No key to a foreign foreground, and no abort commit to Explorer.
    try{AssertOwners(layout);AssertDesktop();IntPtr fg=GetForegroundWindow();if(fg==file||fg==explorer){Escape(marker,attribution);info.EmergencyEscaped=true;Pump(80);}
     if(GetAncestor(WindowFromPoint(new POINT{X=x,Y=y}),2)!=file)throw new Exception("No owned emergency release point");
     Move(x,y,marker,attribution);Mouse(4,marker,attribution);down=false;info.EmergencyMayHaveOfferedSource=!info.EmergencyEscaped;info.ReleasePoint="emergency-source";
    }catch{info.ReleasePoint="emergency-release-failed";}
   }
   LASTINPUTINFO after=new LASTINPUTINFO();after.Size=(uint)Marshal.SizeOf(typeof(LASTINPUTINFO));if(GetLastInputInfo(ref after))info.InputAfter=after.Time;
   if(havePrior&&!info.Interfered){try{AssertOwners(layout);AssertDesktop();IntPtr hit=GetAncestor(WindowFromPoint(prior),2);if(hit==file||hit==explorer){Move(prior.X,prior.Y,marker,attribution);Pump(20);info.CursorRestore="restored-owned";}else info.CursorRestore="skipped-nonowned-prior-point";}catch{info.CursorRestore="failed";}}
   if(mouseHook!=IntPtr.Zero)UnhookWindowsHookEx(mouseHook);if(keyHook!=IntPtr.Zero)UnhookWindowsHookEx(keyHook);GC.KeepAlive(mouse);GC.KeepAlive(keyboard);
   IntPtr foreground=GetForegroundWindow();
   if(!info.Interfered&&(foreground==file||foreground==explorer)&&priorForeground!=IntPtr.Zero&&IsWindow(priorForeground)){uint actual;GetWindowThreadProcessId(priorForeground,out actual);if(actual==priorPid&&IsWindowVisible(priorForeground)){try{ActivateWindow(priorForeground);info.FocusRestore="restored";}catch{info.FocusRestore="failed";}}else info.FocusRestore="prior-window-gone";}
   else info.FocusRestore=info.Interfered?"interfered-focus-left-unchanged":"not-restored-foreground-changed";
   SetThreadDpiAwarenessContext(old);
  }
  return info;
 }
}
