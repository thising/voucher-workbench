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

function Invoke-Fixture([string]$Command, [string]$Root, [string]$Mode = '', [string]$StateDirectory = '') {
    $arguments = @($Command, $Root)
    if ($Mode) { $arguments += $Mode }
    if ($StateDirectory) { $arguments += @('--state-dir', $StateDirectory) }
    $output = & $script:FixtureExecutable @arguments
    Assert-True ($LASTEXITCODE -eq 0) "Synthetic fixture $Command failed."
    return ($output -join "`n") | ConvertFrom-Json -Depth 30
}

function Copy-TaggedSyntheticPdf([string]$Source, [string]$Destination, [string]$Tag) {
    # Replace four ASCII bytes in the generated, uncompressed fixture text.
    # Equal lengths preserve stream lengths and xref offsets, and distinct
    # labels let the downloaded PDF's actual page tree prove file/page order.
    Assert-True ($Tag -cmatch '^[AB][0-9]{3}$') 'Invalid synthetic PDF label.'
    $bytes = [IO.File]::ReadAllBytes($Source)
    $text = [Text.Encoding]::Latin1.GetString($bytes)
    Assert-True ([regex]::Matches($text, 'SYNTHETIC PAGE [12]').Count -eq 2) 'The generated PDF fixture text changed.'
    $tagged = [Text.Encoding]::Latin1.GetBytes($text.Replace('SYNTHETIC PAGE', "SYNTHETIC $Tag"))
    Assert-True ($tagged.Length -eq $bytes.Length) 'Synthetic labels changed PDF byte offsets.'
    [IO.File]::WriteAllBytes($Destination, $tagged)
}

