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
$taskDefinition = @'
using System;
using System.Runtime.InteropServices;
public static class OwnedTaskDialogProbe {
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    struct Activation { public int Size; public uint Flags; public string Source; public ushort Architecture; public ushort Language; public string Directory; public IntPtr Resource; public string Application; public IntPtr Module; }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateActCtxW(ref Activation context);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool ActivateActCtx(IntPtr context, out UIntPtr cookie);
    [DllImport("kernel32.dll")] static extern bool DeactivateActCtx(uint flags, UIntPtr cookie);
    [DllImport("kernel32.dll")] static extern void ReleaseActCtx(IntPtr context);
    [DllImport("comctl32.dll", CharSet=CharSet.Unicode)] static extern int TaskDialog(IntPtr owner, IntPtr instance, string title, string heading, string content, uint buttons, IntPtr icon, out int button);
    public static int Show(string manifest, string content) {
        Activation context = new Activation(); context.Size = Marshal.SizeOf(typeof(Activation)); context.Source = manifest;
        IntPtr handle = CreateActCtxW(ref context);
        if (handle == new IntPtr(-1)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        UIntPtr cookie;
        try {
            if (!ActivateActCtx(handle, out cookie)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            try { int button; int result = TaskDialog(IntPtr.Zero, IntPtr.Zero, "PDF native choice controller probe", null, content, 9, IntPtr.Zero, out button); if (result != 0) Marshal.ThrowExceptionForHR(result); return button; }
            finally { DeactivateActCtx(0, cookie); }
        } finally { ReleaseActCtx(handle); }
    }
}
'@
$choiceFixture = @'
param([Parameter(Mandatory)][string]$Root, [Parameter(Mandatory)][string]$Text, [Parameter(Mandatory)][int]$ExpectedButton)
$ErrorActionPreference = 'Stop'
Add-Type -Path ([IO.Path]::Combine($Root, 'TaskProbe.cs'))
$button = [OwnedTaskDialogProbe]::Show([IO.Path]::Combine($Root, 'task.manifest'), $Text)
if ($button -ne $ExpectedButton) { throw 'Owned task dialog chose a different button.' }
[IO.File]::WriteAllText([IO.Path]::Combine($Root, 'choice.txt'), [string]$button)
'@
$taskManifest = '<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0"><assemblyIdentity name="PdfWorkbench.OwnedDialogProbe" version="1.0.0.0" type="win32"/><dependency><dependentAssembly><assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/></dependentAssembly></dependency></assembly>'
[IO.File]::WriteAllText((Join-Path $work 'TaskProbe.cs'), $taskDefinition, [Text.ASCIIEncoding]::new())
[IO.File]::WriteAllText((Join-Path $work 'task.manifest'), $taskManifest, [Text.ASCIIEncoding]::new())
$choiceFixturePath = Join-Path $work 'owned-task-dialog.ps1'
[IO.File]::WriteAllText($choiceFixturePath, $choiceFixture, [Text.ASCIIEncoding]::new())
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
    $message = '任务或文件保存仍在进行'
    foreach ($choice in @(@{action='cancel';button=2},@{action='accept';button=1})) {
        $choiceStart = [Diagnostics.ProcessStartInfo]::new()
        $choiceStart.FileName = $launch.FileName
        $choiceStart.UseShellExecute = $false
        $choiceStart.CreateNoWindow = $true
        foreach ($argument in @('-NoLogo','-NoProfile','-NonInteractive','-Sta','-File',$choiceFixturePath,'-Root',$work,'-Text',$message,'-ExpectedButton',[string]$choice.button)) { $choiceStart.ArgumentList.Add($argument) }
        $choiceProcess = [Diagnostics.Process]::Start($choiceStart)
        try {
            $choiceControl = & (Join-Path $PSScriptRoot 'windows-desktop-native.ps1') -OwnedProcessId $choiceProcess.Id -Action $choice.action -ExpectedText $message | ConvertFrom-Json
            if (-not $choiceControl.controlled -or -not $choiceControl.expected_text_checked) { throw 'Owned task-dialog text and choice were not verified.' }
            if (-not $choiceProcess.WaitForExit(15000) -or $choiceProcess.ExitCode -ne 0) { throw 'Owned task-dialog fixture failed.' }
            if ([IO.File]::ReadAllText((Join-Path $work 'choice.txt')) -cne [string]$choice.button) { throw 'Owned task-dialog result mismatch.' }
        } finally {
            if (-not $choiceProcess.HasExited) { $choiceProcess.Kill(); $null = $choiceProcess.WaitForExit(10000) }
            $choiceProcess.Dispose()
        }
    }
    [ordered]@{ passed = $true; scope = 'owned-modern-file-and-task-dialogs'; exact_destination = $true; task_dialog_text_and_choices = $true } | ConvertTo-Json -Compress
} finally {
    if (-not $process.HasExited) { $process.Kill(); $null = $process.WaitForExit(10000) }
    $process.Dispose()
    Remove-Item -LiteralPath $work -Recurse -Force
}
