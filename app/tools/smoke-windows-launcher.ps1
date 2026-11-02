#requires -Version 7.2
<#
Run the exact distributed .cmd through real cmd.exe in a private console. Inputs
are generated, and only bounded assertions are persisted. URLs, session tokens,
raw stdout and stderr stay in anonymous pipes and process memory.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ZipPath,
    [Parameter(Mandatory)][string]$FixtureExecutable,
    [Parameter(Mandatory)][string]$ProbeExecutable,
    [string]$WorkDirectory
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'The real CMD launcher smoke requires Windows.' }

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function New-ProbeProcess([string[]]$Arguments) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $script:ProbeExecutable
    $info.UseShellExecute = $false
    $info.RedirectStandardInput = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $info.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    $info.CreateNoWindow = $true
    foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    Assert-True ($process.Start()) 'Cannot start the CI-only launcher probe.'
    return $process
}

function Start-Entry([string]$State, [string]$BrowserRecords, [int]$CodePage = 65001, [string[]]$Extra = @()) {
    $requestPath = Join-Path $script:requests "$([guid]::NewGuid().ToString('N')).json"
    $request = @{
        entry = $script:entry
        arguments = @('--state-dir', $State, '--browser-command', $script:ProbeExecutable) + $Extra
        cwd = $script:unrelated
        code_page = $CodePage
        environment = @{ LOCALAPPDATA = $script:localData; VOUCHER_BROWSER_PROBE_DIR = $BrowserRecords }
    }
    $request | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $requestPath -Encoding utf8NoBOM
    $process = New-ProbeProcess @('spawn-cmd', $requestPath)
    $buffer = [char[]]::new(4096)
    $entry = [pscustomobject]@{
        Process = $process; CmdPid = 0; State = $State; BrowserRecords = $BrowserRecords
        Buffer = $buffer; Read = $process.StandardOutput.ReadAsync($buffer, 0, $buffer.Length)
        ErrorRead = $process.StandardError.ReadToEndAsync(); ErrorText = ''; Ended = $false
        Output = [Text.StringBuilder]::new(); BaseUrl = ''; Token = ''
    }
    $script:owned.Add($entry)
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    while ($entry.CmdPid -eq 0 -and [DateTime]::UtcNow -lt $deadline) {
        Pump-Entry $entry 50
        if ($entry.Output.ToString() -match 'PROBE_CMD_PID=(\d+)') { $entry.CmdPid = [int]$Matches[1] }
        if ($entry.Process.HasExited -and $entry.CmdPid -eq 0) { throw 'The isolated CMD console could not be created.' }
    }
    Assert-True ($entry.CmdPid -gt 0) 'The isolated CMD process did not report its PID.'
    return $entry
}

function Pump-Entry($Entry, [int]$WaitMs = 0) {
    if ($Entry.Ended) { return }
    if (-not $Entry.Read.IsCompleted -and $WaitMs -gt 0) { [void]$Entry.Read.Wait($WaitMs) }
    while ($Entry.Read.IsCompleted -and -not $Entry.Ended) {
        $count = $Entry.Read.GetAwaiter().GetResult()
        if ($count -eq 0) { $Entry.Ended = $true; break }
        [void]$Entry.Output.Append($Entry.Buffer, 0, $count)
        Assert-True ($Entry.Output.Length -lt 131072) 'Launcher output exceeded its bounded in-memory capture.'
        $Entry.Read = $Entry.Process.StandardOutput.ReadAsync($Entry.Buffer, 0, $Entry.Buffer.Length)
    }
}

function Read-EntryErrors($Entry) {
    if ($Entry.Process.HasExited) {
        Pump-Entry $Entry 0
        $Entry.ErrorText = $Entry.ErrorRead.GetAwaiter().GetResult()
    }
}

function Assert-CmdParsed($Entry) {
    $text = $Entry.Output.ToString() + $Entry.ErrorText
    Assert-True ($text -notmatch 'is not recognized as an internal or external command|was unexpected at this time|The syntax of the command is incorrect') 'The packaged CMD emitted a command parsing error.'
}

