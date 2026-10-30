#requires -Version 7.2
<#
Exercise the unpacked release binary using generated inputs only. The fixture
helper and this script are CI/development tools, not files shipped to customers.
No service URL fragment, session token, raw stdout, or raw stderr is published.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][Alias('PackageZip')][string]$ZipPath,
    [Parameter(Mandatory)][Alias('FixtureTool')][string]$FixtureExecutable,
    [Alias('WorkRoot')][string]$WorkDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Get-ComparablePath([string]$Path) {
    # Rust can return the Windows verbatim prefix after canonicalization while
    # PowerShell's filesystem provider returns the equivalent conventional path.
    if ($Path.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) {
        $Path = '\\' + $Path.Substring(8)
    }
    elseif ($Path.StartsWith('\\?\', [StringComparison]::Ordinal)) {
        $Path = $Path.Substring(4)
    }
    return [IO.Path]::TrimEndingDirectorySeparator([IO.Path]::GetFullPath($Path))
}

function New-ProcessInfo([string]$Executable, [string[]]$Arguments, [string]$WorkingDirectory, [string]$LocalData) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Executable
    $info.WorkingDirectory = $WorkingDirectory
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    # Prove that the package loads its own DLL, not a developer's environment.
    [void]$info.Environment.Remove('ORT_DYLIB_PATH')
    $info.Environment['LOCALAPPDATA'] = $LocalData
    return $info
}

function Stop-SmokeProcess($Process) {
    if ($null -eq $Process) { return }
    try { [void]$Process.Id }
    catch { $Process.Dispose(); return } # Start failed before a process existed.
    if (-not $Process.HasExited) {
        # Only this exact owned handle is stopped. Restart is tested after jobs
        # have completed; the application has no shutdown API or GUI automation.
        $Process.Kill()
        Assert-True ($Process.WaitForExit(15000)) 'Owned smoke process did not exit.'
    }
    $Process.Dispose()
}

function Invoke-RuntimeCheck([string]$Executable, [string]$WorkingDirectory, [string]$LocalData) {
    $info = New-ProcessInfo $Executable @('--check-runtime') $WorkingDirectory $LocalData
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    try {
        Assert-True ($process.Start()) 'Could not start runtime diagnostic.'
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        Assert-True ($process.WaitForExit(180000)) 'Runtime diagnostic exceeded 180 seconds.'
        Assert-True ($process.ExitCode -eq 0) 'Packaged OCR runtime failed to load. Inspect the Windows runtime dependencies locally.'
        $result = $stdout.GetAwaiter().GetResult() | ConvertFrom-Json -Depth 30
        [void]$stderr.GetAwaiter().GetResult()
        Assert-True ($result.model_ready -eq $true) 'Runtime diagnostic did not confirm model readiness.'
        Assert-True ($null -eq $result.error) 'Runtime diagnostic reported an error.'
        Assert-True (-not (Test-Path -LiteralPath $result.state_dir)) 'Read-only runtime diagnostic unexpectedly created state.'
        return $result
    }
    finally { Stop-SmokeProcess $process }
}

function Start-SmokeService([string]$Executable, [string]$WorkingDirectory, [string]$StateDirectory, [string]$LocalData) {
    $info = New-ProcessInfo $Executable @('--no-browser', '--state-dir', $StateDirectory) $WorkingDirectory $LocalData
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    try {
        Assert-True ($process.Start()) 'Could not start packaged service.'
        $stderr = $process.StandardError.ReadToEndAsync()
        $lineTask = $process.StandardOutput.ReadLineAsync()
        $deadline = [DateTime]::UtcNow.AddSeconds(180)
        while ([DateTime]::UtcNow -lt $deadline) {
            if ($lineTask.Wait(250)) {
                $line = $lineTask.GetAwaiter().GetResult()
                if ($null -eq $line) { throw 'Packaged service exited before becoming ready.' }
                if ($line -match 'http://127\.0\.0\.1:(\d+)/#token=([0-9a-f]+)') {
                    return [pscustomobject]@{
                        Process = $process
                        BaseUrl = "http://127.0.0.1:$($Matches[1])"
                        Token = $Matches[2]
                        Stdout = $process.StandardOutput.ReadToEndAsync()
                        Stderr = $stderr
                    }
                }
                $lineTask = $process.StandardOutput.ReadLineAsync()
            }
            if ($process.HasExited) { throw 'Packaged service exited during startup.' }
        }
        throw 'Packaged service did not become ready within 180 seconds.'
    }
    catch {
        Stop-SmokeProcess $process
        throw
    }
}

