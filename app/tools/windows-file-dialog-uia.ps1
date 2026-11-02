#requires -Version 5.1
<# Runs under built-in Windows PowerShell (.NET Framework). Controls only the
   caller's exact file-dialog HWND; diagnostics never include Name or Value. #>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][int]$OwnedProcessId,
    [Parameter(Mandatory)][long]$OwnedWindowHandle,
    [Parameter(Mandatory)][string]$SavePath
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding
$script:FailureCode = 'initialization'
$script:FilenameControls = @()
$script:ClientsideProvidersLoaded = $false

# Framework CodeDom-compatible C#: no global input or desktop enumeration.
# Text readback is limited to the exact, structurally verified filename child.
$nativeDefinition = @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class PdfWorkbenchDialogBridge {
    public delegate bool Visitor(IntPtr window, IntPtr data);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] static extern bool IsChild(IntPtr parent, IntPtr child);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] static extern bool IsWindowUnicode(IntPtr window);
    [DllImport("user32.dll", EntryPoint="GetWindowLongW")] static extern int GetWindowLongW(IntPtr window, int index);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr window);
    [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr window, StringBuilder text, int capacity);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, Visitor visitor, IntPtr data);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern IntPtr SendMessageTimeoutW(IntPtr window, uint message, IntPtr wparam, IntPtr lparam, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr window, uint message, IntPtr wparam, StringBuilder lparam, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    static string ClassOf(IntPtr window) { StringBuilder text = new StringBuilder(128); GetClassNameW(window, text, text.Capacity); return text.ToString(); }
    static bool SameProcess(IntPtr window, int process) { uint actual; GetWindowThreadProcessId(window, out actual); return IsWindow(window) && actual == (uint)process; }
    public static bool Root(long root, int process) { IntPtr window = new IntPtr(root); return SameProcess(window, process) && ClassOf(window) == "#32770" && IsWindowVisible(window) && IsWindowEnabled(window); }
    static bool Child(IntPtr window, IntPtr root, int process) { return SameProcess(window, process) && IsChild(root, window) && IsWindowVisible(window) && IsWindowEnabled(window); }
    public static long[] VisibleEdits(long root, int process) {
        List<long> result = new List<long>(); IntPtr parent = new IntPtr(root);
        if (!Root(root, process)) return result.ToArray();
        EnumChildWindows(parent, delegate(IntPtr child, IntPtr data) {
            if (Child(child, parent, process) && ClassOf(child) == "Edit") result.Add(child.ToInt64());
            return result.Count < 65;
        }, IntPtr.Zero);
        return result.ToArray();
    }
    public static double[] Bounds(long handle) {
        // UIA rectangles use physical screen pixels. GetWindowRect otherwise
        // virtualizes coordinates for a DPI-unaware PowerShell client.
        IntPtr previous = SetThreadDpiAwarenessContext(new IntPtr(-3));
        if (previous == IntPtr.Zero) return null;
        try { Rect rect; if (!GetWindowRect(new IntPtr(handle), out rect)) return null;
            return new double[] { rect.Left, rect.Top, rect.Right, rect.Bottom };
        } finally { SetThreadDpiAwarenessContext(previous); }
    }
    public static bool Filename(long handle, long root, int process, double left, double top, double right, double bottom) {
        IntPtr edit = new IntPtr(handle), dialog = new IntPtr(root);
        if (!Root(root, process) || !Child(edit, dialog, process) || ClassOf(edit) != "Edit" ||
            !IsWindowUnicode(edit) || (GetWindowLongW(edit, -16) & 0x800) != 0) return false;
        bool combo = false; bool reached = false; IntPtr parent = GetParent(edit);
        for (int depth = 0; depth < 20 && parent != IntPtr.Zero; depth++, parent = GetParent(parent)) {
            if (parent == dialog) { reached = true; break; }
            if (!SameProcess(parent, process)) return false;
            string kind = ClassOf(parent); if (kind == "ComboBox" || kind == "ComboBoxEx32") combo = true;
        }
        if (!reached || !combo) return false;
        double[] rect = Bounds(handle); if (rect == null || rect[2] <= rect[0] || rect[3] <= rect[1]) return false;
        // Two physical pixels tolerate fractional UIA border rounding only.
        return rect[0] >= left - 2 && rect[1] >= top - 2 && rect[2] <= right + 2 && rect[3] <= bottom + 2;
    }
    public static bool SelectFilename(long handle, long root, int process, double left, double top, double right, double bottom) {
        if (!Filename(handle, root, process, left, top, right, bottom)) return false;
        IntPtr result;
        if (SendMessageTimeoutW(new IntPtr(root), 0x28, new IntPtr(handle), new IntPtr(1), 2, 1000, out result) == IntPtr.Zero) return false;
        if (!Filename(handle, root, process, left, top, right, bottom)) return false;
        // EM_SETSEL selects this Edit's existing text, not a global key chord.
        return SendMessageTimeoutW(new IntPtr(handle), 0xB1, IntPtr.Zero, new IntPtr(-1), 2, 1000, out result) != IntPtr.Zero;
    }
    public static bool FilenameChar(long handle, long root, int process, double left, double top, double right, double bottom, int utf16) {
        if (utf16 < 0 || utf16 > 65535 || !Filename(handle, root, process, left, top, right, bottom)) return false;
        IntPtr result;
        // Directed Unicode WM_CHAR gives Edit/ComboBox their normal change
        // notifications. There is no SendInput, clipboard or global keyboard.
        return SendMessageTimeoutW(new IntPtr(handle), 0x102, new IntPtr(utf16), new IntPtr(1), 2, 1000, out result) != IntPtr.Zero;
    }
    public static bool VerifyFilename(long handle, long root, int process, double left, double top, double right, double bottom, string expected) {
        if (expected == null || !Filename(handle, root, process, left, top, right, bottom)) return false;
        StringBuilder actual = new StringBuilder(expected.Length + 2); IntPtr result;
        if (SendMessageTimeoutW(new IntPtr(handle), 0xD, new IntPtr(actual.Capacity), actual, 2, 1000, out result) == IntPtr.Zero) return false;
        return String.Equals(actual.ToString(), expected, StringComparison.Ordinal);
    }
    public static string[] Chain(long handle, long root, int process) {
        List<string> result = new List<string>(); IntPtr cursor = new IntPtr(handle), dialog = new IntPtr(root);
        for (int depth = 0; depth < 8 && cursor != IntPtr.Zero && cursor != dialog; depth++, cursor = GetParent(cursor)) {
            if (!SameProcess(cursor, process) || !IsChild(dialog, cursor)) break;
            result.Add(ClassOf(cursor) + "#" + GetDlgCtrlID(cursor));
        }
        return result.ToArray();
    }
    public static long SaveButton(long root, int process) {
        IntPtr dialog = new IntPtr(root), button = GetDlgItem(dialog, 1);
        return Root(root, process) && Child(button, dialog, process) && ClassOf(button) == "Button" && GetDlgCtrlID(button) == 1 ? button.ToInt64() : 0;
    }
    public static bool FocusSave(long root, int process, long button) {
        if (button == 0 || SaveButton(root, process) != button) return false;
        IntPtr result;
        // WM_NEXTDLGCTL selects this exact child in the dialog's own UI thread.
        return SendMessageTimeoutW(new IntPtr(root), 0x28, new IntPtr(button), new IntPtr(1), 2, 3000, out result) != IntPtr.Zero;
    }
    public static bool ClickSave(long root, int process, long button) {
        return button != 0 && SaveButton(root, process) == button && PostMessageW(new IntPtr(button), 0xF5, IntPtr.Zero, IntPtr.Zero);
    }
}
'@