function Wait-Exit($Entry, [int]$Seconds = 25) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while (-not $Entry.Process.HasExited -and [DateTime]::UtcNow -lt $deadline) { Pump-Entry $Entry 50 }
    Assert-True ($Entry.Process.HasExited) 'A launcher process did not exit within the expected interval.'
    [void]$Entry.Process.WaitForExit(1000)
    Read-EntryErrors $Entry
    Assert-CmdParsed $Entry
}

function Wait-Reused($Entry) {
    Wait-Exit $Entry
    Assert-True ($Entry.Process.ExitCode -eq 0) 'Reopening an existing workbench must exit successfully.'
    Assert-True ($Entry.Output.ToString() -notmatch '/#token=') 'A secondary launcher disclosed the primary session URL.'
}

function Invoke-Api($Service, [string]$Method, [string]$Path, $Body = $null) {
    $parameters = @{
        Uri = "$($Service.BaseUrl)/api$Path"; Method = $Method
        Headers = @{ 'X-Session-Token' = $Service.Token }
        TimeoutSec = 60; SkipHttpErrorCheck = $true; NoProxy = $true
    }
    if ($null -ne $Body) {
        $parameters.ContentType = 'application/json; charset=utf-8'
        $parameters.Body = [Text.Encoding]::UTF8.GetBytes(($Body | ConvertTo-Json -Depth 20 -Compress))
    }
    $response = Invoke-WebRequest @parameters
    Assert-True ([int]$response.StatusCode -eq 200) "Synthetic API $Method $Path returned an unexpected status."
    return $response.Content | ConvertFrom-Json -Depth 100
}

function Wait-Service($Entry) {
    $deadline = [DateTime]::UtcNow.AddSeconds(180)
    while ([DateTime]::UtcNow -lt $deadline) {
        Pump-Entry $Entry 50
        if ($Entry.Output.ToString() -match '(http://127\.0\.0\.1:\d+)/#token=([0-9a-f]{64})') {
            $Entry.BaseUrl = $Matches[1]
            $Entry.Token = $Matches[2]
            $status = Invoke-Api $Entry 'GET' '/status'
            Assert-True ($status.model_ready -eq $true) 'The launch entry did not load its packaged OCR runtime.'
            Assert-True ($status.version -eq $script:ExpectedVersion) 'Launcher acceptance must run against the selected package version.'
            return
        }
        if ($Entry.Process.HasExited) { throw 'The primary launcher exited before becoming ready.' }
    }
    throw 'The primary launcher did not become ready within 180 seconds.'
}

function Wait-Locked([string]$State) {
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while ([DateTime]::UtcNow -lt $deadline) {
        $result = & $script:ProbeExecutable 'lock-held' $State
        Assert-True ($LASTEXITCODE -eq 0) 'The lock synchronization probe failed.'
        if ((($result -join "`n") | ConvertFrom-Json).held) { return }
        Start-Sleep -Milliseconds 20
    }
    throw 'The first launcher did not acquire the state lock in time.'
}

function Get-BrowserRecords([string]$Directory) {
    if (-not (Test-Path -LiteralPath $Directory)) { return @() }
    return @(Get-ChildItem -LiteralPath $Directory -File -Filter '*.json' | ForEach-Object {
        Get-Content -LiteralPath $_.FullName -Raw -Encoding utf8 | ConvertFrom-Json
    })
}

function Wait-Browsers([string]$Directory, [int]$Count) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while ([DateTime]::UtcNow -lt $deadline) {
        $records = @(Get-BrowserRecords $Directory)
        if ($records.Count -ge $Count) {
            Assert-True ($records.Count -eq $Count) 'The launcher opened more browser windows than requested.'
            Assert-True (@($records | Where-Object { -not $_.loopback -or -not $_.has_token -or $_.token_length -ne 64 }).Count -eq 0) 'A browser request was missing its local authenticated URL.'
            return $records
        }
        Start-Sleep -Milliseconds 50
    }
    throw 'The expected browser requests were not observed.'
}

