# NEW reviewed scope: two owned Itda file windows; one exact fixture Copy prompt.
# No Explorer, user profile, clipboard, UAC, or non-fixture prompt approval.
param([Parameter(Mandatory)][string]$Root,[Parameter(Mandatory)][int]$AppPid,[Parameter(Mandatory)][string]$CreationTicks,[Parameter(Mandatory)][long]$MainHwnd)
$ErrorActionPreference='Stop'
if($env:GITHUB_ACTIONS -ne 'true' -or $env:ITDA_ENABLE_OWNED_INPUT -ne 'true' -or $env:ITDA_EXPECTED_SOURCE_SHA -notmatch '^[0-9a-f]{40}$' -or $env:ITDA_EXPECTED_SOURCE_SHA -cne $env:GITHUB_SHA){throw 'Native input gate closed'}
$canonical=& pwsh -NoProfile -NonInteractive -File "$PSScriptRoot/windows-native-idle-control.ps1" -Operation workspace -Root $Root | ConvertFrom-Json
if($LASTEXITCODE){throw 'Invalid owned workspace'}
$Root=$canonical.root
Add-Type -Path "$PSScriptRoot/fixtures/windows-crosshost-driver.cs"
function Publish([string]$Name,[object]$Value) {
 if($Name -notin @('layout.json','reply.json','controller-error.json','controller-cleanup.json')){throw 'Unknown receipt'}
 $tmp=Join-Path $Root ('.'+$Name+'.'+[Guid]::NewGuid().ToString('N')+'.tmp')
 try{[IO.File]::WriteAllText($tmp,(ConvertTo-Json -InputObject $Value -Depth 12 -EnumsAsStrings),[Text.UTF8Encoding]::new($false));[IO.File]::Move($tmp,(Join-Path $Root $Name),$true)}finally{if(Test-Path -LiteralPath $tmp){Remove-Item -LiteralPath $tmp}}
}
function Own-App {
 $s=& pwsh -NoProfile -NonInteractive -File "$PSScriptRoot/windows-native-idle-control.ps1" -Operation inspect -Root $Root -AppPid $AppPid -CreationTicks $CreationTicks | ConvertFrom-Json
 if($LASTEXITCODE -or -not $s.alive -or $s.pid -ne $AppPid -or $s.creation_ticks -ne $CreationTicks -or $s.session_id -ne (Get-Process -Id $PID).SessionId){throw 'Candidate identity/path/session changed'}
 [ItdaCrossHostWindow]::AssertHomeWindow($AppPid,$MainHwnd)
 $actual=(Get-FileHash -LiteralPath (Join-Path $Root 'itda.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
 if($actual -cne '4b2e3bbe5639e7984348ab002e58d155c131293663a6d594892e04cd4acd33b4'){throw 'Released executable hash changed'}
}
$dragAttempted=$false;$confirmAttempted=$false;$stopped=$false
try {
 Own-App
 $layout=[ItdaCrossHostWindow]::Layout($AppPid,$MainHwnd,'QA source — 파일','QA destination — 파일')
 Publish 'layout.json' $layout
 $until=[DateTime]::UtcNow.AddMinutes(4)
 while([DateTime]::UtcNow -lt $until){
  $file=Join-Path $Root 'command.json'
  if(Test-Path -LiteralPath $file){
   $c=Get-Content -LiteralPath $file -Raw|ConvertFrom-Json;Remove-Item -LiteralPath $file
   $result=$null
   try {
    if($c.request -notmatch '^[a-f0-9-]{36}$'){throw 'Invalid command request identity'}
    if($c.op -eq 'stop'){$stopped=$true;break}
    Own-App;[ItdaCrossHostWindow]::AssertOwners($layout)
    if($c.binding.source_host -cne 'crosshost-source' -or $c.binding.target_host -cne 'crosshost-destination' -or -not $c.binding.source_page -or -not $c.binding.target_page -or $c.binding.source_page -ceq $c.binding.target_page -or -not $c.binding.source_window -or -not $c.binding.target_window -or $c.binding.source_window -ceq $c.binding.target_window){throw 'Fixture host/page/native-window mapping absent'}
    if($c.op -eq 'drag'){
     if($dragAttempted){throw 'No native drag retries'};$dragAttempted=$true;$binding=$c.binding
     $result=[ItdaCrossHostWindow]::Action($layout,[int]$c.x,[int]$c.y,[int]$c.target_x,[int]$c.target_y,$false,0,0)
     if($result.Error -or $result.Interfered -or $result.ReleasePoint -cne 'target'){throw ('Native drag failed: '+$result.Error)}
    }elseif($c.op -eq 'confirm'){
     if(-not $dragAttempted -or $confirmAttempted){throw 'One confirmation only after successful drag'};$confirmAttempted=$true
     if((ConvertTo-Json -Compress $binding) -cne (ConvertTo-Json -Compress $c.binding)){throw 'Fixture mapping changed'}
     if($c.prompt.count -ne 1 -or $c.prompt.other_prompts -ne 0 -or $c.prompt.title -cne '파일 복사' -or $c.prompt.button -cne '복사' -or $c.prompt.policy -cne 'keep' -or $c.prompt.names_count -ne 3 -or $c.prompt.source -cne '/' -or $c.prompt.destination -cne '/' -or $c.prompt.route_destination -cne $c.prompt.destination){throw 'Unexpected confirmation; never approve'}
     if($c.prompt.origin.id -cne $binding.source_host -or $c.prompt.origin.window -cne $binding.source_page -or $c.prompt.origin.side -cne 'remote' -or $c.prompt.origin.path -cne '/' -or $c.prompt.origin.token_sha256 -cnotmatch '^[a-f0-9]{64}$' -or @($c.prompt.origin.names).Count -ne 3){throw 'Observed native origin binding mismatch'}
     $expected=@('payload.bin','정보공개 결정통지서(접수번호_00000000).pdf'.Normalize([Text.NormalizationForm]::FormD),'테스트 폴더'.Normalize([Text.NormalizationForm]::FormD))
     foreach($name in $expected){if(-not ($c.prompt.origin.names -ccontains $name)){throw 'Unexpected generated selection'}}
     if($c.safe_bounds.left -ge $c.safe_bounds.right -or $c.safe_bounds.top -ge $c.safe_bounds.bottom -or $c.safe_x -le $c.safe_bounds.left -or $c.safe_x -ge $c.safe_bounds.right -or $c.safe_y -le $c.safe_bounds.top -or $c.safe_y -ge $c.safe_bounds.bottom -or ($c.x -ge $c.safe_bounds.left -and $c.x -le $c.safe_bounds.right -and $c.y -ge $c.safe_bounds.top -and $c.y -le $c.safe_bounds.bottom)){throw 'Non-choice emergency heading geometry invalid'}
     $result=[ItdaCrossHostWindow]::Action($layout,[int]$c.x,[int]$c.y,[int]$c.x,[int]$c.y,$true,[int]$c.safe_x,[int]$c.safe_y)
     if($result.Error -or $result.Interfered -or $result.ReleasePoint -cne 'confirmation'){throw ('Confirmation failed: '+$result.Error)}
    }else{throw 'Unknown owned command'}
    Publish 'reply.json' @{request=$c.request;result=$result}
   }catch{Publish 'reply.json' @{request=$c.request;error=$_.Exception.Message;result=$result};throw}
  }
  Start-Sleep -Milliseconds 50
 }
 if(-not $stopped){throw 'Controller command deadline exceeded'}
}catch{Publish 'controller-error.json' @{error=$_.Exception.Message};exit 1}
finally{$buttonsUp=[ItdaCrossHostWindow]::ButtonsUp();Publish 'controller-cleanup.json' @{quiescent=$buttonsUp;buttons_up=$buttonsUp;drag_attempted=$dragAttempted;confirmation_attempted=$confirmAttempted;stop_received=$stopped;input_hooks_removed_by_action_finally=$true}}