function Stop-UiaSave([string]$Code) {
    $script:FailureCode = $Code
    throw [System.InvalidOperationException]::new($Code)
}

function Get-SafeAutomationId([string]$Id) {
    if ([string]::IsNullOrEmpty($Id)) { return '' }
    # IDs are structural evidence, not arbitrary provider strings or filenames.
    if ($Id -match '^(?:[A-Za-z_][A-Za-z0-9_.-]{0,63}|[0-9]{1,10})$') { return $Id }
    return '<non-identifier>'
}

function Get-ControlStructure($Element) {
    $current = $Element.Current
    $patterns = @()
    try { $patterns = @($Element.GetSupportedPatterns() | Select-Object -First 8 | ForEach-Object { $_.ProgrammaticName }) } catch { }
    return [pscustomobject][ordered]@{
        automation_id = Get-SafeAutomationId $current.AutomationId
        class_name = Get-SafeAutomationId $current.ClassName
        control_type = $current.ControlType.ProgrammaticName
        enabled = [bool]$current.IsEnabled
        offscreen = [bool]$current.IsOffscreen
        patterns = $patterns
    }
}

function Get-UiaBounds($Element) {
    $rect = $Element.Current.BoundingRectangle
    if ($rect.IsEmpty -or $rect.Width -le 0 -or $rect.Height -le 0) { return $null }
    $bounds = @([double]$rect.Left, [double]$rect.Top, [double]$rect.Right, [double]$rect.Bottom)
    foreach ($coordinate in $bounds) {
        if ([double]::IsNaN($coordinate) -or [double]::IsInfinity($coordinate)) { return $null }
    }
    return ,$bounds
}

