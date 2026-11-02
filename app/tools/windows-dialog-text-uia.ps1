#requires -Version 5.1
<# Defaults to a read-only text assertion in an exact owned native dialog.
   Optional ButtonText invokes one exact-name button after the text matches.
   Names are compared in memory only; stdout never contains Name or Value. #>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][int]$OwnedProcessId,
    [Parameter(Mandatory)][long]$OwnedWindowHandle,
    [Parameter(Mandatory)][string]$ExpectedText,
    [string]$ButtonText
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding
$script:FailureCode = 'initialization'
$script:CheckedNodes = 0
$script:Polls = 0
$script:NodeLimit = $false

function Stop-DialogText([string]$Code) {
    $script:FailureCode = $Code
    throw [InvalidOperationException]::new($Code)
}

function Write-TextResult([bool]$Matched, [string]$Failure, [bool]$Activated = $false) {
    $result = [ordered]@{
        matched = $Matched
        activated = $Activated
        controls_checked = $script:CheckedNodes
        polls = $script:Polls
        node_limit_reached = $script:NodeLimit
        failure = $Failure
    }
    [Console]::Out.WriteLine(($result | ConvertTo-Json -Compress))
}

function Get-VerifiedButtonPattern($Element, $Root) {
    $current = $Element.Current
    if ($current.ProcessId -ne $OwnedProcessId -or $current.IsOffscreen -or -not $current.IsEnabled -or
        ($current.ControlType -ne [System.Windows.Automation.ControlType]::Button -and
         $current.ControlType -ne [System.Windows.Automation.ControlType]::SplitButton) -or
        -not [string]::Equals($current.Name, $ButtonText, [StringComparison]::Ordinal)) {
        Stop-DialogText 'dialog-button-changed'
    }
    $inside = $false
    $cursor = $Element
    for ($depth = 0; $depth -lt 20 -and $null -ne $cursor; $depth++) {
        if ([System.Windows.Automation.Automation]::Compare($cursor, $Root)) { $inside = $true; break }
        if ($cursor.Current.ProcessId -ne $OwnedProcessId) { break }
        $cursor = [System.Windows.Automation.TreeWalker]::RawViewWalker.GetParent($cursor)
    }
    if (-not $inside -or -not $Root.Current.IsEnabled) { Stop-DialogText 'dialog-button-outside-root' }
    $pattern = $null
    if (-not $Element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
        Stop-DialogText 'dialog-button-not-invokable'
    }
    return [System.Windows.Automation.InvokePattern]$pattern
}

function Get-OwnedDialog {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]::new($OwnedWindowHandle))
    if ($null -eq $root) { Stop-DialogText 'dialog-root-unavailable' }
    $current = $root.Current
    if ($OwnedWindowHandle -lt [int]::MinValue -or $OwnedWindowHandle -gt [uint32]::MaxValue -or
        (([long]$current.NativeWindowHandle -band 4294967295L) -ne ($OwnedWindowHandle -band 4294967295L))) {
        Stop-DialogText 'dialog-handle-mismatch'
    }
    if ($current.ProcessId -ne $OwnedProcessId -or $current.ClassName -cne '#32770') {
        Stop-DialogText 'dialog-owner-or-class-mismatch'
    }
    if ($current.IsOffscreen) { Stop-DialogText 'dialog-not-visible' }
    return $root
}

