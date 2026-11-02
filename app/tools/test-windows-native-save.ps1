#requires -Version 7.2
<# Fast controller preflight. The full shipped Tauri save remains mandatory. #>
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Native save controller preflight requires Windows.' }
$work = Join-Path ([IO.Path]::GetTempPath()) ('pdf-save-probe-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $work
$fixture = @'
param([Parameter(Mandatory)][string]$Root)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
$dialog = [Windows.Forms.SaveFileDialog]::new()
try {
    $dialog.AutoUpgradeEnabled = $true
    $dialog.Title = 'PDF native save controller probe'
    $dialog.InitialDirectory = $Root
    $dialog.FileName = 'default.pdf'
    $dialog.Filter = 'PDF files (*.pdf)|*.pdf'
    if ($dialog.ShowDialog() -ne [Windows.Forms.DialogResult]::OK) { throw 'Owned save probe was cancelled.' }
    $expected = [IO.Path]::Combine($Root, 'requested.pdf')
    if (-not [string]::Equals([IO.Path]::GetFullPath($dialog.FileName), $expected, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Owned save probe returned a different destination.'
    }
    [IO.File]::WriteAllText($expected, 'synthetic-native-save-probe', [Text.UTF8Encoding]::new($false))
} finally { $dialog.Dispose() }
'@
$fixturePath = Join-Path $work 'owned-modern-dialog.ps1'
$expected = Join-Path $work 'requested.pdf'
[IO.File]::WriteAllText($fixturePath, $fixture, [Text.ASCIIEncoding]::new())
$launch = [Diagnostics.ProcessStartInfo]::new()
$launch.FileName = Join-Path $env:WINDIR 'System32/WindowsPowerShell/v1.0/powershell.exe'
$launch.UseShellExecute = $false
$launch.CreateNoWindow = $true
foreach ($argument in @('-NoLogo','-NoProfile','-NonInteractive','-Sta','-File',$fixturePath,'-Root',$work)) { $launch.ArgumentList.Add($argument) }
$process = [Diagnostics.Process]::Start($launch)
try {
    $control = & (Join-Path $PSScriptRoot 'windows-desktop-native.ps1') -OwnedProcessId $process.Id -Action save -SavePath $expected -ExpectedText 'PDF native save controller probe' | ConvertFrom-Json
    if (-not $control.controlled -or -not $control.save_target_verified) { throw 'Owned save controller did not verify its target.' }
    if (-not $process.WaitForExit(15000) -or $process.ExitCode -ne 0) { throw 'Owned save fixture did not exit successfully.' }
    if ([IO.File]::ReadAllText($expected) -cne 'synthetic-native-save-probe') { throw 'Owned save destination was not written correctly.' }
    [ordered]@{ passed = $true; scope = 'owned-modern-file-dialog'; exact_destination = $true } | ConvertTo-Json -Compress
} finally {
    if (-not $process.HasExited) { $process.Kill(); $null = $process.WaitForExit(10000) }
    $process.Dispose()
    Remove-Item -LiteralPath $work -Recurse -Force
}