function Get-RawChildStructures($Element) {
    $children = [System.Collections.Generic.List[object]]::new()
    $child = [System.Windows.Automation.TreeWalker]::RawViewWalker.GetFirstChild($Element)
    while ($null -ne $child -and $children.Count -lt 8) {
        if ($child.Current.ProcessId -ne $OwnedProcessId) { break }
        [void]$children.Add((Get-ControlStructure $child))
        $child = [System.Windows.Automation.TreeWalker]::RawViewWalker.GetNextSibling($child)
    }
    return ,@($children.ToArray())
}

function Test-FilenameHost($Current) {
    return ($Current.AutomationId -ceq 'FileNameControlHost' -or
        ($Current.ClassName -ceq 'FileNameControlHost' -and
         $Current.ControlType -eq [System.Windows.Automation.ControlType]::ComboBox) -or
        ($Current.AutomationId -ceq '1148' -and
         $Current.ControlType -eq [System.Windows.Automation.ControlType]::ComboBox))
}

function Get-OwnedRoot {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]::new($OwnedWindowHandle))
    if ($null -eq $root) { Stop-UiaSave 'dialog-root-unavailable' }
    $current = $root.Current
    # UIA exposes HWND as Int32. Compare its exact low bits, accepting only a
    # zero-extended or sign-extended Win32 handle supplied by the owned wrapper.
    if ($OwnedWindowHandle -lt [int]::MinValue -or $OwnedWindowHandle -gt [uint32]::MaxValue -or
        (([long]$current.NativeWindowHandle -band 4294967295L) -ne ($OwnedWindowHandle -band 4294967295L))) {
        Stop-UiaSave 'dialog-handle-mismatch'
    }
    if ($current.ProcessId -ne $OwnedProcessId -or $current.ClassName -cne '#32770') {
        Stop-UiaSave 'dialog-owner-or-class-mismatch'
    }
    if (-not $current.IsEnabled -or $current.IsOffscreen) { Stop-UiaSave 'dialog-not-interactive' }
    return $root
}

