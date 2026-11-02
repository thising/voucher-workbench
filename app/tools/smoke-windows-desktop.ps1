#requires -Version 7.2
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ZipPath,
    [Parameter(Mandatory)][string]$InstallerPath,
    [Parameter(Mandatory)][string]$FixtureExecutable,
    [Parameter(Mandatory)][string]$ToolsDirectory,
    [string]$WorkDirectory,
    [string]$EvidenceDirectory
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Native desktop verification must run on Windows.' }
if (-not $WorkDirectory) { $WorkDirectory = Join-Path ([IO.Path]::GetTempPath()) ('pdf-desktop-' + [Guid]::NewGuid().ToString('N')) }
if (-not $EvidenceDirectory) { $EvidenceDirectory = Join-Path $WorkDirectory 'evidence' }
$WorkDirectory = [IO.Path]::GetFullPath($WorkDirectory)
$EvidenceDirectory = [IO.Path]::GetFullPath($EvidenceDirectory)
$reportFile = Join-Path $EvidenceDirectory 'desktop-smoke-report.json'
function Write-DesktopResult([bool]$Passed, [string]$Phase, [string]$Failure = '') {
    $null = New-Item -ItemType Directory -Force -Path $EvidenceDirectory
    $result = if (Test-Path -LiteralPath $reportFile) {
        Get-Content -LiteralPath $reportFile -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
    } else { @{ checks = @() } }
    $result['passed'] = $Passed
    $result['phase'] = $Phase
    if ($Failure) {
        $detail = $Failure -replace '#token=[^\s"<>]+', '#token=[redacted]'
        if ($result['failure']) { $result['outer_failure'] = $detail } else { $result['failure'] = $detail }
    }
    $result | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $reportFile -Encoding utf8NoBOM
}
if (Test-Path -LiteralPath $WorkDirectory) { throw 'The desktop test directory already exists; refusing to reuse data.' }
$null = New-Item -ItemType Directory -Path $WorkDirectory
$expanded = Join-Path $WorkDirectory '中文 包含空格 便携版'
Expand-Archive -LiteralPath $ZipPath -DestinationPath $expanded
$executables = @(Get-ChildItem -LiteralPath $expanded -File -Recurse -Filter 'PDF工作台.exe')
if ($executables.Count -ne 1) { throw 'Expected one shipped GUI executable.' }
$executable = $executables[0].FullName
& (Join-Path $PSScriptRoot 'verify-windows-package.ps1') -PackageDirectory $executables[0].DirectoryName
$installed = Join-Path $WorkDirectory '安装版 中文 空格'
$installInfo = [Diagnostics.ProcessStartInfo]::new()
$installInfo.FileName = (Resolve-Path -LiteralPath $InstallerPath).Path
$installInfo.UseShellExecute = $false
# NSIS requires /D= as the last switch and consumes the remaining path verbatim.
$installInfo.Arguments = "/S /D=$installed"
$installation = [Diagnostics.Process]::Start($installInfo)
try {
    if (-not $installation.WaitForExit(180000) -or $installation.ExitCode -ne 0) { throw 'Current-user NSIS installation failed.' }
    $installedExe = Join-Path $installed 'pdf-workbench.exe'
    if (-not (Test-Path -LiteralPath $installedExe)) { throw 'Installer did not place the desktop executable at the requested owned path.' }
    $installedHash = (Get-FileHash -LiteralPath $installedExe -Algorithm SHA256).Hash.ToLowerInvariant()
    $portableHash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($installedHash -ne $portableHash) { throw "Portable and installed GUI binaries differ: portable SHA256=$portableHash, installed SHA256=$installedHash." }
    foreach ($file in @('runtime/onnxruntime.dll','models/title-ocr/manifest.json','licenses/rust-desktop-dependencies.json','licenses/pdfjs/LICENSE','release-source.json','使用说明.txt')) {
        if (-not (Test-Path -LiteralPath (Join-Path $installed $file))) { throw "Installer resource missing: $file" }
    }
    & node (Join-Path $PSScriptRoot 'smoke-windows-desktop.mjs') $executable (Resolve-Path -LiteralPath $FixtureExecutable).Path (Resolve-Path -LiteralPath $ToolsDirectory).Path (Join-Path $WorkDirectory '合成运行场景') $EvidenceDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Real desktop/WebView2 verification failed; only the bounded report and synthetic screenshot may be published.' }
    # The JavaScript runner proves the portable copy only. Overall success is
    # withheld until installed-window validation and owned cleanup also pass.
    Write-DesktopResult $false 'installed-validation'
    # Also prove that an installed copy resolves the same model/DLL payload from
    # an unrelated CWD; no installer-run fixture or global user data is used.
    $check = [Diagnostics.ProcessStartInfo]::new()
    $check.FileName = $installedExe
    $check.ArgumentList.Add('--check-runtime')
    $check.ArgumentList.Add('--state-dir')
    $check.ArgumentList.Add((Join-Path $WorkDirectory '安装版只读检查状态'))
    $check.WorkingDirectory = $WorkDirectory
    $check.UseShellExecute = $false
    $check.RedirectStandardOutput = $true
    $check.RedirectStandardError = $true
    $process = [Diagnostics.Process]::Start($check)
    try {
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(120000) -or $process.ExitCode -ne 0) { throw 'Installed OCR payload failed its read-only diagnostic.' }
        $diagnostic = $stdout.GetAwaiter().GetResult() | ConvertFrom-Json
        $null = $stderr.GetAwaiter().GetResult()
        if (-not $diagnostic.model_ready) { throw 'Installed OCR payload is not ready.' }
    } finally { if (-not $process.HasExited) { $process.Kill() }; $process.Dispose() }
    $installedProbe = Join-Path $WorkDirectory '安装版窗口探针'
    $launch = [Diagnostics.ProcessStartInfo]::new()
    $launch.FileName = $installedExe
    $launch.UseShellExecute = $false
    foreach ($argument in @('--state-dir',(Join-Path $WorkDirectory '安装版窗口记录'),'--test-probe-dir',$installedProbe)) { $launch.ArgumentList.Add($argument) }
    $installedProcess = [Diagnostics.Process]::Start($launch)
    try {
        $deadline = [DateTimeOffset]::UtcNow.AddSeconds(90)
        $ready = $false
        do {
            if ($installedProcess.HasExited) { throw 'The installed GUI exited before creating its window.' }
            $probeFile = Join-Path $installedProbe 'desktop-probe.json'
            if (Test-Path -LiteralPath $probeFile) {
                try { $probe = Get-Content -LiteralPath $probeFile -Raw -Encoding utf8 | ConvertFrom-Json; $ready = $probe.webview_loaded -and $probe.window_visible } catch { $ready = $false }
            }
            if (-not $ready) { Start-Sleep -Milliseconds 100 }
        } while (-not $ready -and [DateTimeOffset]::UtcNow -lt $deadline)
        if (-not $ready) { throw 'The installed app did not load a real visible WebView2 window.' }
        $null = & (Join-Path $PSScriptRoot 'windows-desktop-native.ps1') -OwnedProcessId $installedProcess.Id -Action close
        if (-not $installedProcess.WaitForExit(60000) -or $installedProcess.ExitCode -ne 0) { throw 'The installed GUI did not close safely.' }
    } finally { if (-not $installedProcess.HasExited) { $installedProcess.Kill() }; $installedProcess.Dispose() }
    $report = Get-Content -LiteralPath $reportFile -Raw -Encoding utf8 | ConvertFrom-Json
    $report | Add-Member -NotePropertyName binary_sha256 -NotePropertyValue @{ portable = $portableHash; installed = $installedHash } -Force
    $report.checks += @('current-user-nsis-install-chinese-space-path','installed-and-portable-identical-exe','installed-model-runtime-and-license-payload','installed-real-visible-webview-and-normal-close')
    $report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $reportFile -Encoding utf8NoBOM
} catch {
    Write-DesktopResult $false 'failed' $_.Exception.Message
    throw
} finally {
  try {
    $installation.Dispose()
    $uninstaller = Join-Path $installed 'uninstall.exe'
    if (Test-Path -LiteralPath $uninstaller) {
        $removeInfo = [Diagnostics.ProcessStartInfo]::new()
        $removeInfo.FileName = $uninstaller
        $removeInfo.ArgumentList.Add('/S')
        $removeInfo.UseShellExecute = $false
        $remove = [Diagnostics.Process]::Start($removeInfo)
        if (-not $remove.WaitForExit(120000)) { throw 'Owned current-user test uninstall did not finish.' }
        if ($remove.ExitCode -ne 0) { throw 'Owned current-user test uninstall failed.' }
        $remove.Dispose()
    }
  } catch {
    Write-DesktopResult $false 'cleanup-failed' $_.Exception.Message
    throw
  }
}
Write-DesktopResult $true 'completed'
