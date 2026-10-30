#requires -Version 7.2
[CmdletBinding()]
param(
    [string]$OutputDirectory,
    [string]$CacheDirectory,
    [switch]$CheckOnly
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Build this package on Windows x64 with the MSVC build tools.' }
$project = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$app = Join-Path $project 'app'
$distribution = Join-Path $app 'distribution'
$settings = Get-Content -LiteralPath (Join-Path $distribution 'windows-dependencies.json') -Raw -Encoding utf8 | ConvertFrom-Json
$toolchain = [string]$settings.rust_toolchain
$target = [string]$settings.target
$targetDirectory = Join-Path $app 'target/windows-dist'
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $project 'dist' }
if (-not $CacheDirectory) { $CacheDirectory = Join-Path $project '.local/windows-package-cache' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$CacheDirectory = [IO.Path]::GetFullPath($CacheDirectory)
$manifestPath = Join-Path $app 'Cargo.toml'

function Invoke-Cargo {
    param([Parameter(Mandatory)][string[]]$ArgumentList)
    & cargo "+$toolchain" @ArgumentList
    if ($LASTEXITCODE -ne 0) { throw "cargo $($ArgumentList[0]) failed with exit code $LASTEXITCODE" }
}
function Assert-Hash {
    param([string]$Path, [string]$Expected)
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Expected.ToLowerInvariant()) { throw "SHA256 mismatch: $Path" }
}
function Get-VerifiedDownload {
    param([string]$Name, [string]$Url, [string]$Sha256)
    if ([Uri]::new($Url).Scheme -ne 'https' -or [IO.Path]::GetFileName($Name) -ne $Name) { throw 'Invalid dependency download specification.' }
    $destination = Join-Path $CacheDirectory $Name
    if (Test-Path -LiteralPath $destination) {
        Assert-Hash $destination $Sha256
        return $destination
    }
    $temporary = "$destination.$([Guid]::NewGuid().ToString('N')).download"
    try {
        Invoke-WebRequest -Uri $Url -OutFile $temporary -MaximumRetryCount 2 -RetryIntervalSec 3
        Assert-Hash $temporary $Sha256
        Move-Item -LiteralPath $temporary -Destination $destination
    } finally { if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force } }
    return $destination
}
function Copy-PayloadFile {
    param([string]$Source, [string]$Relative)
    $destination = Join-Path $package $Relative
    $null = New-Item -ItemType Directory -Force -Path ([IO.Path]::GetDirectoryName($destination))
    Copy-Item -LiteralPath $Source -Destination $destination
}