function Get-ScopedAncestry($Element, $Root) {
    $chain = [System.Collections.Generic.List[object]]::new()
    $cursor = $Element
    $filenameHost = $false
    $inside = $false
    for ($depth = 0; $depth -lt 20 -and $null -ne $cursor; $depth++) {
        if ([System.Windows.Automation.Automation]::Compare($cursor, $Root)) {
            $inside = $true
            break
        }
        $current = $cursor.Current
        if ($current.ProcessId -ne $OwnedProcessId) { break }
        if ($chain.Count -lt 8) { [void]$chain.Add((Get-ControlStructure $cursor)) }
        # A virtual filename host may expose ValuePattern itself without an
        # Edit child. SearchEditBox and arbitrary ComboBoxes never qualify.
        if (Test-FilenameHost $current) { $filenameHost = $true }
        $cursor = [System.Windows.Automation.TreeWalker]::RawViewWalker.GetParent($cursor)
    }
    return [pscustomobject]@{ inside = $inside; filename_host = $filenameHost; chain = @($chain.ToArray()) }
}

function Get-FilenameHost($Element, $Root) {
    $hostElement = $null
    $cursor = $Element
    for ($depth = 0; $depth -lt 20 -and $null -ne $cursor; $depth++) {
        if ([System.Windows.Automation.Automation]::Compare($cursor, $Root)) { return $hostElement }
        $current = $cursor.Current
        if ($current.ProcessId -ne $OwnedProcessId) { return $null }
        if ($null -eq $hostElement -and (Test-FilenameHost $current)) { $hostElement = $cursor }
        $cursor = [System.Windows.Automation.TreeWalker]::RawViewWalker.GetParent($cursor)
    }
    return $null
}

function Get-VerifiedNativeBounds($Candidate, $Root) {
    $null = Get-OwnedRoot
    $hostScope = Get-ScopedAncestry $Candidate.host $Root
    $bounds = Get-UiaBounds $Candidate.host
    if (-not $hostScope.inside -or -not (Test-FilenameHost $Candidate.host.Current) -or
        -not $Candidate.host.Current.IsEnabled -or $Candidate.host.Current.IsOffscreen -or $null -eq $bounds -or
        -not [PdfWorkbenchDialogBridge]::Filename($Candidate.native_handle, $OwnedWindowHandle, $OwnedProcessId,
            $bounds[0], $bounds[1], $bounds[2], $bounds[3])) { Stop-UiaSave 'filename-native-scope-changed' }
    return ,$bounds
}

