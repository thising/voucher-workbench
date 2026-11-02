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
    return [pscustomobject][ordered]@{
        automation_id = Get-SafeAutomationId $current.AutomationId
        class_name = Get-SafeAutomationId $current.ClassName
        control_type = $current.ControlType.ProgrammaticName
        enabled = [bool]$current.IsEnabled
        offscreen = [bool]$current.IsOffscreen
    }
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
            })
        }
        $script:FilenameControls = @($structures.ToArray())
        if (-not $ancestry.inside -or -not $ancestry.filename_host -or
            -not $current.IsEnabled -or $current.IsOffscreen -or -not $hasValue -or $readOnly) { continue }
        $hostElement = Get-FilenameHost $control $root
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
        $candidate = [pscustomobject]@{ element = $control; pattern = $valuePattern; host = $hostElement }
        if ($isHost) { [void]$hostGroup.preferred.Add($candidate) }
        else { [void]$hostGroup.fallback.Add($candidate) }
    }
    # Host and child Edit can both implement ValuePattern. Treat them as one
    # filename host and prefer its own pattern; distinct hosts remain ambiguous.
    if ($hosts.Count -gt 1) { Stop-UiaSave ('filename-host-count=' + $hosts.Count) }
    if ($hosts.Count -eq 1) {
        $hostGroup = $hosts[0]
        if ($hostGroup.preferred.Count -gt 0) {
            foreach ($candidate in $hostGroup.preferred) { [void]$candidates.Add($candidate) }
        } else {
            foreach ($candidate in $hostGroup.fallback) { [void]$candidates.Add($candidate) }
        }
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
            [void]$saveCandidates.Add([pscustomobject]@{ element = $button; pattern = [System.Windows.Automation.InvokePattern]$pattern })
        }
    }
    if ($saveCandidates.Count -ne 1) { Stop-UiaSave ('save-button-candidate-count=' + $saveCandidates.Count) }

    $filename = $candidates[0]
    $save = $saveCandidates[0]
    $script:FailureCode = 'dialog-revalidation'
    $null = Get-OwnedRoot
    $currentHost = Get-FilenameHost $filename.element $root
    if ($null -eq $currentHost -or -not [System.Windows.Automation.Automation]::Compare($currentHost, $filename.host) -or
        -not $filename.element.Current.IsEnabled -or $filename.element.Current.IsOffscreen -or
        $filename.pattern.Current.IsReadOnly) { Stop-UiaSave 'filename-left-dialog-or-not-writable' }
    $script:FailureCode = 'filename-focus'
    $filename.element.SetFocus()
    $script:FailureCode = 'filename-write'
    $filename.pattern.SetValue($SavePath)
    $script:FailureCode = 'filename-readback'
    if (-not [string]::Equals($filename.pattern.Current.Value, $SavePath, [StringComparison]::Ordinal)) {
        Stop-UiaSave 'filename-readback-mismatch'
    }
    # Focus commits the filename edit before invoking Save. Capture all evidence
    # first: invoking Save can destroy the dialog and invalidate every element.
    $script:FailureCode = 'save-button-focus'
    $save.element.SetFocus()
    $null = Get-OwnedRoot
    if (-not (Get-ScopedAncestry $save.element $root).inside -or
        -not $save.element.Current.IsEnabled -or $save.element.Current.IsOffscreen) {
        Stop-UiaSave 'save-button-left-dialog-or-disabled'
    }
    $success = [ordered]@{ controlled = $true; save_target_verified = $true; filename_controls = $script:FilenameControls }
    $script:FailureCode = 'save-button-invoke'
    $save.pattern.Invoke()
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
    }
    [Console]::Out.WriteLine(($failure | ConvertTo-Json -Depth 10 -Compress))
    exit 1
}