function Assert-OneOwner($Records, [int]$Expected = 0) {
    $owners = @($Records | Select-Object -ExpandProperty owner_pid -Unique)
    Assert-True ($owners.Count -eq 1 -and $owners[0] -gt 0) 'Secondary launches did not delegate browser opening to one primary service.'
    if ($Expected -gt 0) { Assert-True ($owners[0] -eq $Expected) 'The original primary process did not reopen the browser.' }
    return [int]$owners[0]
}

function Stop-Entry($Entry, [switch]$Graceful) {
    if ($null -eq $Entry -or $Entry.Process.HasExited) { return }
    if ($Graceful) {
        # Answer cmd.exe's optional batch-termination prompt through its own
        # redirected input. CTRL+C targets only the separately created console.
        $Entry.Process.StandardInput.WriteLine('y')
        $result = & $script:ProbeExecutable 'ctrl-c' ([string]$Entry.CmdPid)
        Assert-True ($LASTEXITCODE -eq 0 -and (($result -join "`n") | ConvertFrom-Json).sent) 'Could not deliver CTRL+C to the owned isolated console.'
        Wait-Exit $Entry 40
    }
    else {
        $Entry.Process.Kill($true)
        Assert-True ($Entry.Process.WaitForExit(15000)) 'The owned test process tree did not terminate.'
        Read-EntryErrors $Entry
    }
}

function Wait-Failure($Entry, [string]$ReasonPattern) {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    $paused = $false
    while ([DateTime]::UtcNow -lt $deadline -and -not $Entry.Process.HasExited) {
        Pump-Entry $Entry 50
        if ($Entry.Output.ToString() -match 'Press any key|任意键') { $paused = $true; break }
    }
    Assert-True ($paused -and -not $Entry.Process.HasExited) 'A real launch failure did not preserve its CMD window at pause.'
    $Entry.Process.StandardInput.WriteLine('')
    Wait-Exit $Entry
    Assert-True ($Entry.Process.ExitCode -ne 0) 'A real launch failure was incorrectly reported as success.'
    $text = $Entry.Output.ToString() + $Entry.ErrorText
    Assert-True ($text -match $ReasonPattern) 'The launch failure did not report its distinct Chinese cause.'
    Assert-True ($text -match '请|重新|重试|检查|联系') 'The launch failure did not include an actionable Chinese next step.'
}

function Wait-Job($Service, $Job) {
    $deadline = [DateTime]::UtcNow.AddSeconds(300)
    while ($Job.status -in @('queued', 'running')) {
        Assert-True ([DateTime]::UtcNow -lt $deadline) 'Synthetic OCR/generation exceeded 300 seconds.'
        Start-Sleep -Milliseconds 150
        $Job = Invoke-Api $Service 'GET' "/jobs/$($Job.id)"
    }
    Assert-True ($Job.status -eq 'completed' -and $Job.done -eq $Job.total) 'A synthetic job did not complete normally.'
    return $Job
}

function Assert-Preserved($Service, [string]$PlanId, [string]$JobId, [int]$Threads, $Hashes) {
    $settings = Invoke-Api $Service 'GET' '/settings'
    Assert-True ($settings.threads -eq $Threads) 'The thread setting did not survive entry-based restart.'
    $plan = Invoke-Api $Service 'GET' "/plans/$PlanId"
    Assert-True ($plan.orientation_workflow.analysis_completed) 'The completed analysis stage did not survive restart.'
    $images = @($plan.vouchers | ForEach-Object { $_.files } | Where-Object kind -EQ 'image')
    Assert-True ($images.Count -eq 12 -and @($images | Where-Object reviewed -EQ $true).Count -eq 12) 'Manual confirmations were not preserved.'
    foreach ($file in $images) {
        $angle = if ($file.sequence -eq 2) { 270 } else { 0 }
        Assert-True ($file.rotation_cw -eq $angle) 'A manually corrected rotation was lost.'
    }
    $job = Invoke-Api $Service 'GET' "/jobs/$JobId"
    Assert-True ($job.status -eq 'completed' -and @($job.results).Count -eq 2) 'Completed history was not preserved.'
    foreach ($path in $Hashes.Keys) {
        Assert-True ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -eq $Hashes[$path]) 'A restart changed an existing generated PDF.'
    }
}

