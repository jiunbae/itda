# One owned Explorer window and tagged input command loop. Invoked only after explicit CI gate.
param([Parameter(Mandatory)][string]$Root,[Parameter(Mandatory)][int]$AppPid,[Parameter(Mandatory)][string]$CreationTicks,[Parameter(Mandatory)][long]$MainHwnd)
$ErrorActionPreference='Stop'
if($env:GITHUB_ACTIONS -ne 'true' -or $env:ITDA_ENABLE_OWNED_INPUT -ne 'true' -or $env:ITDA_EXPECTED_SOURCE_SHA -notmatch '^[0-9a-f]{40}$' -or $env:ITDA_EXPECTED_SOURCE_SHA -cne $env:GITHUB_SHA){throw 'Native input gate closed'}
$canonical=& pwsh -NoProfile -NonInteractive -File "$PSScriptRoot/windows-native-idle-control.ps1" -Operation workspace -Root $Root | ConvertFrom-Json
if($LASTEXITCODE){throw 'Invalid owned workspace'}
$Root=$canonical.root
Add-Type -Path "$PSScriptRoot/fixtures/windows-drop-driver.cs"
$folder=Join-Path $Root 'drop-destination'
if(-not (Test-Path -LiteralPath $folder -PathType Container) -or (Get-Item -LiteralPath $folder).Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Invalid destination'}
$explorer=$null;$layout=$null;$shell=$null;$explorerCreationAttempted=$false
function Publish-Receipt {
 param([ValidateSet('layout.json','controller-error.json','reply.json','controller-cleanup.json')][string]$Name,[object]$Value,[int]$Depth=10)
 # Serialize before touching disk. Publish only a closed, complete UTF-8 file
 # through a same-directory rename; the strict reader never retries partial JSON.
 $json=ConvertTo-Json -InputObject $Value -Depth $Depth -EnumsAsStrings
 $destination=Join-Path $Root $Name
 $temporary=Join-Path $Root ('.'+$Name+'.'+[Guid]::NewGuid().ToString('N')+'.tmp')
 try {
  [IO.File]::WriteAllText($temporary,$json,[Text.UTF8Encoding]::new($false))
  [IO.File]::Move($temporary,$destination,$true)
 } finally {if(Test-Path -LiteralPath $temporary){Remove-Item -LiteralPath $temporary}}
}
function Own-App {
 $state=& pwsh -NoProfile -NonInteractive -File "$PSScriptRoot/windows-native-idle-control.ps1" -Operation inspect -Root $Root -AppPid $AppPid -CreationTicks $CreationTicks | ConvertFrom-Json
 if($LASTEXITCODE -or -not $state.alive -or $state.pid -ne $AppPid -or $state.creation_ticks -ne $CreationTicks -or $state.session_id -ne (Get-Process -Id $PID).SessionId){throw 'Candidate identity changed'}
 [ItdaQualityWindow]::AssertHomeWindow($AppPid,$MainHwnd)
}
try {
 Own-App
 $shell=New-Object -ComObject Shell.Application
 # This creates normal ShellBag/view-state on the disposable runner only, explicitly in scope.
 $explorerCreationAttempted=$true
 $shell.Explore($folder)
 $until=[DateTime]::UtcNow.AddSeconds(15)
 do{$matches=@($shell.Windows()|Where-Object{try{[IO.Path]::GetFullPath($_.Document.Folder.Self.Path) -eq $folder}catch{$false}});if($matches.Count -eq 1){$explorer=$matches[0];break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $until)
 if(-not $explorer){throw 'Unique owned Explorer Cabinet unavailable'}
 $layout=[ItdaQualityWindow]::Layout($AppPid,$MainHwnd,[long]$explorer.HWND)
 Publish-Receipt -Name 'layout.json' -Value $layout -Depth 8
 $until=[DateTime]::UtcNow.AddMinutes(4)
 while([DateTime]::UtcNow -lt $until){
  $file=Join-Path $Root 'command.json'
  if(Test-Path -LiteralPath $file){
   $c=Get-Content -LiteralPath $file -Raw|ConvertFrom-Json;Remove-Item -LiteralPath $file
   try {
    if($c.op -eq 'stop'){break}
    Own-App
    if([long]$explorer.HWND -ne $layout.Explorer -or [IO.Path]::GetFullPath($explorer.Document.Folder.Self.Path) -ne $folder){throw 'Explorer ownership/path changed'}
    if($c.op -ne 'drag' -or $c.scenario -notin @('cancel','positive')){throw 'Unknown owned command'}
    $before=[ItdaQualityWindow]::ProbeIdle()
    $result=[ItdaQualityWindow]::Drag($layout,[int]$c.x,[int]$c.y,($c.scenario -eq 'cancel'))
    $result.GuardBefore=$before;$result.GuardAfter=[ItdaQualityWindow]::ProbeIdle()
    $reply=@{request=$c.request;result=$result}
   }catch{$reply=@{request=$c.request;error=$_.Exception.Message}}
   Publish-Receipt -Name 'reply.json' -Value $reply
  }
  Start-Sleep -Milliseconds 50
 }
} catch { Publish-Receipt -Name 'controller-error.json' -Value @{error=$_.Exception.Message};exit 1 }
finally {
 $cleanup=@{explorer_creation_attempted=$explorerCreationAttempted;explorer_state='unconfirmed';explorer_close_requested=$false;no_owned_window_confirmed=$false}
 try {if($explorer -and [IO.Path]::GetFullPath($explorer.Document.Folder.Self.Path) -eq $folder){
  $handle=[long]$explorer.HWND;$explorer.Quit();$cleanup.explorer_close_requested=$true;$until=[DateTime]::UtcNow.AddSeconds(10)
  do{$remaining=@($shell.Windows()|Where-Object{try{[long]$_.HWND -eq $handle -or [IO.Path]::GetFullPath($_.Document.Folder.Self.Path) -eq $folder}catch{$false}});if(-not $remaining.Count){$cleanup.explorer_state='closed';$cleanup.no_owned_window_confirmed=$true;break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $until)
 }else{
  # Even when Own-App fails before Explore, record an actual read-only absence
  # check. Do not close unpinned or ambiguous windows found during cleanup.
  if(-not $shell){$shell=New-Object -ComObject Shell.Application}
  $remaining=@($shell.Windows()|Where-Object{try{[IO.Path]::GetFullPath($_.Document.Folder.Self.Path) -eq $folder}catch{$false}})
  if(-not $remaining.Count){$cleanup.explorer_state=if($explorerCreationAttempted){'not-present'}else{'not-created'};$cleanup.no_owned_window_confirmed=$true}
 }}
 catch {$cleanup.error=$_.Exception.Message}
 Publish-Receipt -Name 'controller-cleanup.json' -Value $cleanup
}
