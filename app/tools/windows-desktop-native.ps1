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
  [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr h);
  [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr h);
  [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, string l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint m,IntPtr w,string l,uint flags,uint timeout,out IntPtr result);
  [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint m,IntPtr w,StringBuilder l,uint flags,uint timeout,out IntPtr result);
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
  public static bool Button(IntPtr h,int id) { var b=GetDlgItem(h,id);if(b==IntPtr.Zero || Class(b)!="Button" || !IsWindowVisible(b) || !IsWindowEnabled(b))return false;PostMessageW(b,0xF5,IntPtr.Zero,IntPtr.Zero);return true; }
  static bool FilenameEdit(IntPtr edit,IntPtr dialog) {
    if(Class(edit)!="Edit" || !IsWindowVisible(edit) || !IsWindowEnabled(edit))return false;
    bool filenameHost=false;
    var parent=GetParent(edit);
    for(int depth=0;depth<8 && parent!=IntPtr.Zero;depth++,parent=GetParent(parent)) {
      if(parent==dialog)return filenameHost || (GetDlgCtrlID(edit)==1152 && GetParent(edit)==dialog);
      var kind=Class(parent);
      if(GetDlgCtrlID(parent)==1148 && (kind=="ComboBoxEx32" || kind=="ComboBox"))filenameHost=true;
    }
    return false;
  }
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
  public static bool Save(IntPtr h,string path) {
    var edits=new List<IntPtr>();
    // Search and filename controls can both use Edit ID1001. Only the filename
    // ComboBox host (cmb13=1148), or a direct classic edt1, is eligible.
    EnumChildWindows(h,(c,p)=> {if(FilenameEdit(c,h))edits.Add(c);return true;},IntPtr.Zero);
    if(edits.Count!=1)return false;
    var edit=edits[0];IntPtr result;
    if(SendMessageTimeoutW(edit,0xC,IntPtr.Zero,path,2,3000,out result)==IntPtr.Zero)return false;
    // GetWindowText cannot read another process's Edit text; WM_GETTEXT can.
    var actual=new StringBuilder(32768);
    if(SendMessageTimeoutW(edit,0xD,(IntPtr)actual.Capacity,actual,2,3000,out result)==IntPtr.Zero || actual.ToString()!=path)return false;
    return Button(h,1);
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
if ($ExpectedText -and -not $contents.Contains($ExpectedText,[StringComparison]::Ordinal)) { throw 'The owned dialog did not contain the expected Chinese explanation.' }
switch ($Action) {
    minimize { [OwnedDesktop]::Minimize($window) }
    close { [OwnedDesktop]::Close($window) }
    cancel { if (-not [OwnedDesktop]::Button($window,1001) -and -not [OwnedDesktop]::Button($window,2)) { [OwnedDesktop]::Close($window) } }
    accept { if (-not [OwnedDesktop]::Button($window,1000) -and -not [OwnedDesktop]::Button($window,1)) { throw 'The owned confirmation button was not found.' } }
    save {
        if (-not $SavePath -or -not [IO.Path]::IsPathFullyQualified($SavePath)) { throw 'Native save test requires an absolute owned fixture path.' }
        $saved = $false
        do {
            $saved = [OwnedDesktop]::Save($window,$SavePath)
            if (-not $saved) { Start-Sleep -Milliseconds 100 }
        } while (-not $saved -and [DateTimeOffset]::UtcNow -lt $deadline)
        if (-not $saved) { throw "The unique owned filename control could not be set and verified. Control structure: $([OwnedDesktop]::SaveControls($window))" }
    }
    error { if (-not [OwnedDesktop]::Button($window,1)) { throw 'The owned native error acknowledgement was not found.' } }
}
[ordered]@{controlled=$true; action=$Action; owned_pid=$OwnedProcessId; expected_text_checked=[bool]$ExpectedText} | ConvertTo-Json -Compress
