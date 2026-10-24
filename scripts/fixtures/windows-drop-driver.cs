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
 public class DragInfo { public string Outcome,ReleasePoint,Error;public bool Interfered,EmergencyEscaped,EmergencyMayHaveOfferedSource;public uint InputBefore,InputAfter,IdleBefore,TickBefore;public string FocusRestore,CursorRestore;public uint PriorForegroundPid;public long StartedUnixMs,ReleaseUnixMs,FirstExplorerUnixMs;public uint[] GuardBefore,GuardAfter; }
 public class LayoutInfo { public long File,Explorer; public int OriginX,OriginY,TargetX,TargetY; public RECT Work,Source,Destination;public uint SourcePid,ExplorerPid,SourceIntegrity,ExplorerIntegrity;public int Session;public string SourceStartedTicks,ExplorerStartedTicks; }
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
 static void Mouse(uint flags,UIntPtr marker){INPUT i=new INPUT();i.Value.Mouse.Flags=flags;i.Value.Mouse.Extra=marker;if(SendInput(1,new INPUT[]{i},Marshal.SizeOf(typeof(INPUT)))!=1)throw new Exception("Mouse input rejected");}
 static void Move(int x,int y,UIntPtr marker,uint buttons=0){int left=GetSystemMetrics(76),top=GetSystemMetrics(77),w=GetSystemMetrics(78),h=GetSystemMetrics(79);if(w<2||h<2)throw new Exception("Virtual desktop unavailable");INPUT i=new INPUT();i.Value.Mouse.X=(int)((long)(x-left)*65535/(w-1));i.Value.Mouse.Y=(int)((long)(y-top)*65535/(h-1));i.Value.Mouse.Flags=0xc001|buttons;i.Value.Mouse.Extra=marker;if(SendInput(1,new INPUT[]{i},Marshal.SizeOf(typeof(INPUT)))!=1)throw new Exception("Move input rejected");}
 static void Escape(UIntPtr marker){INPUT a=new INPUT();a.Type=1;a.Value.Key.Key=0x1b;a.Value.Key.Extra=marker;INPUT b=a;b.Value.Key.Flags=2;if(SendInput(2,new INPUT[]{a,b},Marshal.SizeOf(typeof(INPUT)))!=2)throw new Exception("Cancel input rejected");}
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
  DragInfo info=new DragInfo();info.StartedUnixMs=DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();IntPtr old=SetThreadDpiAwarenessContext(new IntPtr(-4));if(old==IntPtr.Zero)throw new Exception("PMv2 context unavailable");
  bool down=false;POINT prior;bool havePrior=GetCursorPos(out prior);IntPtr file=new IntPtr(source),explorer=new IntPtr(destination),mouseHook=IntPtr.Zero,keyHook=IntPtr.Zero,priorForeground=IntPtr.Zero;uint priorPid=0;
  UIntPtr marker=new UIntPtr((ulong)DateTime.UtcNow.Ticks);HookProc mouse=null,keyboard=null;
  try{
   AssertOwners(layout);AssertDesktop();
   LASTINPUTINFO idle=new LASTINPUTINFO();idle.Size=(uint)Marshal.SizeOf(typeof(LASTINPUTINFO));
   if(!havePrior||!GetLastInputInfo(ref idle))throw new Exception("Console cursor/idle state unavailable");
   info.InputBefore=idle.Time;info.TickBefore=unchecked((uint)Environment.TickCount);info.IdleBefore=unchecked(info.TickBefore-idle.Time);if(info.IdleBefore>Int32.MaxValue||info.IdleBefore<5000)throw new Exception("Console has recent input; no injected drag attempted");
   foreach(int key in new int[]{1,2,4,5,6,16,17,18,91,92})if((GetAsyncKeyState(key)&0x8000)!=0)throw new Exception("Input button/modifier already down");
   priorForeground=GetForegroundWindow();GetWindowThreadProcessId(priorForeground,out priorPid);info.PriorForegroundPid=priorPid;
   // Monitor only attribution, never record key values or cursor data. Every
   // injected event is tagged; any other event invalidates this run.
   mouse=(code,w,l)=>{if(code>=0&&((MOUSEHOOK)Marshal.PtrToStructure(l,typeof(MOUSEHOOK))).Extra!=marker)info.Interfered=true;return CallNextHookEx(IntPtr.Zero,code,w,l);};
   keyboard=(code,w,l)=>{if(code>=0&&((KEYHOOK)Marshal.PtrToStructure(l,typeof(KEYHOOK))).Extra!=marker)info.Interfered=true;return CallNextHookEx(IntPtr.Zero,code,w,l);};
   mouseHook=SetWindowsHookEx(14,mouse,GetModuleHandle(null),0);keyHook=SetWindowsHookEx(13,keyboard,GetModuleHandle(null),0);
   if(mouseHook==IntPtr.Zero||keyHook==IntPtr.Zero)throw new Exception("Input attribution hooks unavailable");
   // The desktop is idle and attribution is active. Raise only our two test
   // windows, then focus the source; never send raw input to the old foreground.
   if(!SetWindowPos(file,IntPtr.Zero,0,0,0,0,0x13))throw new Exception("Owned source could not be raised");
   try{ActivateWindow(file);}catch{}
   if(GetForegroundWindow()!=file){
    RECT bounds;if(!GetWindowRect(file,out bounds))throw new Exception("Owned source geometry unavailable");
    // Owned blank status-bar space, never a foreign foreground window or title-bar button.
    int ax=bounds.Right-32,ay=bounds.Bottom-18;AssertOwners(layout);AssertDesktop();
    if(GetAncestor(WindowFromPoint(new POINT{X=ax,Y=ay}),2)!=file)throw new Exception("Activation point is not owned");
    Move(ax,ay,marker);Pump(30);if(info.Interfered)throw new Exception("External input during activation");
    if(GetAncestor(WindowFromPoint(new POINT{X=ax,Y=ay}),2)!=file)throw new Exception("Activation point changed");
    Mouse(2,marker);down=true;Mouse(4,marker);down=false;Pump(80);
    if(GetForegroundWindow()!=file)throw new Exception("Owned source activation refused");
   }
   if(!SetWindowPos(explorer,IntPtr.Zero,0,0,0,0,0x13))throw new Exception("Owned Explorer could not be raised");
   Pump(50);if(info.Interfered)throw new Exception("External console input during focus");
   if(GetAncestor(WindowFromPoint(new POINT{X=x,Y=y}),2)!=file||GetAncestor(WindowFromPoint(new POINT{X=targetX,Y=targetY}),2)!=explorer)throw new Exception("Drop coordinates do not belong to owned windows");
   Move(x,y,marker);Pump(80);if(info.Interfered||GetForegroundWindow()!=file)throw new Exception("Input/focus changed before mouse down");Move(x,y,marker,2);down=true;
   for(int i=1;i<=120;i++){
    int nx=x+(targetX-x)*i/120,ny=y+(targetY-y)*i/120;
    AssertOwners(layout);IntPtr hit=GetAncestor(WindowFromPoint(new POINT{X=nx,Y=ny}),2);
    if(hit!=file&&hit!=explorer)throw new Exception("Drag path left owned windows");
    if(GetForegroundWindow()!=file&&GetForegroundWindow()!=explorer)throw new Exception("Foreground left owned windows");
    if(hit==explorer&&info.FirstExplorerUnixMs==0)info.FirstExplorerUnixMs=DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();AssertDesktop();Move(nx,ny,marker);Pump(12);if(info.Interfered)throw new Exception("External console input detected");
   }
   Pump(500);if(info.Interfered)throw new Exception("External console input detected");
   if(GetAncestor(WindowFromPoint(new POINT{X=targetX,Y=targetY}),2)!=explorer)throw new Exception("Drop target changed");
   if(cancel){Escape(marker);Pump(100);}

   if(info.Interfered||(GetForegroundWindow()!=file&&GetForegroundWindow()!=explorer))throw new Exception("Input/focus changed before release");
   AssertOwners(layout);AssertDesktop();info.ReleaseUnixMs=DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();Move(targetX,targetY,marker,4);down=false;Pump(50);info.ReleasePoint=cancel?"cancel":"target";info.Outcome=cancel?"cancelled-input":"drop-input";
  }catch(Exception e){info.Error=e.Message;}
  finally{
   if(down){
    // No key to a foreign foreground, and no abort commit to Explorer.
    try{AssertOwners(layout);AssertDesktop();IntPtr fg=GetForegroundWindow();if(fg==file||fg==explorer){Escape(marker);info.EmergencyEscaped=true;Pump(80);}
     if(GetAncestor(WindowFromPoint(new POINT{X=x,Y=y}),2)!=file)throw new Exception("No owned emergency release point");
     Move(x,y,marker);Mouse(4,marker);down=false;info.EmergencyMayHaveOfferedSource=!info.EmergencyEscaped;info.ReleasePoint="emergency-source";
    }catch{info.ReleasePoint="emergency-release-failed";}
   }
   LASTINPUTINFO after=new LASTINPUTINFO();after.Size=(uint)Marshal.SizeOf(typeof(LASTINPUTINFO));if(GetLastInputInfo(ref after))info.InputAfter=after.Time;
   if(havePrior&&!info.Interfered){try{AssertOwners(layout);AssertDesktop();IntPtr hit=GetAncestor(WindowFromPoint(prior),2);if(hit==file||hit==explorer){Move(prior.X,prior.Y,marker);Pump(20);info.CursorRestore="restored-owned";}else info.CursorRestore="skipped-nonowned-prior-point";}catch{info.CursorRestore="failed";}}
   if(mouseHook!=IntPtr.Zero)UnhookWindowsHookEx(mouseHook);if(keyHook!=IntPtr.Zero)UnhookWindowsHookEx(keyHook);GC.KeepAlive(mouse);GC.KeepAlive(keyboard);
   IntPtr foreground=GetForegroundWindow();
   if(!info.Interfered&&(foreground==file||foreground==explorer)&&priorForeground!=IntPtr.Zero&&IsWindow(priorForeground)){uint actual;GetWindowThreadProcessId(priorForeground,out actual);if(actual==priorPid&&IsWindowVisible(priorForeground)){try{ActivateWindow(priorForeground);info.FocusRestore="restored";}catch{info.FocusRestore="failed";}}else info.FocusRestore="prior-window-gone";}
   else info.FocusRestore=info.Interfered?"interfered-focus-left-unchanged":"not-restored-foreground-changed";
   SetThreadDpiAwarenessContext(old);
  }
  return info;
 }
}