function Save-SmokePdf($Service, [string]$ApiPath, [string]$Destination, [string]$ExpectedHash) {
    $response = Invoke-WebRequest -Uri "$($Service.BaseUrl)/api$ApiPath" -Method GET -Headers @{ 'X-Session-Token' = $Service.Token } -TimeoutSec 60 -SkipHttpErrorCheck -NoProxy -OutFile $Destination -PassThru
    Assert-True ([int]$response.StatusCode -eq 200) 'The common PDF download endpoint rejected a synthetic output.'
    Assert-True ([string]$response.Headers['Content-Type'] -match '^application/pdf') 'The PDF download returned the wrong content type.'
    Assert-True ((Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash.ToLowerInvariant() -eq $ExpectedHash.ToLowerInvariant()) 'Downloaded PDF bytes did not match the saved output fingerprint.'
}

function Get-SyntheticPdfPageLabels([string]$Path) {
    # This deliberately small reader applies only to our known, plain-object
    # fixture PDFs. It follows /Kids and /Contents rather than file byte order.
    $text = [Text.Encoding]::Latin1.GetString([IO.File]::ReadAllBytes($Path))
    Assert-True ($text.StartsWith('%PDF-')) 'Downloaded output is not a PDF.'
    $objects = @{}
    foreach ($object in [regex]::Matches($text, '(?ms)(\d+)\s+0\s+obj\s*(.*?)\s*endobj')) {
        $objects[$object.Groups[1].Value] = $object.Groups[2].Value
    }
    $roots = @($objects.Values | Where-Object { $_ -match '/Type\s*/Pages\b' })
    Assert-True ($roots.Count -eq 1) 'The synthetic merged PDF did not have one page-tree root.'
    $kids = [regex]::Match($roots[0], '(?s)/Kids\s*\[(.*?)\]').Groups[1].Value
    $references = @([regex]::Matches($kids, '(\d+)\s+0\s+R'))
    Assert-True ($references.Count -eq 4) 'The downloaded generic output did not contain four actual PDF pages.'
    foreach ($reference in $references) {
        $page = $objects[$reference.Groups[1].Value]
        Assert-True ($page -match '/Type\s*/Page\b') 'A synthetic page-tree child was not a page.'
        $content = [regex]::Match($page, '/Contents\s+(\d+)\s+0\s+R')
        Assert-True ($content.Success) 'A synthetic PDF page lost its content reference.'
        $label = [regex]::Match($objects[$content.Groups[1].Value], 'SYNTHETIC ([AB][0-9]{3}) ([12])')
        Assert-True ($label.Success) 'The downloaded PDF lost a synthetic source-page label.'
        Write-Output "$($label.Groups[1].Value)-$($label.Groups[2].Value)"
    }
}

function Assert-GenericOutput($Service, $Job, $Plan, [string]$Downloads, $Tags) {
    $snapshot = Invoke-SmokeApi $Service 'GET' "/jobs/$($Job.id)/report"
    Assert-True ($snapshot.mode -eq 'generic_pdf' -and $snapshot.plan_id -eq $Plan.id -and $snapshot.successful_pages -eq 8) 'The common report lost the generic task snapshot or page total.'
    Assert-True ($snapshot.level -eq 1 -and $snapshot.sort -eq $Plan.sort -and @($snapshot.manifests).Count -eq 2) 'The generic report lost grouping or sort rules.'
    foreach ($group in $Plan.groups) {
        $manifest = @($snapshot.manifests | Where-Object group_key -EQ $group.relative_path)
        Assert-True ($manifest.Count -eq 1) 'A generic group is missing from the output report.'
        $manifest = $manifest[0]
        Assert-True ($manifest.mode -eq 'generic_pdf' -and $manifest.pages -eq 4 -and $manifest.output_name -eq $group.output_name -and @($manifest.sources).Count -eq 2) 'A generic output manifest lost its name or pages.'
        $expectedLabels = @()
        for ($index = 0; $index -lt 2; $index++) {
            $file = $group.files[$index]
            $mapping = $manifest.sources[$index]
            Assert-True ($mapping.source -eq $file.relative_path -and $mapping.sha256 -eq $file.sha256 -and $mapping.pages -eq 2 -and $mapping.first_page -eq 1 + $index * 2 -and $mapping.last_page -eq 2 + $index * 2) 'Generic source order or output page mapping is incorrect.'
            Assert-True ((Get-FileHash -LiteralPath (Join-Path $Plan.root $file.relative_path) -Algorithm SHA256).Hash.ToLowerInvariant() -eq $file.sha256) 'Generic merging modified a source PDF.'
            $tag = $Tags[$file.relative_path]
            Assert-True (-not [string]::IsNullOrWhiteSpace($tag)) 'Unknown synthetic generic source.'
            $expectedLabels += @("$tag-1", "$tag-2")
        }
        $download = Join-Path $Downloads "$($Job.id)-$($group.id).pdf"
        Save-SmokePdf $Service "/jobs/$($Job.id)/outputs/$($group.id)" $download $manifest.output_sha256
        $labels = @(Get-SyntheticPdfPageLabels $download)
        Assert-True (($labels -join ',') -ceq ($expectedLabels -join ',')) 'The downloaded PDF page tree does not follow the requested file order.'
    }
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
    source_images = 4
    native_pdf_pages = 4
    total_output_pages = 8
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
    Assert-True ($fixture.synthetic -eq $true -and $fixture.pages -eq 8 -and $fixture.vouchers -eq 2) 'Fixture creation failed.'
    $service = Start-SmokeService $executable $workingDirectory $stateDirectory $localData
    $status = Invoke-SmokeApi $service 'GET' '/status'
    Assert-True ($status.model_ready -eq $true) 'Running package did not load bundled OCR components.'
    $settings = Invoke-SmokeApi $service 'GET' '/settings'
    Assert-True ($settings.threads -eq 1) 'New state must default to one processing thread.'
    [void](Invoke-SmokeApi $service 'PATCH' '/settings' @{ threads = 0 } 400)
    [void](Invoke-SmokeApi $service 'PATCH' '/settings' @{ threads = 11 } 400)
    $settings = Invoke-SmokeApi $service 'PATCH' '/settings' @{ threads = 10 }
    Assert-True ($settings.threads -eq 10) 'Ten-thread preference was not saved.'

    $plan = Invoke-SmokeApi $service 'POST' '/scan' @{ root = $fixtureRoot }
    $planId = $plan.id
    Assert-True ($plan.orientation_workflow.enabled -eq $false) 'New plan must default to direction recognition disabled.'
    Assert-True (@($plan.vouchers).Count -eq 2) 'Synthetic ledger did not match two vouchers.'
    $files = @($plan.vouchers | ForEach-Object { $_.files })
    Assert-True ($files.Count -eq 6 -and $plan.ignored -eq 4) 'Synthetic source sorting or ignored export resources are incorrect.'
    Assert-True ($plan.sheet -eq '合成首表' -and $plan.vouchers[0].subject -eq '合成科目') 'First-sheet or duplicate-first-row matching failed.'
    [void](Invoke-SmokeApi $service 'POST' "/plans/$planId/analyze" $null 400)
    $offJob = Wait-SmokeJob $service (Invoke-SmokeApi $service 'POST' "/plans/$planId/generate" @{})
    Assert-True ($offJob.total -eq 8 -and @($offJob.results).Count -eq 2 -and $offJob.threads -eq 10) 'Disabled-mode generation returned wrong page/result counts.'
    [void](Invoke-Fixture 'verify' $fixtureRoot 'disabled' $stateDirectory)
    $report.checks += 'disabled-workflow-original-orientation-and-native-pdf'
    Assert-True ($offJob.results[0].voucher_id -eq $plan.vouchers[0].id -and $offJob.results[1].voucher_id -eq $plan.vouchers[1].id) 'Parallel results are not in voucher order.'


    [void](Invoke-SmokeApi $service 'PATCH' "/plans/$planId/orientation" @{ enabled = $true; accept_disclaimer = $false } 400)
    $plan = Invoke-SmokeApi $service 'PATCH' "/plans/$planId/orientation" @{ enabled = $true; accept_disclaimer = $true }
    Assert-True ($plan.orientation_workflow.consent_version -eq $status.orientation_disclaimer_version) 'Consent version was not recorded.'
    Assert-True (-not [string]::IsNullOrWhiteSpace($plan.orientation_workflow.consented_at)) 'Consent time was not recorded.'
    Assert-True ($plan.orientation_workflow.analysis_completed -eq $false) 'Enabling recognition must require analysis completion.'
    $images = @($plan.vouchers | ForEach-Object { $_.files } | Where-Object kind -EQ 'image')
    [void](Invoke-SmokeApi $service 'POST' "/plans/$planId/review" @{ file_ids = @($images[0].id) } 400)
    [void](Invoke-SmokeApi $service 'POST' "/plans/$planId/generate" @{} 400)
    $analysisStarted = Invoke-SmokeApi $service 'POST' "/plans/$planId/analyze"
    [void](Invoke-SmokeApi $service 'PATCH' '/settings' @{ threads = 1 } 400)
    $analysis = Wait-SmokeJob $service $analysisStarted
    Assert-True ($analysis.total -eq 4 -and $analysis.done -eq 4 -and $analysis.threads -eq 10) 'Analysis did not process all four images using the requested thread limit.'
    $plan = Invoke-SmokeApi $service 'GET' "/plans/$planId"
    Assert-True ($plan.orientation_workflow.analysis_completed -eq $true) 'Analysis completion stage was not persisted.'
    $images = @($plan.vouchers | ForEach-Object { $_.files } | Where-Object kind -EQ 'image')
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
    Assert-True ($onJob.total -eq 8 -and $onJob.threads -eq 10) 'Confirmed-mode output has unexpected page/thread count.'
    [void](Invoke-Fixture 'verify' $fixtureRoot 'enabled' $stateDirectory)
    $report.checks += 'consent-analysis-inference-manual-gates-and-corrected-output'
    [void](Invoke-SmokeApi $service 'PATCH' '/settings' @{ threads = 1 })

    Write-Host 'Package scenario: generic PDF grouping, exclusions, manual order and downloads.'
    $genericRoot = Join-Path $WorkDirectory '通用 合成资料'
    $genericOutput = Join-Path $genericRoot 'groupA/交付/最终 PDF'
    $genericDownloads = Join-Path $WorkDirectory '通用 下载核验'
    [void](New-Item -ItemType Directory -Path $genericOutput, (Join-Path $genericRoot 'groupB/indexImg'), (Join-Path $genericRoot '忽略整组'), $genericDownloads)
    $nativeFixture = Join-Path $fixtureRoot '合成分项/260101-00001/2601_00001_3.pdf'
    $genericTags = @{ 'groupA/2.pdf' = 'A002'; 'groupA/10.pdf' = 'A010'; 'groupB/3.pdf' = 'B003'; 'groupB/20.pdf' = 'B020' }
    foreach ($relative in $genericTags.Keys) {
        Copy-TaggedSyntheticPdf $nativeFixture (Join-Path $genericRoot $relative) $genericTags[$relative]
    }
    Copy-Item -LiteralPath $nativeFixture -Destination (Join-Path $genericRoot 'groupA/跳过.pdf')
    Copy-Item -LiteralPath $nativeFixture -Destination (Join-Path $genericRoot '忽略整组/排除.pdf')
    Copy-Item -LiteralPath $nativeFixture -Destination (Join-Path $genericOutput '历史排除.pdf')
    [IO.File]::WriteAllText((Join-Path $genericRoot 'groupB/indexImg/不能纳入.pdf'), 'SYNTHETIC INVALID PDF; MUST BE EXCLUDED BEFORE PARSING')
    $genericRequest = @{ root = $genericRoot; output_dir = $genericOutput; level = 1; exclude_paths = @('groupA/跳过.pdf', '忽略整组'); sort = 'natural_asc' }
    $genericPlan = Invoke-SmokeApi $service 'POST' '/generic/scan' $genericRequest
    $genericPlanId = $genericPlan.id
    Assert-True ($genericPlan.mode -eq 'generic_pdf' -and $genericPlan.level -eq 1 -and @($genericPlan.groups).Count -eq 2) 'Generic level-one scan did not produce the two expected groups.'
    Assert-True ((Get-ComparablePath $genericPlan.output_dir) -eq (Get-ComparablePath $genericOutput)) 'The custom nested generic output directory was not retained.'
    Assert-True (@($genericPlan.issues | Where-Object severity -EQ 'error').Count -eq 0) 'Synthetic generic scan unexpectedly reported an error.'
    Assert-True (($genericPlan.groups[0].files.name -join ',') -eq '2.pdf,10.pdf' -and ($genericPlan.groups[1].files.name -join ',') -eq '3.pdf,20.pdf') 'Generic natural sorting or file exclusions are incorrect.'
    Assert-True (@($genericPlan.exclude_globs) -contains '**/indexImg/**') 'The generic indexImg exclusion preset was not applied by default.'
    Assert-True (@($genericPlan.exclusions | Where-Object { $_.kind -eq 'path' -and @($_.matches).Count -gt 0 }).Count -eq 2) 'An explicit generic file/directory exclusion did not match.'
    Assert-True (@($genericPlan.exclusions | Where-Object { $_.kind -eq 'glob' -and $_.rule -eq '**/indexImg/**' -and @($_.matches).Count -gt 0 }).Count -eq 1) 'The indexImg preset did not exclude its invalid PDF.'
    Assert-True (@($genericPlan.exclusions | Where-Object { $_.kind -eq 'output' -and @($_.matches).Count -gt 0 }).Count -eq 1) 'The nested generic output directory was scanned as input.'
    $genericOne = Wait-SmokeJob $service (Invoke-SmokeApi $service 'POST' "/generic/plans/$genericPlanId/generate" @{})
    Assert-True ($genericOne.kind -eq 'generic_generate' -and $genericOne.total -eq 8 -and $genericOne.threads -eq 1 -and @($genericOne.results | Where-Object status -EQ 'completed').Count -eq 2) 'Single-thread generic generation did not produce both four-page outputs.'
    Assert-GenericOutput $service $genericOne $genericPlan $genericDownloads $genericTags
    $report.checks += 'generic-level-output-exclusion-preset-natural-order-single-thread-and-real-pdf-pages'

    $genericOrders = @($genericPlan.groups | ForEach-Object {
        $ids = @($_.files | Select-Object -ExpandProperty id)
        [array]::Reverse($ids)
        $name = if ($_.relative_path -eq 'groupA') { '甲组倒序.pdf' } else { '乙组倒序.pdf' }
        @{ group_id = $_.id; file_ids = $ids; output_name = $name }
    })
    $genericManual = Invoke-SmokeApi $service 'POST' "/generic/plans/$genericPlanId/order" @{ groups = $genericOrders }
    $genericManualId = $genericManual.id
    Assert-True ($genericManual.id -ne $genericPlanId -and $genericManual.parent_plan_id -eq $genericPlanId -and $genericManual.sort -eq 'manual') 'Generic ordering did not create an immutable derived plan.'
    Assert-True (($genericManual.groups[0].files.name -join ',') -eq '10.pdf,2.pdf' -and $genericManual.groups[0].output_name -eq '甲组倒序.pdf' -and ($genericManual.groups[1].files.name -join ',') -eq '20.pdf,3.pdf') 'The requested manual order or renamed output was lost.'
    $genericOriginal = Invoke-SmokeApi $service 'GET' "/generic/plans/$genericPlanId"
    Assert-True ($genericOriginal.sort -eq 'natural_asc' -and ($genericOriginal.groups[0].files.name -join ',') -eq '2.pdf,10.pdf' -and $genericOriginal.groups[0].output_name -eq 'groupA.pdf') 'Editing a generic plan changed the original historical plan.'
    [void](Invoke-SmokeApi $service 'PATCH' '/settings' @{ threads = 10 })
    $genericTen = Wait-SmokeJob $service (Invoke-SmokeApi $service 'POST' "/generic/plans/$genericManualId/generate" @{})
    Assert-True ($genericTen.total -eq 8 -and $genericTen.threads -eq 10 -and @($genericTen.results | Where-Object status -EQ 'completed').Count -eq 2) 'Ten-thread generic generation did not retain its requested thread limit.'
    Assert-GenericOutput $service $genericTen $genericManual $genericDownloads $genericTags
    Assert-GenericOutput $service $genericOne $genericPlan $genericDownloads $genericTags
    [void](Invoke-SmokeApi $service 'PATCH' '/settings' @{ threads = 1 })
    $report.checks += 'generic-immutable-manual-order-rename-ten-threads-report-and-output-downloads'


    Stop-SmokeProcess $service.Process
    $service = $null
    $service = Start-SmokeService $executable $workingDirectory $stateDirectory $localData
    $settings = Invoke-SmokeApi $service 'GET' '/settings'
    Assert-True ($settings.threads -eq 1) 'Single-thread preference did not survive restart.'
    $resumed = Invoke-SmokeApi $service 'GET' "/plans/$planId"
    Assert-True ($resumed.orientation_workflow.analysis_completed -eq $true) 'Completed analysis did not survive restart.'
    Assert-True (@($resumed.vouchers | ForEach-Object { $_.files } | Where-Object { $_.kind -eq 'image' -and $_.reviewed }).Count -eq 4) 'Manual confirmations did not survive restart.'
    $reuseJob = Wait-SmokeJob $service (Invoke-SmokeApi $service 'POST' "/plans/$planId/generate" @{})
    Assert-True ($reuseJob.threads -eq 1 -and @($reuseJob.results).Count -eq 2 -and @($reuseJob.results | Where-Object status -EQ 'skipped').Count -eq 2) 'Restart did not reuse the verified existing output.'
    [void](Invoke-Fixture 'verify' $fixtureRoot 'enabled' $stateDirectory)
    $report.checks += 'restart-preserves-state-and-reuses-verified-output'
    $retryJob = Wait-SmokeJob $service (Invoke-SmokeApi $service 'POST' "/jobs/$($onJob.id)/retry")
    Assert-True ($retryJob.threads -eq 10 -and @($retryJob.results | Where-Object status -EQ 'skipped').Count -eq 2) 'Retry did not retain the original ten-thread snapshot.'
    $settings = Invoke-SmokeApi $service 'GET' '/settings'
    Assert-True ($settings.threads -eq 1) 'Retry changed the global thread preference.'
    $report.checks += 'one-to-ten-thread-setting-and-retry-snapshot'

    $originalReport = Invoke-SmokeApi $service 'GET' "/jobs/$($offJob.id)/report"
    $latestReport = Invoke-SmokeApi $service 'GET' "/jobs/$($reuseJob.id)/report"
    Assert-True (@($originalReport.manifests).Count -eq 2 -and $originalReport.manifests[0].orientation_workflow.enabled -eq $false) 'Original report snapshot changed after regeneration or restart.'
    Assert-True (@($latestReport.manifests).Count -eq 2 -and $latestReport.manifests[0].orientation_workflow.enabled -eq $true) 'Latest report download failed after metadata separation.'
    Assert-True (@($latestReport.manifests[0].sources).Count -eq 3 -and $latestReport.manifests[0].pages -eq 4) 'Latest report lost source/page mappings.'
    $report.checks += 'metadata-outside-output-and-job-report-snapshots'

    $genericResumed = Invoke-SmokeApi $service 'GET' "/generic/plans/$genericManualId"
    Assert-True ($genericResumed.parent_plan_id -eq $genericPlanId -and $genericResumed.sort -eq 'manual' -and ($genericResumed.groups[0].files.name -join ',') -eq '10.pdf,2.pdf') 'The manual generic plan did not survive restart.'
    $genericHistory = @(Invoke-SmokeApi $service 'GET' '/generic/jobs')
    Assert-True (@($genericHistory | Where-Object { $_.id -eq $genericTen.id -and $_.status -eq 'completed' -and $_.threads -eq 10 }).Count -eq 1) 'Generic completed history or thread snapshot did not survive restart.'
    $genericReuse = Wait-SmokeJob $service (Invoke-SmokeApi $service 'POST' "/generic/plans/$genericManualId/generate" @{})
    Assert-True ($genericReuse.total -eq 8 -and $genericReuse.threads -eq 1 -and @($genericReuse.results | Where-Object status -EQ 'skipped').Count -eq 2) 'Restart did not reuse both verified generic outputs with the current single-thread preference.'
    Assert-GenericOutput $service $genericReuse $genericResumed $genericDownloads $genericTags
    Assert-GenericOutput $service $genericTen $genericManual $genericDownloads $genericTags
    Assert-GenericOutput $service $genericOne $genericPlan $genericDownloads $genericTags
    $genericRescan = Invoke-SmokeApi $service 'POST' '/generic/scan' $genericRequest
    Assert-True (@($genericRescan.groups).Count -eq 2 -and @($genericRescan.groups | ForEach-Object { $_.files }).Count -eq 4) 'Rescanning reingested the nested generic outputs.'
    Assert-True (@(Get-ChildItem -LiteralPath $genericOutput -File | Where-Object Extension -NE '.pdf').Count -eq 0) 'The generic output directory contains bookkeeping instead of only PDFs.'
    $report.checks += 'generic-restart-preserves-history-and-manual-plan-reuses-output-and-excludes-output-on-rescan'
    $report.passed = $true
}
finally {
    if ($null -ne $service) { Stop-SmokeProcess $service.Process; $service = $null }
    # Persist only this bounded result. Captured stdout/stderr and tokens remain
    # in memory and are never copied to CI artifacts or diagnostic files.
    $report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $WorkDirectory 'smoke-summary.json') -Encoding utf8
}

Write-Host 'Windows package smoke passed: synthetic voucher workflow and generic PDF merging; runtime, orientation review, manual file order, grouping/exclusions, real PDF page-tree/download checks, restart/history/reuse, separate reports and one-to-ten-thread settings verified.'
Write-Output (Join-Path $WorkDirectory 'smoke-summary.json')
