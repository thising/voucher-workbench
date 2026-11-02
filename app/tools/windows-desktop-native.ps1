#requires -Version 7.2
<# Only controls windows owned by a caller-supplied test process. No global
   keyboard input, external browser control, or arbitrary process discovery. #>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][int]$OwnedProcessId,
    [Parameter(Mandatory)][ValidateSet('minimize','close','cancel','accept','save','error')][string]$Action,
    [string]$SavePath,
    [string]$ExpectedText
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class OwnedDesktop {
  public delegate bool Visitor(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] static extern bool EnumWindows(Visitor f, IntPtr p);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h, Visitor f, IntPtr p);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr h);
  [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr h, int id);
  [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
  static string Text(IntPtr h) { var s=new StringBuilder(8192); GetWindowTextW(h,s,s.Capacity); return s.ToString(); }
  static string Class(IntPtr h) { var s=new StringBuilder(256); GetClassNameW(h,s,s.Capacity); return s.ToString(); }
  public static IntPtr Find(int pid, bool dialog) {
    IntPtr result=IntPtr.Zero;
    EnumWindows((h,p)=> {uint owner;GetWindowThreadProcessId(h,out owner);
      if(owner==(uint)pid && IsWindowVisible(h) && (dialog ? Class(h)=="#32770" : Text(h)=="PDF工作台")) { result=h;return false; } return true;},IntPtr.Zero);
    return result;
  }
  public static string Contents(IntPtr h) { var lines=new List<string>{Text(h)}; EnumChildWindows(h,(c,p)=> {lines.Add(Text(c));return true;},IntPtr.Zero);return string.Join("\n",lines); }
  public static void Minimize(IntPtr h) { ShowWindow(h,6); }
  public static void Close(IntPtr h) { PostMessageW(h,0x10,IntPtr.Zero,IntPtr.Zero); }
  public static bool Button(IntPtr h,int id) { var b=GetDlgItem(h,id);if(b==IntPtr.Zero)return false;PostMessageW(b,0xF5,IntPtr.Zero,IntPtr.Zero);return true; }
  public static string SaveControls(IntPtr h) {
    var info=new List<string>();
    EnumChildWindows(h,(c,p)=> {
      if(Class(c)=="Edit" && IsWindowVisible(c) && info.Count<16) {
        var chain=new List<string>{"Edit#"+GetDlgCtrlID(c)};var parent=GetParent(c);
        for(int d=0;d<8 && parent!=IntPtr.Zero && parent!=h;d++,parent=GetParent(parent))chain.Add(Class(parent)+"#"+GetDlgCtrlID(parent));
        info.Add(string.Join("/",chain));
      }
      return true;
    },IntPtr.Zero);
    return string.Join("; ",info);
  }

}
'@
$process = Get-Process -Id $OwnedProcessId -ErrorAction Stop
$deadline = [DateTimeOffset]::UtcNow.AddSeconds(25)
$dialog = $Action -notin @('minimize','close')
$window = [IntPtr]::Zero
do {
    if ($process.HasExited) { throw 'The owned GUI process exited before its expected window appeared.' }
    $window = [OwnedDesktop]::Find($OwnedProcessId, $dialog)
    if ($window -ne [IntPtr]::Zero) { break }
    Start-Sleep -Milliseconds 100
} while ([DateTimeOffset]::UtcNow -lt $deadline)
if ($window -eq [IntPtr]::Zero) { throw 'The expected owned native window did not appear.' }
$contents = if ($dialog) { [OwnedDesktop]::Contents($window) } else { '' }
$textSource = 'win32'
$textControlsChecked = 0
if ($ExpectedText -and -not $contents.Contains($ExpectedText,[StringComparison]::Ordinal)) {
    # Modern TaskDialog text can be a virtual DirectUI element. Retain the exact
    # text assertion, using a read-only .NET Framework UIA client in this HWND.
    $textStart = [Diagnostics.ProcessStartInfo]::new()
    $textStart.FileName = Join-Path $env:WINDIR 'System32/WindowsPowerShell/v1.0/powershell.exe'
    $textStart.UseShellExecute = $false
    $textStart.CreateNoWindow = $true
    $textStart.RedirectStandardOutput = $true
    $textStart.RedirectStandardError = $true
    $textStart.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $textStart.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    foreach ($argument in @('-NoLogo','-NoProfile','-NonInteractive','-Mta','-File',(Join-Path $PSScriptRoot 'windows-dialog-text-uia.ps1'),'-OwnedProcessId',[string]$OwnedProcessId,'-OwnedWindowHandle',$window.ToInt64().ToString([Globalization.CultureInfo]::InvariantCulture),'-ExpectedText',$ExpectedText)) { $textStart.ArgumentList.Add($argument) }
    $textProcess = [Diagnostics.Process]::Start($textStart)
    try {
        $textOutput = $textProcess.StandardOutput.ReadToEndAsync()
        $textError = $textProcess.StandardError.ReadToEndAsync()
        if (-not $textProcess.WaitForExit(9500)) { $textProcess.Kill(); throw 'Owned dialog UIA text assertion timed out.' }
        $textJson = $textOutput.GetAwaiter().GetResult()
        $null = $textError.GetAwaiter().GetResult()
        try { $textResult = $textJson | ConvertFrom-Json } catch { throw 'Owned dialog UIA text assertion returned invalid JSON.' }
        if ($null -eq $textResult -or -not $textResult.PSObject.Properties['matched'] -or
            -not $textResult.PSObject.Properties['failure'] -or -not $textResult.PSObject.Properties['controls_checked']) {
            throw 'Owned dialog UIA text assertion returned an invalid result.'
        }
        if ($textProcess.ExitCode -ne 0 -or $textResult.matched -isnot [bool] -or $textResult.matched -ne $true) {
            $textFailure = if ([string]$textResult.failure -match '^[a-z0-9-]{1,80}$') { [string]$textResult.failure } else { 'assertion-failed' }
            throw "The owned dialog did not contain the expected Chinese explanation ($textFailure)."
        }
        $textSource = 'uia'
        $textControlsChecked = [Math]::Min(512,[Math]::Max(0,[int]$textResult.controls_checked))
    } finally { if (-not $textProcess.HasExited) { $textProcess.Kill() }; $textProcess.Dispose() }
}
switch ($Action) {
    minimize { [OwnedDesktop]::Minimize($window) }
    close { [OwnedDesktop]::Close($window) }
    cancel { if (-not [OwnedDesktop]::Button($window,1001) -and -not [OwnedDesktop]::Button($window,2)) { [OwnedDesktop]::Close($window) } }
    accept { if (-not [OwnedDesktop]::Button($window,1000) -and -not [OwnedDesktop]::Button($window,1)) { throw 'The owned confirmation button was not found.' } }
    save {
        if (-not $SavePath -or -not [IO.Path]::IsPathFullyQualified($SavePath)) { throw 'Native save test requires an absolute owned fixture path.' }
        # Modern IFileSaveDialog exposes virtual UIA filename controls whose
        # Win32 ComboBox ID can be zero. Use the built-in desktop CLR UIA client.
        $uiShell = Join-Path $env:WINDIR 'System32/WindowsPowerShell/v1.0/powershell.exe'
        $uiStart = [Diagnostics.ProcessStartInfo]::new()
        $uiStart.FileName = $uiShell
        $uiStart.UseShellExecute = $false
        $uiStart.CreateNoWindow = $true
        $uiStart.RedirectStandardOutput = $true
        $uiStart.RedirectStandardError = $true
        $uiStart.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
        $uiStart.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
        foreach ($argument in @('-NoLogo','-NoProfile','-NonInteractive','-Mta','-File',(Join-Path $PSScriptRoot 'windows-file-dialog-uia.ps1'),'-OwnedProcessId',[string]$OwnedProcessId,'-OwnedWindowHandle',$window.ToInt64().ToString([Globalization.CultureInfo]::InvariantCulture),'-SavePath',$SavePath)) { $uiStart.ArgumentList.Add($argument) }
        $uiProcess = [Diagnostics.Process]::Start($uiStart)
        try {
            $uiOutput = $uiProcess.StandardOutput.ReadToEndAsync()
            $uiError = $uiProcess.StandardError.ReadToEndAsync()
            if (-not $uiProcess.WaitForExit(28000)) { $uiProcess.Kill(); throw 'Owned UI Automation save controller timed out.' }
            $uiText = $uiOutput.GetAwaiter().GetResult()
            $uiFailure = $uiError.GetAwaiter().GetResult()
            if ($uiProcess.ExitCode -ne 0) { throw "Owned UI Automation save failed: $uiText $uiFailure. Win32 structure: $([OwnedDesktop]::SaveControls($window))" }
            $saveControlResult = $uiText | ConvertFrom-Json
            if (-not $saveControlResult.controlled -or -not $saveControlResult.save_target_verified) { throw 'Owned UI Automation did not verify the filename target.' }
        } finally { if (-not $uiProcess.HasExited) { $uiProcess.Kill() }; $uiProcess.Dispose() }
    }
    error { if (-not [OwnedDesktop]::Button($window,1)) { throw 'The owned native error acknowledgement was not found.' } }
}
$result = [ordered]@{controlled=$true; action=$Action; owned_pid=$OwnedProcessId; expected_text_checked=[bool]$ExpectedText}
if ($ExpectedText) { $result['expected_text_source'] = $textSource; $result['text_controls_checked'] = $textControlsChecked }
if ($Action -eq 'save') { $result['save_target_verified'] = $true; $result['filename_controls'] = $saveControlResult.filename_controls }
$result | ConvertTo-Json -Depth 10 -Compress