try {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { Stop-DialogText 'windows-required' }
    if ($PSVersionTable.PSEdition -ne 'Desktop') { Stop-DialogText 'windows-powershell-framework-required' }
    if ($OwnedProcessId -le 0 -or $OwnedWindowHandle -eq 0 -or
        [string]::IsNullOrEmpty($ExpectedText) -or $ExpectedText.Length -gt 4096 -or
        ($ButtonText -and $ButtonText.Length -gt 4096)) {
        Stop-DialogText 'invalid-arguments'
    }
    $script:FailureCode = 'uia-assembly-load'
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $matched = $false
    $lastFailure = 'expected-text-not-found'
    do {
        $script:Polls++
        $script:FailureCode = 'dialog-root-read'
        try { $root = Get-OwnedDialog } catch {
            if ($script:FailureCode -notin @('dialog-root-read','dialog-root-unavailable')) { throw }
            $lastFailure = 'dialog-root-unavailable'
            if ($clock.ElapsedMilliseconds -lt 5000) { Start-Sleep -Milliseconds 200 }
            continue
        }
        $pending = [System.Collections.Generic.Queue[object]]::new()
        $pending.Enqueue($root)
        $buttons = [System.Collections.Generic.List[object]]::new()
        $matched = $false
        $scanComplete = $false
        try {
            while ($pending.Count -gt 0 -and $script:CheckedNodes -lt 512 -and $clock.ElapsedMilliseconds -lt 5000) {
                $element = $pending.Dequeue()
                $script:CheckedNodes++
                $current = $element.Current
                # Foreign-provider subtrees are never inspected or traversed.
                if ($current.ProcessId -ne $OwnedProcessId) { continue }
                if (-not $current.IsOffscreen) {
                    $name = $current.Name
                    if ($null -ne $name -and $name.IndexOf($ExpectedText, [StringComparison]::Ordinal) -ge 0) {
                        $matched = $true
                        if (-not $ButtonText) { break }
                    }
                    if ($ButtonText -and $current.IsEnabled -and
                        ($current.ControlType -eq [System.Windows.Automation.ControlType]::Button -or
                         $current.ControlType -eq [System.Windows.Automation.ControlType]::SplitButton) -and
                        [string]::Equals($name, $ButtonText, [StringComparison]::Ordinal)) {
                        $pattern = $null
                        if ($element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
                            [void]$buttons.Add($element)
                        }
                    }
                }
                $child = [System.Windows.Automation.TreeWalker]::RawViewWalker.GetFirstChild($element)
                while ($null -ne $child) {
                    if ($script:CheckedNodes + $pending.Count -ge 512) { $script:NodeLimit = $true; break }
                    $pending.Enqueue($child)
                    $child = [System.Windows.Automation.TreeWalker]::RawViewWalker.GetNextSibling($child)
                }
            }
            $scanComplete = $pending.Count -eq 0
        } catch {
            # UIA nodes can appear/disappear during native-dialog initialization.
            # Retry within the same root/deadline; never expose provider messages.
            $lastFailure = 'uia-text-read-failed'
        }
        if ($matched -and -not $ButtonText) { break }
        if ($ButtonText -and $buttons.Count -gt 1) { Stop-DialogText 'dialog-button-ambiguous' }
        if ($script:CheckedNodes -ge 512) { $script:NodeLimit = $true }
        if ($script:NodeLimit) { $lastFailure = 'dialog-node-limit'; break }
        if ($matched -and $ButtonText -and $scanComplete -and $buttons.Count -eq 1) {
            $script:FailureCode = 'dialog-button-revalidation'
            $root = Get-OwnedDialog
            $invoke = Get-VerifiedButtonPattern $buttons[0] $root
            $script:FailureCode = 'dialog-button-invoke'
            $invoke.Invoke()
            # Do not read a dialog or element after Invoke may have destroyed it.
            Write-TextResult $true '' $true
            exit 0
        }
        if ($matched -and $ButtonText) { $lastFailure = 'dialog-button-not-found' }
        if ($clock.ElapsedMilliseconds -lt 5000) { Start-Sleep -Milliseconds 200 }
    } while ($clock.ElapsedMilliseconds -lt 5000)
    if ($matched -and -not $ButtonText) { Write-TextResult $true ''; exit 0 }
    Write-TextResult $matched $lastFailure
    exit 1
} catch {
    Write-TextResult $false $script:FailureCode
    exit 1
}