function Invoke-SmokeApi($Service, [string]$Method, [string]$Path, $Body = $null, [int]$ExpectedStatus = 200) {
    $parameters = @{
        Uri = "$($Service.BaseUrl)/api$Path"
        Method = $Method
        Headers = @{ 'X-Session-Token' = $Service.Token }
        TimeoutSec = 60
        SkipHttpErrorCheck = $true
        NoProxy = $true
    }
    if ($null -ne $Body) {
        $parameters.ContentType = 'application/json; charset=utf-8'
        $parameters.Body = [Text.Encoding]::UTF8.GetBytes(($Body | ConvertTo-Json -Depth 20 -Compress))
    }
    $response = Invoke-WebRequest @parameters
    Assert-True ([int]$response.StatusCode -eq $ExpectedStatus) "Unexpected HTTP status for $Method $Path; expected $ExpectedStatus, received $($response.StatusCode)."
    return $response.Content | ConvertFrom-Json -Depth 100
}

function Wait-SmokeJob($Service, $Job) {
    $deadline = [DateTime]::UtcNow.AddSeconds(300)
    while (@('queued', 'running') -contains $Job.status) {
        Assert-True ([DateTime]::UtcNow -lt $deadline) 'Synthetic smoke job exceeded 300 seconds.'
        Start-Sleep -Milliseconds 250
        $Job = Invoke-SmokeApi $Service 'GET' "/jobs/$($Job.id)"
    }
    Assert-True ($Job.status -eq 'completed') "Synthetic $($Job.kind) job did not complete successfully."
    Assert-True ($Job.done -eq $Job.total) 'Synthetic job completion count is inconsistent.'
    return $Job
}

function Invoke-Fixture([string]$Command, [string]$Root, [string]$Mode = '') {
    $arguments = @($Command, $Root)
    if ($Mode) { $arguments += $Mode }
    $output = & $script:FixtureExecutable @arguments
    Assert-True ($LASTEXITCODE -eq 0) "Synthetic fixture $Command failed."
    return ($output -join "`n") | ConvertFrom-Json -Depth 30
}

