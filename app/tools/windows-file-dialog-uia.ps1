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
        control_type = $current.ControlType.ProgrammaticName
        enabled = [bool]$current.IsEnabled
        offscreen = [bool]$current.IsOffscreen
    }
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
        # The filename host is an ancestor of the Edit, never a search field
        # selected solely by its reused Edit ID1001.
        if ($depth -gt 0 -and ($current.AutomationId -ceq 'FileNameControlHost' -or
            ($current.AutomationId -ceq '1148' -and
             $current.ControlType -eq [System.Windows.Automation.ControlType]::ComboBox))) {
            $filenameHost = $true
        }
        $cursor = [System.Windows.Automation.TreeWalker]::RawViewWalker.GetParent($cursor)
    }
    return [pscustomobject]@{ inside = $inside; filename_host = $filenameHost; chain = @($chain.ToArray()) }
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
    $editCondition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Edit)
    $script:FailureCode = 'filename-enumeration'
    $edits = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $editCondition)
    if ($edits.Count -gt 64) { Stop-UiaSave 'too-many-edit-controls' }
    $candidates = [System.Collections.Generic.List[object]]::new()
    $structures = [System.Collections.Generic.List[object]]::new()
    foreach ($edit in $edits) {
        $ancestry = Get-ScopedAncestry $edit $root
        if ($structures.Count -lt 16) { [void]$structures.Add($ancestry) }
        $script:FilenameControls = @($structures.ToArray())
        $current = $edit.Current
        if (-not $ancestry.inside -or -not $ancestry.filename_host -or
            -not $current.IsEnabled -or $current.IsOffscreen) { continue }
        $pattern = $null
        if (-not $edit.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)) { continue }
        $valuePattern = [System.Windows.Automation.ValuePattern]$pattern
        if ($valuePattern.Current.IsReadOnly) { continue }
        [void]$candidates.Add([pscustomobject]@{ element = $edit; pattern = $valuePattern })
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
    if (-not (Get-ScopedAncestry $filename.element $root).inside) { Stop-UiaSave 'filename-left-dialog' }
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
