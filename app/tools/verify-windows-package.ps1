#requires -Version 7.2
[CmdletBinding()]
param([Parameter(Mandatory)][string]$PackageDirectory)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$package = (Resolve-Path -LiteralPath $PackageDirectory).Path
$manifestPath = Join-Path $package 'package-manifest.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding utf8 | ConvertFrom-Json
if ($manifest.schema_version -notin @(1,2) -or $manifest.target -ne 'x86_64-pc-windows-msvc') {
    throw 'Unsupported Windows package manifest.'
}
$required = @('voucher-workbench.exe', '启动凭证工作台.cmd', '使用说明.txt', 'LICENSE',
    'THIRD_PARTY_NOTICES.md', 'models/title-ocr/det.onnx', 'models/title-ocr/rec.onnx',
    'models/title-ocr/dictionary.txt', 'models/title-ocr/manifest.json', 'models/title-ocr/LICENSE',
    'models/title-ocr/det.yml', 'models/title-ocr/rec.yml', 'models/README.md',
    'runtime/onnxruntime.dll', 'licenses/rust-dependencies.json', 'licenses/onnxruntime/LICENSE',
    'licenses/onnxruntime/ThirdPartyNotices.txt')
$desktop = $manifest.schema_version -eq 2
if ($desktop) {
    if ($manifest.name -ne 'pdf-workbench' -or $manifest.ui -ne 'tauri' -or $manifest.executable -ne 'PDF工作台.exe') { throw 'Unexpected desktop executable contract.' }
    $required = @($required | Where-Object { $_ -notin @('voucher-workbench.exe','启动凭证工作台.cmd','licenses/rust-dependencies.json') })
    $required += @('PDF工作台.exe','release-source.json','licenses/rust-desktop-dependencies.json','licenses/pdfjs/LICENSE','licenses/pdfjs/manifest.json')
}
$items = @(Get-ChildItem -LiteralPath $package -Recurse -Force)
foreach ($item in $items) {
    if ($item.LinkType -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Linked package entry: $($item.Name)" }
}
$files = @(Get-ChildItem -LiteralPath $package -File -Recurse -Force)
$actualPaths = @($files | ForEach-Object { [IO.Path]::GetRelativePath($package, $_.FullName).Replace('\', '/') })
$declared = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($entry in $manifest.files) {
    $relative = [string]$entry.path
    if ($relative -match '(^/|\\|(^|/)\.\.?(/|$)|:)' -or -not $declared.Add($relative)) {
        throw "Unsafe or duplicate manifest entry: $relative"
    }
    if ($relative -notmatch '^(voucher-workbench\.exe|启动凭证工作台\.cmd|PDF工作台\.exe|release-source\.json|使用说明\.txt|LICENSE|THIRD_PARTY_NOTICES\.md|models/README\.md|models/title-ocr/(det\.onnx|rec\.onnx|dictionary\.txt|manifest\.json|LICENSE|det\.yml|rec\.yml)|runtime/(onnxruntime\.dll|onnxruntime_providers_shared\.dll)|licenses/[\w./-]+)$') {
        throw "Unexpected package file: $relative"
    }
    if ($desktop -and $relative -in @('voucher-workbench.exe','启动凭证工作台.cmd')) { throw 'Desktop package contains a legacy daily entry point.' }
    $path = Join-Path $package $relative
    $item = Get-Item -LiteralPath $path
    if ($item.LinkType -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Linked package file: $relative" }
    $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -ne $entry.sha256 -or $item.Length -ne $entry.bytes) { throw "Hash or size mismatch: $relative" }
}
foreach ($relative in $required) { if (-not $declared.Contains($relative)) { throw "Missing required package file: $relative" } }
foreach ($relative in $actualPaths) {
    if ($relative -ne 'package-manifest.json' -and -not $declared.Contains($relative)) { throw "Unlisted package file: $relative" }
}
if ($actualPaths.Count -ne $declared.Count + 1) { throw 'Package file count mismatch.' }
$modelManifest = Get-Content -LiteralPath (Join-Path $package 'models/title-ocr/manifest.json') -Raw -Encoding utf8 | ConvertFrom-Json
foreach ($model in @($modelManifest.detector, $modelManifest.recognizer, $modelManifest.dictionary)) {
    $path = Join-Path $package ('models/title-ocr/' + $model.filename)
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $model.sha256) {
        throw "Model digest mismatch: $($model.filename)"
    }
}
$executable = Join-Path $package $(if ($desktop) { 'PDF工作台.exe' } else { 'voucher-workbench.exe' })
$stream = [IO.File]::OpenRead($executable)
try {
    $reader = [IO.BinaryReader]::new($stream)
    if ($reader.ReadUInt16() -ne 0x5a4d) { throw 'Executable is not a PE file.' }
    $stream.Position = 0x3c
    $peOffset = $reader.ReadUInt32()
    $stream.Position = $peOffset
    if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne 0x8664) { throw 'Executable is not Windows x64.' }
    if ($desktop) {
        $stream.Position = $peOffset + 24 + 68
        if ($reader.ReadUInt16() -ne 2) { throw 'Desktop executable is not a Windows GUI-subsystem program.' }
    }
} finally { $stream.Dispose() }
Write-Host "Verified Windows x64 package $($manifest.version): $($declared.Count) payload files."