Push-Location -LiteralPath $project
$previousRustFlags = $env:RUSTFLAGS
try {
    # Keep the launcher usable even when the optional OCR DLL's VC runtime is absent.
    # ONNX Runtime's own dynamic VC++ dependency is documented in the user guide.
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    & rustc "+$toolchain" --version
    if ($LASTEXITCODE -ne 0) { throw "Install Rust $toolchain with rustfmt, clippy, and target $target first." }
    Invoke-Cargo -ArgumentList @('fmt', '--manifest-path', $manifestPath, '--all', '--', '--check')
    Invoke-Cargo -ArgumentList @('clippy', '--manifest-path', $manifestPath, '--locked', '--all-targets', '--target', $target, '--target-dir', $targetDirectory, '--', '-D', 'warnings')
    Invoke-Cargo -ArgumentList @('test', '--manifest-path', $manifestPath, '--locked', '--all-targets', '--target', $target, '--target-dir', $targetDirectory)
    & node --check (Join-Path $app 'web/app.js')
    if ($LASTEXITCODE -ne 0) { throw 'Frontend JavaScript syntax check failed.' }
    if ($CheckOnly) { Write-Host 'Windows checks passed. No package was produced.'; return }
    Invoke-Cargo -ArgumentList @('build', '--manifest-path', $manifestPath, '--release', '--locked', '--target', $target, '--target-dir', $targetDirectory, '--bin', 'voucher-workbench', '--example', 'package_fixture')
    $metadataText = & cargo "+$toolchain" metadata --manifest-path $manifestPath --locked --format-version 1 --filter-platform $target
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed.' }
    $metadata = ($metadataText -join "`n") | ConvertFrom-Json
    $application = @($metadata.packages | Where-Object { $_.id -eq $metadata.resolve.root })
    if ($application.Count -ne 1) { throw 'Cannot identify the application package.' }
    $version = [string]$application[0].version
    if ($version -notmatch '^\d+\.\d+\.\d+([+-][0-9A-Za-z.-]+)?$') { throw 'Invalid release version.' }
    $packageName = "voucher-workbench-$version-windows-x64"
    $null = New-Item -ItemType Directory -Force -Path $OutputDirectory, $CacheDirectory
    $zipPath = Join-Path $OutputDirectory "$packageName.zip"
    $checksumPath = "$zipPath.sha256"
    if ((Test-Path -LiteralPath $zipPath) -or (Test-Path -LiteralPath $checksumPath)) { throw 'Release output already exists; choose an empty output directory.' }
    $stage = Join-Path $OutputDirectory ('.staging-' + [Guid]::NewGuid().ToString('N'))
    $package = Join-Path $stage $packageName
    $null = New-Item -ItemType Directory -Path $package
    try {
        Copy-PayloadFile (Join-Path $targetDirectory "$target/release/voucher-workbench.exe") 'voucher-workbench.exe'
        Copy-PayloadFile (Join-Path $distribution '启动凭证工作台.cmd') '启动凭证工作台.cmd'
        Copy-PayloadFile (Join-Path $distribution 'README-Windows.txt') '使用说明.txt'
        Copy-PayloadFile (Join-Path $app 'LICENSE') 'LICENSE'
        Copy-PayloadFile (Join-Path $app 'THIRD_PARTY_NOTICES.md') 'THIRD_PARTY_NOTICES.md'
        $modelDirectory = Join-Path $app 'models/title-ocr'
        $modelManifest = Get-Content -LiteralPath (Join-Path $modelDirectory 'manifest.json') -Raw -Encoding utf8 | ConvertFrom-Json
        foreach ($model in @($modelManifest.detector, $modelManifest.recognizer)) {
            $source = Join-Path $modelDirectory $model.filename
            if (Test-Path -LiteralPath $source) { Assert-Hash $source $model.sha256 }
            else { $source = Get-VerifiedDownload $model.filename $model.url $model.sha256 }
            Copy-PayloadFile $source ('models/title-ocr/' + $model.filename)
        }
        Assert-Hash (Join-Path $modelDirectory 'dictionary.txt') $modelManifest.dictionary.sha256
        Assert-Hash (Join-Path $modelDirectory 'det.yml') $modelManifest.detector.config_sha256
        Assert-Hash (Join-Path $modelDirectory 'rec.yml') $modelManifest.recognizer.config_sha256
        foreach ($name in @('dictionary.txt', 'manifest.json', 'LICENSE', 'det.yml', 'rec.yml')) { Copy-PayloadFile (Join-Path $modelDirectory $name) "models/title-ocr/$name" }
        Copy-PayloadFile (Join-Path $app 'models/README.md') 'models/README.md'
        $runtime = $settings.runtime
        $runtimeZip = Get-VerifiedDownload $runtime.archive $runtime.url $runtime.sha256
        if ((Get-Item -LiteralPath $runtimeZip).Length -ne $runtime.bytes) { throw 'ONNX Runtime archive size mismatch.' }
        $expanded = Join-Path $stage 'runtime-source'
        Expand-Archive -LiteralPath $runtimeZip -DestinationPath $expanded
        $runtimeRoot = Join-Path $expanded $runtime.directory
        $dlls = @(Get-ChildItem -LiteralPath (Join-Path $runtimeRoot 'lib') -File -Filter '*.dll')
        if (-not ($dlls.Name -contains 'onnxruntime.dll')) { throw 'ONNX Runtime DLL not found in the official archive.' }
        foreach ($dll in $dlls) {
            if ($dll.Name -notin @('onnxruntime.dll', 'onnxruntime_providers_shared.dll')) { throw "Unexpected runtime DLL: $($dll.Name)" }
            Copy-PayloadFile $dll.FullName ('runtime/' + $dll.Name)
        }
        foreach ($name in @('LICENSE', 'ThirdPartyNotices.txt', 'Privacy.md', 'VERSION_NUMBER', 'GIT_COMMIT_ID')) {
            $source = Join-Path $runtimeRoot $name
            if (Test-Path -LiteralPath $source) { Copy-PayloadFile $source ('licenses/onnxruntime/' + $name) }
            elseif ($name -in @('LICENSE', 'ThirdPartyNotices.txt')) { throw "Official runtime license material is missing: $name" }
        }
        # Copy complete notices from the locked runtime dependency closure. Dev-only
        # fixture libraries are deliberately excluded from the distributed executable.
        $nodes = @{}
        foreach ($node in $metadata.resolve.nodes) { $nodes[$node.id] = $node }
        $visited = [Collections.Generic.HashSet[string]]::new()
        $pending = [Collections.Generic.Stack[string]]::new()
        $pending.Push([string]$metadata.resolve.root)
        while ($pending.Count -gt 0) {
            $id = $pending.Pop()
            if (-not $visited.Add($id)) { continue }
            foreach ($dependency in $nodes[$id].deps) {
                if (@($dependency.dep_kinds | Where-Object { $_.kind -ne 'dev' }).Count -gt 0) { $pending.Push([string]$dependency.pkg) }
            }
        }
        $licenseIndex = @()
        foreach ($dependency in ($metadata.packages | Sort-Object name, version)) {
            if (-not $visited.Contains($dependency.id) -or -not $dependency.source) { continue }
            $sourceRoot = Split-Path -Parent $dependency.manifest_path
            $licenseFiles = @(Get-ChildItem -LiteralPath $sourceRoot -File -Recurse | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)' -or $_.Name -eq 'UNLICENSE' } | Sort-Object FullName)
            $fallback = $false
            if ($licenseFiles.Count -eq 0 -and $dependency.name -eq 'atoi_simd' -and $dependency.version -eq '0.16.1') {
                $sourceRoot = Join-Path $distribution 'licenses/atoi_simd-0.16.1'
                $licenseFiles = @(Get-ChildItem -LiteralPath $sourceRoot -File -Filter 'LICENSE-*' | Sort-Object Name)
                $fallback = $true
            }
            if ($licenseFiles.Count -eq 0) { throw "Missing complete license text for $($dependency.name) $($dependency.version); review before distribution." }
            $copiedLicenses = @()
            for ($i = 0; $i -lt $licenseFiles.Count; $i++) {
                $file = $licenseFiles[$i]
                $safeName = $file.Name -replace '[^a-zA-Z0-9._-]', '_'
                $relative = "licenses/rust/$($dependency.name)-$($dependency.version)/$i-$safeName"
                Copy-PayloadFile $file.FullName $relative
                $copiedLicenses += [ordered]@{path=$relative; source_relative_path=[IO.Path]::GetRelativePath($sourceRoot, $file.FullName).Replace('\','/')}
            }
            $licenseIndex += [ordered]@{name=$dependency.name; version=$dependency.version; license=$dependency.license; repository=$dependency.repository; source=$dependency.source; upstream_license_fallback=$fallback; files=$copiedLicenses}
        }
        Copy-PayloadFile (Join-Path $distribution 'licenses/atoi_simd-0.16.1/SOURCE.json') 'licenses/atoi_simd-source.json'
        $licenseIndex | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $package 'licenses/rust-dependencies.json') -Encoding utf8NoBOM
        $payload = @(Get-ChildItem -LiteralPath $package -File -Recurse | Sort-Object FullName | ForEach-Object {
            [ordered]@{path=[IO.Path]::GetRelativePath($package,$_.FullName).Replace('\','/'); bytes=$_.Length; sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
        })
        $commit = $null
        if ($env:GITHUB_SHA -match '^[0-9a-f]{40}$') { $commit = $env:GITHUB_SHA }
        $packageManifest = [ordered]@{
            schema_version=1; name='voucher-workbench'; version=$version; target=$target;
            rust_toolchain=$toolchain; rust_crt='static'; source_commit=$commit;
            created_at=[DateTimeOffset]::UtcNow.ToString('o');
            cargo_lock_sha256=(Get-FileHash -LiteralPath (Join-Path $app 'Cargo.lock') -Algorithm SHA256).Hash.ToLowerInvariant();
            runtime=$runtime; models=$modelManifest; files=$payload
        }
        $packageManifest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $package 'package-manifest.json') -Encoding utf8NoBOM
        & (Join-Path $PSScriptRoot 'verify-windows-package.ps1') -PackageDirectory $package
        Compress-Archive -LiteralPath $package -DestinationPath $zipPath -CompressionLevel Optimal
        $zipHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
        "$zipHash  $packageName.zip" | Set-Content -LiteralPath $checksumPath -Encoding ascii
        Write-Host "Package created: $zipPath"
        Write-Host "Checksum created: $checksumPath"
        Write-Host 'Run smoke-windows.ps1 against the ZIP before distributing it.'
    } finally { if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force } }
} finally { $env:RUSTFLAGS = $previousRustFlags; Pop-Location }