try {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { Stop-UiaSave 'windows-required' }
    if ($PSVersionTable.PSEdition -ne 'Desktop') { Stop-UiaSave 'windows-powershell-framework-required' }
    if ($OwnedProcessId -le 0 -or $OwnedWindowHandle -eq 0) { Stop-UiaSave 'invalid-owned-window' }
    $script:FailureCode = 'invalid-save-path'
    if ($SavePath.Length -gt 32760 -or $SavePath -match '[\x00-\x1f"*?]' -or
        -not ($SavePath -match '^[A-Za-z]:[\\/]' -or $SavePath -match '^\\\\[^\\/]+\\[^\\/]+\\')) {
        Stop-UiaSave 'invalid-save-path'
    }
    $saveParent = [IO.Path]::GetDirectoryName($SavePath)
    if ([string]::IsNullOrEmpty([IO.Path]::GetFileName($SavePath)) -or -not [IO.Directory]::Exists($saveParent)) {
        Stop-UiaSave 'save-parent-or-filename-invalid'
    }
    $script:FailureCode = 'uia-assembly-load'
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    # Framework normally registers these proxies automatically. Explicit loading
    # is recorded as evidence, not treated as proof of the missing-tree cause.
    try { Add-Type -AssemblyName UIAutomationClientsideProviders; $script:ClientsideProvidersLoaded = $true } catch { }
    Add-Type -TypeDefinition $nativeDefinition -Language CSharp
    $script:FailureCode = 'dialog-root-read'
    $root = Get-OwnedRoot
    $filenameCondition = [System.Windows.Automation.OrCondition]::new(
        [System.Windows.Automation.OrCondition]::new(
            [System.Windows.Automation.PropertyCondition]::new(
                [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
                [System.Windows.Automation.ControlType]::Edit),
            [System.Windows.Automation.PropertyCondition]::new(
                [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
                [System.Windows.Automation.ControlType]::ComboBox)),
        [System.Windows.Automation.OrCondition]::new(
            [System.Windows.Automation.PropertyCondition]::new(
                [System.Windows.Automation.AutomationElement]::AutomationIdProperty, 'FileNameControlHost'),
            [System.Windows.Automation.PropertyCondition]::new(
                [System.Windows.Automation.AutomationElement]::ClassNameProperty, 'FileNameControlHost')))
    $script:FailureCode = 'filename-enumeration'
    $controls = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $filenameCondition)
    if ($controls.Count -gt 64) { Stop-UiaSave 'too-many-filename-controls' }
    $candidates = [System.Collections.Generic.List[object]]::new()
    $hosts = [System.Collections.Generic.List[object]]::new()
    $hostRegions = [System.Collections.Generic.List[object]]::new()
    $structures = [System.Collections.Generic.List[object]]::new()
    foreach ($control in $controls) {
        $ancestry = Get-ScopedAncestry $control $root
        $current = $control.Current
        $pattern = $null
        $hasValue = $control.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)
        $valuePattern = if ($hasValue) { [System.Windows.Automation.ValuePattern]$pattern } else { $null }
        $readOnly = if ($hasValue) { [bool]$valuePattern.Current.IsReadOnly } else { $null }
        if ($structures.Count -lt 16) {
            [void]$structures.Add([pscustomobject]@{
                inside = $ancestry.inside; filename_host = $ancestry.filename_host
                value_pattern = [bool]$hasValue; read_only = $readOnly; chain = $ancestry.chain
                raw_children = if (Test-FilenameHost $current) { Get-RawChildStructures $control } else { @() }
            })
        }
        $script:FilenameControls = @($structures.ToArray())
        $hostElement = $null
        if ($ancestry.inside -and $ancestry.filename_host) {
            $hostElement = Get-FilenameHost $control $root
            if ($null -ne $hostElement -and $hostElement.Current.IsEnabled -and -not $hostElement.Current.IsOffscreen) {
                $knownRegion = $false
                foreach ($region in $hostRegions) {
                    if ([System.Windows.Automation.Automation]::Compare($region, $hostElement)) { $knownRegion = $true; break }
                }
                if (-not $knownRegion) { [void]$hostRegions.Add($hostElement) }
            }
        }
        if (-not $ancestry.inside -or -not $ancestry.filename_host -or
            -not $current.IsEnabled -or $current.IsOffscreen -or -not $hasValue -or $readOnly) { continue }
        if ($null -eq $hostElement) { continue }
        $hostCurrent = $hostElement.Current
        if (-not $hostCurrent.IsEnabled -or $hostCurrent.IsOffscreen) { continue }
        $isHost = [System.Windows.Automation.Automation]::Compare($control, $hostElement)
        if (-not $isHost -and $current.ControlType -ne [System.Windows.Automation.ControlType]::Edit) { continue }
        $hostGroup = $null
        foreach ($entry in $hosts) {
            if ([System.Windows.Automation.Automation]::Compare($entry.element, $hostElement)) { $hostGroup = $entry; break }
        }
        if ($null -eq $hostGroup) {
            $hostGroup = [pscustomobject]@{
                element = $hostElement
                preferred = [System.Collections.Generic.List[object]]::new()
                fallback = [System.Collections.Generic.List[object]]::new()
            }
            [void]$hosts.Add($hostGroup)
        }
        $candidate = [pscustomobject]@{ element = $control; pattern = $valuePattern; host = $hostElement; native_handle = [long]0 }
        if ($isHost) { [void]$hostGroup.preferred.Add($candidate) }
        else { [void]$hostGroup.fallback.Add($candidate) }
    }
    # Host and child Edit can both implement ValuePattern. Treat them as one
    # filename host and prefer its own pattern; distinct hosts remain ambiguous.
    if ($hostRegions.Count -gt 1) { Stop-UiaSave ('filename-host-count=' + $hostRegions.Count) }
    if ($hosts.Count -eq 1) {
        $hostGroup = $hosts[0]
        if ($hostGroup.preferred.Count -gt 0) {
            foreach ($candidate in $hostGroup.preferred) { [void]$candidates.Add($candidate) }
        } else {
            foreach ($candidate in $hostGroup.fallback) { [void]$candidates.Add($candidate) }
        }
    }
    if ($candidates.Count -eq 0 -and $hostRegions.Count -eq 1) {
        $script:FailureCode = 'filename-native-bridge'
        $hostElement = $hostRegions[0]
        $hostBounds = Get-UiaBounds $hostElement
        if ($null -eq $hostBounds) { Stop-UiaSave 'filename-host-bounds-unavailable' }
        $bridgeInfo = [pscustomobject]@{
            kind = 'owned-win32-bridge'; host = Get-ControlStructure $hostElement
            host_bounds = $hostBounds; native_controls = @(); from_handle = $null
            input_mode = $null; readback_verified = $false
        }
        $script:FilenameControls = @($script:FilenameControls) + @($bridgeInfo)
        $nativeEdits = @([PdfWorkbenchDialogBridge]::VisibleEdits($OwnedWindowHandle, $OwnedProcessId))
        if ($nativeEdits.Count -gt 64) { Stop-UiaSave 'too-many-native-edit-controls' }
        $nativeMatches = [System.Collections.Generic.List[long]]::new()
        $nativeStructures = [System.Collections.Generic.List[object]]::new()
        foreach ($nativeHandle in $nativeEdits) {
            $eligible = [PdfWorkbenchDialogBridge]::Filename($nativeHandle, $OwnedWindowHandle, $OwnedProcessId,
                $hostBounds[0], $hostBounds[1], $hostBounds[2], $hostBounds[3])
            if ($nativeStructures.Count -lt 16) {
                [void]$nativeStructures.Add([pscustomobject]@{
                    native_chain = @([PdfWorkbenchDialogBridge]::Chain($nativeHandle, $OwnedWindowHandle, $OwnedProcessId))
                    physical_bounds = @([PdfWorkbenchDialogBridge]::Bounds($nativeHandle)); inside_filename_host = [bool]$eligible
                })
            }
            if ($eligible) { [void]$nativeMatches.Add($nativeHandle) }
        }
        $bridgeInfo.native_controls = @($nativeStructures.ToArray())
        if ($nativeMatches.Count -ne 1) { Stop-UiaSave ('filename-native-candidate-count=' + $nativeMatches.Count) }
        $nativeHandle = $nativeMatches[0]
        $script:FailureCode = 'filename-from-handle'
        $nativeElement = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]::new($nativeHandle))
        if ($null -eq $nativeElement) { Stop-UiaSave 'filename-from-handle-unavailable' }
        $current = $nativeElement.Current
        $pattern = $null
        $hasValue = $nativeElement.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)
        $valuePattern = if ($hasValue) { [System.Windows.Automation.ValuePattern]$pattern } else { $null }
        $bridgeInfo.from_handle = [pscustomobject]@{
            control = Get-ControlStructure $nativeElement; ancestry = Get-ScopedAncestry $nativeElement $root
            value_pattern = [bool]$hasValue; read_only = if ($hasValue) { [bool]$valuePattern.Current.IsReadOnly } else { $null }
        }
        # The managed UIA proxy can report Pane/no patterns for this verified
        # native Edit. Its exact HWND/PID identity remains mandatory.
        if ($current.ProcessId -ne $OwnedProcessId -or
            (([long]$current.NativeWindowHandle -band 4294967295L) -ne ($nativeHandle -band 4294967295L))) {
            Stop-UiaSave 'filename-from-handle-identity-mismatch'
        }
        if (-not $current.IsEnabled -or $current.IsOffscreen -or ($hasValue -and $valuePattern.Current.IsReadOnly)) {
            Stop-UiaSave 'filename-from-handle-not-writable'
        }
        [void]$candidates.Add([pscustomobject]@{
            element = $nativeElement; pattern = $valuePattern; host = $hostElement; native_handle = $nativeHandle
        })
    }
    if ($candidates.Count -ne 1) { Stop-UiaSave ('filename-candidate-count=' + $candidates.Count) }

    $saveTypeCondition = [System.Windows.Automation.OrCondition]::new(
        [System.Windows.Automation.PropertyCondition]::new(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::Button),
        [System.Windows.Automation.PropertyCondition]::new(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::SplitButton))
    $saveCondition = [System.Windows.Automation.AndCondition]::new(
        [System.Windows.Automation.PropertyCondition]::new(
            [System.Windows.Automation.AutomationElement]::AutomationIdProperty, '1'), $saveTypeCondition)
    $script:FailureCode = 'save-button-enumeration'
    $buttons = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $saveCondition)
    $saveCandidates = [System.Collections.Generic.List[object]]::new()
    if ($buttons.Count -gt 16) { Stop-UiaSave 'too-many-save-controls' }
    foreach ($button in $buttons) {
        $ancestry = Get-ScopedAncestry $button $root
        $current = $button.Current
        if (-not $ancestry.inside -or -not $current.IsEnabled -or $current.IsOffscreen) { continue }
        $pattern = $null
        if ($button.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
            [void]$saveCandidates.Add([pscustomobject]@{ element = $button; pattern = [System.Windows.Automation.InvokePattern]$pattern; native_handle = [long]0 })
        }
    }
    if ($saveCandidates.Count -eq 0) {
        $nativeButton = [PdfWorkbenchDialogBridge]::SaveButton($OwnedWindowHandle, $OwnedProcessId)
        if ($nativeButton -ne 0) {
            [void]$saveCandidates.Add([pscustomobject]@{ element = $null; pattern = $null; native_handle = $nativeButton })
        }
    }
    if ($saveCandidates.Count -ne 1) { Stop-UiaSave ('save-button-candidate-count=' + $saveCandidates.Count) }

    $filename = $candidates[0]
    $save = $saveCandidates[0]
    $script:FailureCode = 'dialog-revalidation'
    $null = Get-OwnedRoot
    if ($filename.native_handle -ne 0) {
        # A native proxy may have no managed UIA-host ancestor. Revalidate its
        # HWND ancestry and exact physical relationship to the known UIA host.
        $hostBounds = Get-VerifiedNativeBounds $filename $root
    } else {
        $currentHost = Get-FilenameHost $filename.element $root
        if ($null -eq $currentHost -or -not [System.Windows.Automation.Automation]::Compare($currentHost, $filename.host)) {
            Stop-UiaSave 'filename-left-dialog'
        }
    }
    if (-not $filename.element.Current.IsEnabled -or $filename.element.Current.IsOffscreen -or
        ($null -ne $filename.pattern -and $filename.pattern.Current.IsReadOnly)) { Stop-UiaSave 'filename-not-writable' }
    if ($null -eq $filename.pattern -and $filename.native_handle -ne 0) {
        $bridgeInfo.input_mode = 'owned-edit-wm-char'
        $script:FailureCode = 'filename-native-focus-select'
        $hostBounds = Get-VerifiedNativeBounds $filename $root
        if (-not [PdfWorkbenchDialogBridge]::SelectFilename($filename.native_handle, $OwnedWindowHandle, $OwnedProcessId,
            $hostBounds[0], $hostBounds[1], $hostBounds[2], $hostBounds[3])) { Stop-UiaSave 'filename-native-focus-select-failed' }
        $inputClock = [Diagnostics.Stopwatch]::StartNew()
        foreach ($unit in $SavePath.ToCharArray()) {
            $script:FailureCode = 'filename-native-character-input'
            if ($inputClock.ElapsedMilliseconds -gt 20000) { Stop-UiaSave 'filename-native-input-deadline' }
            # Refresh the exact host rectangle/ownership before every UTF-16
            # code unit; C# rechecks the HWND, parents, readonly style and bounds.
            $hostBounds = Get-VerifiedNativeBounds $filename $root
            if (-not [PdfWorkbenchDialogBridge]::FilenameChar($filename.native_handle, $OwnedWindowHandle, $OwnedProcessId,
                $hostBounds[0], $hostBounds[1], $hostBounds[2], $hostBounds[3], [int]$unit)) {
                Stop-UiaSave 'filename-native-character-input-failed'
            }
        }
        $script:FailureCode = 'filename-native-readback'
        $hostBounds = Get-VerifiedNativeBounds $filename $root
        if (-not [PdfWorkbenchDialogBridge]::VerifyFilename($filename.native_handle, $OwnedWindowHandle, $OwnedProcessId,
            $hostBounds[0], $hostBounds[1], $hostBounds[2], $hostBounds[3], $SavePath)) { Stop-UiaSave 'filename-native-readback-mismatch' }
        $bridgeInfo.readback_verified = $true
    } else {
        if ($filename.native_handle -ne 0) { $bridgeInfo.input_mode = 'uia-value-pattern' }
        $script:FailureCode = 'filename-focus'
        $filename.element.SetFocus()
        $script:FailureCode = 'filename-write'
        $filename.pattern.SetValue($SavePath)
        $script:FailureCode = 'filename-readback'
        if (-not [string]::Equals($filename.pattern.Current.Value, $SavePath, [StringComparison]::Ordinal)) {
            Stop-UiaSave 'filename-readback-mismatch'
        }
        if ($filename.native_handle -ne 0) { $bridgeInfo.readback_verified = $true }
    }
    # Focus commits the filename edit before invoking Save. Capture all evidence
    # first: invoking Save can destroy the dialog and invalidate every element.
    $script:FailureCode = 'save-button-focus'
    if ($save.native_handle -ne 0) {
        if (-not [PdfWorkbenchDialogBridge]::FocusSave($OwnedWindowHandle, $OwnedProcessId, $save.native_handle)) {
            Stop-UiaSave 'save-button-native-focus-failed'
        }
    } else {
        $save.element.SetFocus()
        $null = Get-OwnedRoot
        if (-not (Get-ScopedAncestry $save.element $root).inside -or
            -not $save.element.Current.IsEnabled -or $save.element.Current.IsOffscreen) {
            Stop-UiaSave 'save-button-left-dialog-or-disabled'
        }
    }
    $success = [ordered]@{ controlled = $true; save_target_verified = $true; filename_controls = $script:FilenameControls; clientside_providers_loaded = $script:ClientsideProvidersLoaded }
    $script:FailureCode = 'save-button-invoke'
    if ($save.native_handle -ne 0) {
        if (-not [PdfWorkbenchDialogBridge]::ClickSave($OwnedWindowHandle, $OwnedProcessId, $save.native_handle)) {
            Stop-UiaSave 'save-button-native-click-failed'
        }
    } else { $save.pattern.Invoke() }
    # Never query an element or a dialog property after Invoke.
    [Console]::Out.WriteLine(($success | ConvertTo-Json -Depth 10 -Compress))
    exit 0
} catch {
    # Provider exceptions can contain a filename, a Value or localized Name.
    # Emit only the stable operation code, exception type/HRESULT and structure.
    $caughtException = $_.Exception
    $failure = [ordered]@{
        controlled = $false
        save_target_verified = $false
        failure = $script:FailureCode
        exception_type = $caughtException.GetType().FullName
        hresult = ('0x{0:x8}' -f ([long]$caughtException.HResult -band 4294967295L))
        filename_controls = $script:FilenameControls
        clientside_providers_loaded = $script:ClientsideProvidersLoaded
    }
    [Console]::Out.WriteLine(($failure | ConvertTo-Json -Depth 10 -Compress))
    exit 1
}