$ZipPath = (Resolve-Path -LiteralPath $ZipPath).Path
$FixtureExecutable = (Resolve-Path -LiteralPath $FixtureExecutable).Path
if ([string]::IsNullOrWhiteSpace($WorkDirectory)) {
    $WorkDirectory = Join-Path ([IO.Path]::GetTempPath()) ("voucher-smoke-" + [guid]::NewGuid().ToString('N'))
}
$WorkDirectory = [IO.Path]::GetFullPath($WorkDirectory)
Assert-True (-not (Test-Path -LiteralPath $WorkDirectory)) 'Smoke work directory already exists; refusing to overwrite.'
[void](New-Item -ItemType Directory -Path $WorkDirectory)
$service = $null
$report = [ordered]@{
    passed = $false
    synthetic_only = $true
    package_sha256 = (Get-FileHash -LiteralPath $ZipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    version = $null
    runtime_loaded = $false
    source_images = 2
    native_pdf_pages = 2
    total_output_pages = 4
    checks = @()
}

try {
    $expanded = Join-Path $WorkDirectory '中文 包含空格 解压目录'
    Expand-Archive -LiteralPath $ZipPath -DestinationPath $expanded
    $executables = @(Get-ChildItem -LiteralPath $expanded -Recurse -File -Filter 'voucher-workbench.exe')
    Assert-True ($executables.Count -eq 1) 'ZIP must contain exactly one workbench executable.'
    $executable = $executables[0].FullName
    $packageDirectory = $executables[0].DirectoryName
    $verifyScript = Join-Path $PSScriptRoot 'verify-windows-package.ps1'
    if (Test-Path -LiteralPath $verifyScript) {
        & $verifyScript -PackageDirectory $packageDirectory
        if (-not $?) { throw 'Extracted package verification failed.' }
    }
    $workingDirectory = Join-Path $WorkDirectory '其他 工作目录'
    [void](New-Item -ItemType Directory -Path $workingDirectory)
    $localData = Join-Path $WorkDirectory '隔离 LocalAppData'
    $runtime = Invoke-RuntimeCheck $executable $workingDirectory $localData
    $expectedModel = Join-Path $packageDirectory 'models/title-ocr'
    Assert-True ((Get-ComparablePath $runtime.model) -eq (Get-ComparablePath $expectedModel)) 'Runtime diagnostic used a model outside the package.'
    $report.runtime_loaded = $true
    $report.version = $runtime.version
    $report.checks += 'packaged-runtime-load-from-unrelated-cwd'

    $fixtureRoot = Join-Path $WorkDirectory '合成资料 测试批次'
    $stateDirectory = Join-Path $WorkDirectory '隔离 状态'
    $fixture = Invoke-Fixture 'create' $fixtureRoot
    Assert-True ($fixture.synthetic -eq $true -and $fixture.pages -eq 4) 'Fixture creation failed.'
    $service = Start-SmokeService $executable $workingDirectory $stateDirectory $localData
    $status = Invoke-SmokeApi $service 'GET' '/status'
    Assert-True ($status.model_ready -eq $true) 'Running package did not load bundled OCR components.'
    $plan = Invoke-SmokeApi $service 'POST' '/scan' @{ root = $fixtureRoot }
    $planId = $plan.id
    Assert-True ($plan.orientation_workflow.enabled -eq $false) 'New plan must default to direction recognition disabled.'
    Assert-True (@($plan.vouchers).Count -eq 1) 'Synthetic ledger did not match one voucher.'
    $files = @($plan.vouchers[0].files)
    Assert-True ($files.Count -eq 3 -and $plan.ignored -eq 2) 'Synthetic source sorting or ignored export resources are incorrect.'
    Assert-True ($plan.sheet -eq '合成首表' -and $plan.vouchers[0].subject -eq '合成科目') 'First-sheet or duplicate-first-row matching failed.'
    [void](Invoke-SmokeApi $service 'POST' "/plans/$planId/analyze" $null 400)
    $offJob = Wait-SmokeJob $service (Invoke-SmokeApi $service 'POST' "/plans/$planId/generate" @{})
    Assert-True ($offJob.total -eq 4 -and @($offJob.results).Count -eq 1) 'Disabled-mode generation returned wrong page/result counts.'
    [void](Invoke-Fixture 'verify' $fixtureRoot 'disabled')
    $report.checks += 'disabled-workflow-original-orientation-and-native-pdf'

    [void](Invoke-SmokeApi $service 'PATCH' "/plans/$planId/orientation" @{ enabled = $true; accept_disclaimer = $false } 400)
    $plan = Invoke-SmokeApi $service 'PATCH' "/plans/$planId/orientation" @{ enabled = $true; accept_disclaimer = $true }
    Assert-True ($plan.orientation_workflow.consent_version -eq $status.orientation_disclaimer_version) 'Consent version was not recorded.'
    Assert-True (-not [string]::IsNullOrWhiteSpace($plan.orientation_workflow.consented_at)) 'Consent time was not recorded.'
    Assert-True ($plan.orientation_workflow.analysis_completed -eq $false) 'Enabling recognition must require analysis completion.'
    $images = @($plan.vouchers[0].files | Where-Object kind -EQ 'image')
    [void](Invoke-SmokeApi $service 'POST' "/plans/$planId/review" @{ file_ids = @($images[0].id) } 400)
    [void](Invoke-SmokeApi $service 'POST' "/plans/$planId/generate" @{} 400)
    $analysis = Wait-SmokeJob $service (Invoke-SmokeApi $service 'POST' "/plans/$planId/analyze")
    Assert-True ($analysis.total -eq 2 -and $analysis.done -eq 2) 'Analysis did not process both synthetic images.'
    $plan = Invoke-SmokeApi $service 'GET' "/plans/$planId"
    Assert-True ($plan.orientation_workflow.analysis_completed -eq $true) 'Analysis completion stage was not persisted.'
    $images = @($plan.vouchers[0].files | Where-Object kind -EQ 'image')
    foreach ($file in $images) {
        Assert-True ($null -eq $file.error -and $null -ne $file.orientation) 'Actual model inference failed on a synthetic image.'
        Assert-True ($file.orientation.model -eq $status.orientation_model) 'Inference did not use the current title rule.'
        Assert-True ($file.reviewed -eq $false) 'Inference must not imply human confirmation.'
    }
    [void](Invoke-SmokeApi $service 'POST' "/plans/$planId/generate" @{} 400)
    [void](Invoke-SmokeApi $service 'POST' "/plans/$planId/generate" @{ allow_unreviewed = $true } 400)
    # Ground truth is defined by the synthetic fixture, not inferred from scores.
    # English bitmap titles may remain unresolved under the Chinese title rules.
    foreach ($file in $images) {
        $angle = if ($file.sequence -eq 2) { 270 } else { 0 }
        $plan = Invoke-SmokeApi $service 'PATCH' "/plans/$planId/files/$($file.id)" @{ rotation_cw = $angle; reviewed = $true }
    }
    $onJob = Wait-SmokeJob $service (Invoke-SmokeApi $service 'POST' "/plans/$planId/generate" @{})
    Assert-True ($onJob.total -eq 4) 'Confirmed-mode output has unexpected page count.'
    [void](Invoke-Fixture 'verify' $fixtureRoot 'enabled')
    $report.checks += 'consent-analysis-inference-manual-gates-and-corrected-output'

    Stop-SmokeProcess $service.Process
    $service = $null
    $service = Start-SmokeService $executable $workingDirectory $stateDirectory $localData
    $resumed = Invoke-SmokeApi $service 'GET' "/plans/$planId"
    Assert-True ($resumed.orientation_workflow.analysis_completed -eq $true) 'Completed analysis did not survive restart.'
    Assert-True (@($resumed.vouchers[0].files | Where-Object { $_.kind -eq 'image' -and $_.reviewed }).Count -eq 2) 'Manual confirmations did not survive restart.'
    $reuseJob = Wait-SmokeJob $service (Invoke-SmokeApi $service 'POST' "/plans/$planId/generate" @{})
    Assert-True (@($reuseJob.results).Count -eq 1 -and $reuseJob.results[0].status -eq 'skipped') 'Restart did not reuse the verified existing output.'
    [void](Invoke-Fixture 'verify' $fixtureRoot 'enabled')
    $report.checks += 'restart-preserves-state-and-reuses-verified-output'
    $report.passed = $true
}
finally {
    if ($null -ne $service) { Stop-SmokeProcess $service.Process; $service = $null }
    # Persist only this bounded result. Captured stdout/stderr and tokens remain
    # in memory and are never copied to CI artifacts or diagnostic files.
    $report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $WorkDirectory 'smoke-summary.json') -Encoding utf8
}

Write-Host 'Windows package smoke passed: synthetic 2 images + 2 native PDF pages; runtime, consent, confirmation, correction, restart and reuse verified.'
Write-Output (Join-Path $WorkDirectory 'smoke-summary.json')