$ZipPath = (Resolve-Path -LiteralPath $ZipPath).Path
$FixtureExecutable = (Resolve-Path -LiteralPath $FixtureExecutable).Path
$ProbeExecutable = (Resolve-Path -LiteralPath $ProbeExecutable).Path
if (-not $WorkDirectory) { $WorkDirectory = Join-Path ([IO.Path]::GetTempPath()) ("voucher-launcher-" + [guid]::NewGuid().ToString('N')) }
$WorkDirectory = [IO.Path]::GetFullPath($WorkDirectory)
Assert-True (-not (Test-Path -LiteralPath $WorkDirectory)) 'Launcher smoke directory already exists; refusing to overwrite.'
[void](New-Item -ItemType Directory -Path $WorkDirectory)
$owned = [Collections.Generic.List[object]]::new()
$holders = [Collections.Generic.List[object]]::new()
$requests = Join-Path $WorkDirectory 'requests'
$unrelated = Join-Path $WorkDirectory '其他 中文 工作目录'
$localData = Join-Path $WorkDirectory '隔离 LocalAppData'
[void](New-Item -ItemType Directory -Path $requests, $unrelated)
$summary = [ordered]@{
    passed = $false; synthetic_only = $true; version = $null
    package_sha256 = (Get-FileHash -LiteralPath $ZipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    code_pages = @(936, 65001); checks = @()
}
$denied = $null
$savedAcl = $null
$listener = $null

try {
    $expanded = Join-Path $WorkDirectory '中文 包含空格 解压目录'
    Expand-Archive -LiteralPath $ZipPath -DestinationPath $expanded
    $entries = @(Get-ChildItem -LiteralPath $expanded -Recurse -File -Filter '启动凭证工作台.cmd')
    Assert-True ($entries.Count -eq 1) 'ZIP must contain exactly one distributed CMD entry.'
    $entry = $entries[0].FullName
    $script:ExpectedVersion = [string](Get-Content (Join-Path $entries[0].DirectoryName 'package-manifest.json') -Raw -Encoding utf8 | ConvertFrom-Json).version
    $summary.version = $script:ExpectedVersion
    $bytes = [IO.File]::ReadAllBytes($entry)
    Assert-True (@($bytes | Where-Object { $_ -gt 127 }).Count -eq 0) 'The package CMD must contain only ASCII bytes.'
    $ascii = [Text.Encoding]::ASCII.GetString($bytes)
    Assert-True ($ascii -match "`r`n" -and $ascii -notmatch "(?<!`r)`n|`r(?!`n)") 'The package CMD must use CRLF exclusively.'
    Assert-True ($ascii -match '%\*' -and $ascii -match '(?i)pause' -and $ascii -match '(?i)exit /b') 'The CMD entry did not forward arguments and preserve failure status.'
    $summary.checks += 'distributed-cmd-ascii-crlf-and-argument-forwarding'

    foreach ($codePage in @(936, 65001)) {
        Write-Host "Launcher scenario: first/repeated/starting and graceful restart, code page $codePage."
        $state = Join-Path $WorkDirectory "中文 状态 $codePage"
        $records = Join-Path $WorkDirectory "browser-$codePage"
        $primary = Start-Entry $state $records $codePage @('--startup-delay-ms', '2000')
        Wait-Locked $state
        $duringStart = Start-Entry $state $records $codePage
        Pump-Entry $duringStart 200
        Assert-True (-not $duringStart.Process.HasExited) 'A second click exited before the delayed primary became ready.'
        Assert-True (@(Get-BrowserRecords $records).Count -eq 0) 'A delayed startup opened a browser before readiness.'
        Wait-Service $primary
        Wait-Reused $duringStart
        $owner = Assert-OneOwner @(Wait-Browsers $records 2)
        for ($repeat = 0; $repeat -lt 2; $repeat++) {
            $again = Start-Entry $state $records $codePage
            Wait-Reused $again
        }
        [void](Assert-OneOwner @(Wait-Browsers $records 4) $owner)
        $ping = Start-Entry $state $records $codePage @('--no-browser')
        Wait-Reused $ping
        Assert-True (@(Get-BrowserRecords $records).Count -eq 4) '--no-browser unexpectedly reopened the browser.'
        [void](Invoke-Api $primary 'PATCH' '/settings' @{ threads = 7 })
        Stop-Entry $primary -Graceful
        $restart = Start-Entry $state $records $codePage
        Wait-Service $restart
        Assert-True ((Invoke-Api $restart 'GET' '/settings').threads -eq 7) 'Normal CTRL+C exit did not preserve settings.'
        [void](Wait-Browsers $records 5)
        Stop-Entry $restart -Graceful
        $summary.checks += "real-cmd-$codePage-first-repeat-starting-ping-and-ctrl-c-restart"
    }

    Write-Host 'Launcher scenario: simultaneous launches and active-task reopening.'
    $state = Join-Path $WorkDirectory '同时启动 状态'
    $records = Join-Path $WorkDirectory 'browser-concurrent'
    $one = Start-Entry $state $records 936 @('--startup-delay-ms', '1000')
    $two = Start-Entry $state $records 936 @('--startup-delay-ms', '1000')
    $deadline = [DateTime]::UtcNow.AddSeconds(180)
    $primary = $null
    while ($null -eq $primary -and [DateTime]::UtcNow -lt $deadline) {
        foreach ($candidate in @($one, $two)) {
            Pump-Entry $candidate 20
            if ($candidate.Output.ToString() -match '/#token=') { $primary = $candidate; break }
        }
    }
    Assert-True ($null -ne $primary) 'Neither simultaneous CMD launch became the primary service.'
    Wait-Service $primary
    $secondary = if ($primary -eq $one) { $two } else { $one }
    Wait-Reused $secondary
    $owner = Assert-OneOwner @(Wait-Browsers $records 2)
    Assert-True (-not $primary.Process.HasExited) 'The primary exited during concurrent activation.'
    $summary.checks += 'simultaneous-real-cmd-launches-single-service-and-primary-browser-owner'

    $fixtureRoot = Join-Path $WorkDirectory '合成资料 工作中双击'
    $fixtureOutput = & $FixtureExecutable 'create' $fixtureRoot
    Assert-True ($LASTEXITCODE -eq 0 -and (($fixtureOutput -join "`n") | ConvertFrom-Json).synthetic) 'Could not generate isolated synthetic inputs.'
    $firstVoucher = Join-Path $fixtureRoot '合成分项/260101-00001'
    # Twelve images make the busy interval observable even on a fast CI runner.
    foreach ($sequence in 4..11) {
        Copy-Item -LiteralPath (Join-Path $firstVoucher '2601_00001_1.jpg') -Destination (Join-Path $firstVoucher "2601_00001_$sequence.jpg")
    }
    [void](Invoke-Api $primary 'PATCH' '/settings' @{ threads = 1 })
    $plan = Invoke-Api $primary 'POST' '/scan' @{ root = $fixtureRoot }
    $planId = [string]$plan.id
    $plan = Invoke-Api $primary 'PATCH' "/plans/$planId/orientation" @{ enabled = $true; accept_disclaimer = $true }
    $analysis = Invoke-Api $primary 'POST' "/plans/$planId/analyze"
    $before = Invoke-Api $primary 'GET' "/jobs/$($analysis.id)"
    Assert-True ($before.status -in @('queued', 'running') -and $before.total -eq 12) 'The synthetic busy scenario was not active.'
    $busyClick = Start-Entry $state $records 65001
    Wait-Reused $busyClick
    $after = Invoke-Api $primary 'GET' "/jobs/$($analysis.id)"
    Assert-True ($after.status -in @('queued', 'running', 'completed') -and $after.done -ge $before.done) 'A repeated click interrupted the active task or reset progress.'
    $analysis = Wait-Job $primary $after
    Assert-True ($analysis.total -eq 12) 'The active OCR task lost pages during repeat launch.'
    [void](Assert-OneOwner @(Wait-Browsers $records 3) $owner)
    $plan = Invoke-Api $primary 'GET' "/plans/$planId"
    foreach ($file in @($plan.vouchers | ForEach-Object { $_.files } | Where-Object kind -EQ 'image')) {
        $angle = if ($file.sequence -eq 2) { 270 } else { 0 }
        [void](Invoke-Api $primary 'PATCH' "/plans/$planId/files/$($file.id)" @{ rotation_cw = $angle; reviewed = $true })
    }
    [void](Invoke-Api $primary 'PATCH' '/settings' @{ threads = 4 })
    $generated = Wait-Job $primary (Invoke-Api $primary 'POST' "/plans/$planId/generate" @{})
    Assert-True ($generated.total -eq 16 -and @($generated.results).Count -eq 2) 'Synthetic generation lost source pages.'
    $hashes = @{}
    foreach ($result in $generated.results) { $hashes[$result.output_path] = (Get-FileHash -LiteralPath $result.output_path -Algorithm SHA256).Hash }
    Stop-Entry $primary -Graceful
    $primary = Start-Entry $state $records
    Wait-Service $primary
    Assert-Preserved $primary $planId $generated.id 4 $hashes
    $summary.checks += 'busy-task-repeat-keeps-progress-and-normal-restart-keeps-history-reviews-settings'

    Write-Host 'Launcher scenario: forced process exit and interrupted-task recovery.'
    [void](Invoke-Api $primary 'PATCH' '/settings' @{ threads = 1 })
    $crashPlan = Invoke-Api $primary 'POST' '/scan' @{ root = $fixtureRoot }
    [void](Invoke-Api $primary 'PATCH' "/plans/$($crashPlan.id)/orientation" @{ enabled = $true; accept_disclaimer = $true })
    $crashJob = Invoke-Api $primary 'POST' "/plans/$($crashPlan.id)/analyze"
    Assert-True ($crashJob.status -in @('queued', 'running') -and $crashJob.total -eq 12) 'The crash recovery task was not active.'
    Stop-Entry $primary
    $primary = Start-Entry $state $records
    Wait-Service $primary
    $recovered = Invoke-Api $primary 'GET' "/jobs/$($crashJob.id)"
    Assert-True ($recovered.status -eq 'interrupted') 'A task active during a killed process was not recovered as interrupted.'
    Assert-Preserved $primary $planId $generated.id 1 $hashes
    [void](Wait-Job $primary (Invoke-Api $primary 'POST' "/jobs/$($crashJob.id)/retry"))
    Stop-Entry $primary -Graceful
    $summary.checks += 'abnormal-process-exit-releases-lock-preserves-state-and-retries-interrupted-task'

    Write-Host 'Launcher scenario: denied state-directory access.'
    $denied = Join-Path $WorkDirectory '仅测试 无写权限'
    [void](New-Item -ItemType Directory -Path $denied)
    $savedAcl = Get-Acl -LiteralPath $denied
    $testAcl = Get-Acl -LiteralPath $denied
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $rule = [Security.AccessControl.FileSystemAccessRule]::new($identity, [Security.AccessControl.FileSystemRights]::Write, [Security.AccessControl.AccessControlType]::Deny)
    $testAcl.AddAccessRule($rule)
    Set-Acl -LiteralPath $denied -AclObject $testAcl
    try {
        $failed = Start-Entry $denied (Join-Path $WorkDirectory 'browser-denied')
        Wait-Failure $failed '权限|写入|无权'
    }
    finally { Set-Acl -LiteralPath $denied -AclObject $savedAcl; $savedAcl = $null }
    $summary.checks += 'state-directory-permission-error-chinese-actions-nonzero-and-cmd-pause'

    Write-Host 'Launcher scenario: occupied TCP port.'
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    $failed = Start-Entry (Join-Path $WorkDirectory '端口占用 状态') (Join-Path $WorkDirectory 'browser-port') 936 @('--port', [string]$port)
    Wait-Failure $failed '端口.*占用|端口.*监听'
    $listener.Stop(); $listener = $null
    $summary.checks += 'port-occupied-distinct-chinese-actions-nonzero-and-cmd-pause'

    Write-Host 'Launcher scenario: incompatible legacy lock holder.'
    $legacyState = Join-Path $WorkDirectory '旧版占锁 状态'
    $holder = New-ProbeProcess @('hold-lock', $legacyState)
    $holders.Add($holder)
    $ready = $holder.StandardOutput.ReadLineAsync()
    Assert-True ($ready.Wait(15000) -and $ready.GetAwaiter().GetResult() -eq 'PROBE_LOCK_READY') 'The synthetic legacy lock holder did not become ready.'
    $legacyRecords = Join-Path $WorkDirectory 'browser-legacy'
    $failed = Start-Entry $legacyState $legacyRecords
    Wait-Failure $failed '旧版|无法联系|已有.*服务|已有.*实例|原.*程序|占用'
    Assert-True (-not $holder.HasExited -and (Test-Path -LiteralPath (Join-Path $legacyState 'instance.lock'))) 'The launcher killed or removed the legacy lock holder.'
    $holder.Kill(); [void]$holder.WaitForExit(15000)
    $primary = Start-Entry $legacyState $legacyRecords
    Wait-Service $primary
    Stop-Entry $primary -Graceful
    $summary.checks += 'legacy-unreachable-lock-holder-left-intact-and-restart-after-owner-exits'

    Write-Host 'Launcher scenario: browser failure, preserved service and successful retry.'
    $state = Join-Path $WorkDirectory '浏览器失败 状态'
    $records = Join-Path $WorkDirectory 'browser-failure'
    [void](New-Item -ItemType Directory -Path $records)
    [IO.File]::WriteAllText((Join-Path $records 'fail'), 'synthetic')
    $primary = Start-Entry $state $records
    Wait-Service $primary
    $owner = Assert-OneOwner @(Wait-Browsers $records 1)
    Assert-True (-not $primary.Process.HasExited) 'A first browser-opening failure terminated the usable service.'
    $failed = Start-Entry $state $records
    Wait-Failure $failed '浏览器'
    [void](Invoke-Api $primary 'GET' '/status')
    [void](Assert-OneOwner @(Wait-Browsers $records 2) $owner)
    Remove-Item -LiteralPath (Join-Path $records 'fail')
    $again = Start-Entry $state $records
    Wait-Reused $again
    $browserResults = @(Wait-Browsers $records 3)
    [void](Assert-OneOwner $browserResults $owner)
    Assert-True (@($browserResults | Where-Object failed -EQ $false).Count -eq 1) 'Reopening did not recover after fixing the browser launcher.'
    Stop-Entry $primary -Graceful
    Assert-True ($primary.ErrorText -match '浏览器打开失败.*检查.*默认浏览器') 'The primary browser failure omitted its Chinese recovery instructions.'
    $summary.checks += 'first-browser-failure-keeps-service-secondary-failure-pauses-and-retry-recovers'
    $summary.passed = $true
}
finally {
    if ($null -ne $listener) { $listener.Stop() }
    if ($null -ne $savedAcl -and $null -ne $denied) { Set-Acl -LiteralPath $denied -AclObject $savedAcl }
    foreach ($ownedEntry in $owned) {
        if (-not $ownedEntry.Process.HasExited) { Stop-Entry $ownedEntry }
        $ownedEntry.Process.Dispose()
    }
    foreach ($holder in $holders) {
        if (-not $holder.HasExited) { $holder.Kill(); [void]$holder.WaitForExit(15000) }
        $holder.Dispose()
    }
    $summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $WorkDirectory 'launcher-summary.json') -Encoding utf8NoBOM
}
Write-Host 'Windows real CMD launcher smoke passed: ASCII/CRLF, Chinese and space paths, code pages 936/65001, first/repeat/concurrent/busy/startup activation, graceful/crash restart, protected failures and browser-owner recovery.'
Write-Output (Join-Path $WorkDirectory 'launcher-summary.json')
