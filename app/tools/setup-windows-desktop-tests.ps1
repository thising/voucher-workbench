#requires -Version 7.2
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Directory)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'The native desktop test tools require Windows.' }
$settings = Get-Content (Join-Path $PSScriptRoot '../distribution/windows-dependencies.json') -Raw -Encoding utf8 | ConvertFrom-Json
$Directory = [IO.Path]::GetFullPath($Directory)
$null = New-Item -ItemType Directory -Force -Path $Directory
$driver = Join-Path $Directory 'bin/tauri-driver.exe'
if (-not (Test-Path -LiteralPath $driver)) {
    & cargo "+$($settings.rust_toolchain)" install tauri-driver --version "=$($settings.desktop.tauri_driver)" --locked --root $Directory
    if ($LASTEXITCODE -ne 0) { throw 'The pinned tauri-driver build failed.' }
}
$guid = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
$versions = @()
foreach ($key in @("HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$guid", "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\$guid", "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$guid")) {
    if (Test-Path -LiteralPath $key) {
        $pv = Get-ItemPropertyValue -LiteralPath $key -Name pv -ErrorAction SilentlyContinue
        if ($pv -match '^\d+\.\d+\.\d+\.\d+$' -and [version]$pv -gt [version]'0.0.0.0') { $versions += [version]$pv }
    }
}
if ($versions.Count -eq 0) { throw 'A real WebView2 runtime must be installed on the CI host; the tests do not substitute an external browser.' }
$version = [string]($versions | Sort-Object -Descending | Select-Object -First 1)
$zip = Join-Path $Directory 'edgedriver_win64.zip'
try {
    Invoke-WebRequest -Uri "https://msedgedriver.microsoft.com/$version/edgedriver_win64.zip" -OutFile $zip
} catch {
    $major = $version.Split('.')[0]
    $response = Invoke-WebRequest -Uri "https://msedgedriver.microsoft.com/LATEST_RELEASE_${major}_WINDOWS"
    $content = $response.Content
    if ($content -is [byte[]]) {
        $content = if ($content.Length -ge 2 -and $content[0] -eq 255 -and $content[1] -eq 254) { [Text.Encoding]::Unicode.GetString($content) } else { [Text.Encoding]::UTF8.GetString($content) }
    }
    $candidate = ([string]$content).Replace([string][char]0,'').Trim([char]0xFEFF).Trim()
    if ($candidate -notmatch '^\d+\.\d+\.\d+\.\d+$' -or (($candidate.Split('.')[0..2] -join '.') -ne ($version.Split('.')[0..2] -join '.'))) { throw "No EdgeDriver matches the installed WebView2 build $version." }
    Invoke-WebRequest -Uri "https://msedgedriver.microsoft.com/$candidate/edgedriver_win64.zip" -OutFile $zip
}
Expand-Archive -LiteralPath $zip -DestinationPath (Join-Path $Directory 'edge') -Force
$edge = Join-Path $Directory 'edge/msedgedriver.exe'
$signature = Get-AuthenticodeSignature -LiteralPath $edge
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(^|,\s*)O=Microsoft Corporation(,|$)') { throw 'Microsoft EdgeDriver Authenticode verification failed.' }
$driverVersion = [regex]::Match((Get-Item -LiteralPath $edge).VersionInfo.ProductVersion,'\d+\.\d+\.\d+\.\d+').Value
if (($driverVersion.Split('.')[0..2] -join '.') -ne ($version.Split('.')[0..2] -join '.')) { throw 'EdgeDriver and WebView2 build numbers differ.' }
[ordered]@{schema_version=1; tauri_driver=$settings.desktop.tauri_driver; webview2_version=$version; edgedriver_version=$driverVersion; edgedriver_sha256=(Get-FileHash -LiteralPath $edge -Algorithm SHA256).Hash.ToLowerInvariant()} | ConvertTo-Json | Set-Content (Join-Path $Directory 'tools.json') -Encoding utf8NoBOM
Write-Host "Native test tools ready: tauri-driver $($settings.desktop.tauri_driver), WebView2 $version, EdgeDriver $driverVersion."
