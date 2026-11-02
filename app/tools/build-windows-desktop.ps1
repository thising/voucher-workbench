#requires -Version 7.2
[CmdletBinding()]
param([string]$OutputDirectory, [string]$CacheDirectory, [switch]$CheckOnly)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Build the desktop package on Windows x64 with MSVC and WebView2.' }
$project = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$app = Join-Path $project 'app'
$desktop = Join-Path $app 'desktop'
$settings = Get-Content (Join-Path $app 'distribution/windows-dependencies.json') -Raw -Encoding utf8 | ConvertFrom-Json
$toolchain = [string]$settings.rust_toolchain
$target = [string]$settings.target
$targetDirectory = Join-Path $desktop 'target/windows-dist'
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $project 'dist' }
if (-not $CacheDirectory) { $CacheDirectory = Join-Path $project '.local/windows-package-cache' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$CacheDirectory = [IO.Path]::GetFullPath($CacheDirectory)
$null = New-Item -ItemType Directory -Force -Path $OutputDirectory, $CacheDirectory
$manifest = Join-Path $desktop 'Cargo.toml'
function Invoke-DesktopCargo([string[]]$Arguments) {
    & cargo "+$toolchain" @Arguments
    if ($LASTEXITCODE -ne 0) { throw "cargo $($Arguments[0]) failed ($LASTEXITCODE)." }
}
function Hash([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Copy-Payload([string]$Source, [string]$Relative) {
    $destination = Join-Path $package $Relative
    $null = New-Item -ItemType Directory -Force -Path ([IO.Path]::GetDirectoryName($destination))
    Copy-Item -LiteralPath $Source -Destination $destination
}
function Write-Checksum([string]$Path) { "$(Hash $Path)  $([IO.Path]::GetFileName($Path))" | Set-Content -LiteralPath "$Path.sha256" -Encoding ascii }
function Download-OfflineRuntime {
    $spec = $settings.desktop.webview2
    $path = Join-Path $CacheDirectory $spec.filename
    if (-not (Test-Path -LiteralPath $path)) {
        $temporary = "$path.$([Guid]::NewGuid().ToString('N')).download"
        try {
            Invoke-WebRequest -Uri $spec.url -OutFile $temporary -MaximumRetryCount 2 -RetryIntervalSec 3
            if ((Hash $temporary) -ne $spec.sha256) { throw 'WebView2 offline installer digest differs from the reviewed dependency manifest.' }
            Move-Item -LiteralPath $temporary -Destination $path
        } finally { if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force } }
    }
    if ((Hash $path) -ne $spec.sha256 -or (Get-Item -LiteralPath $path).Length -ne $spec.bytes) { throw 'WebView2 offline installer hash/size mismatch.' }
    $signature = Get-AuthenticodeSignature -LiteralPath $path
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(^|,\s*)O=Microsoft Corporation(,|$)') { throw 'The offline WebView2 installer lacks a valid Microsoft Authenticode signature.' }
    return $path
}
function Copy-DesktopLicenses($Metadata) {
    $nodes = @{}
    foreach ($node in $Metadata.resolve.nodes) { $nodes[$node.id] = $node }
    $visited = [Collections.Generic.HashSet[string]]::new()
    $pending = [Collections.Generic.Stack[string]]::new()
    $pending.Push([string]$Metadata.resolve.root)
    while ($pending.Count -gt 0) {
        $id = $pending.Pop()
        if (-not $visited.Add($id)) { continue }
        foreach ($dependency in $nodes[$id].deps) {
            if (@($dependency.dep_kinds | Where-Object { $_.kind -ne 'dev' }).Count -gt 0) { $pending.Push([string]$dependency.pkg) }
        }
    }
    $index = @()
    foreach ($dependency in ($Metadata.packages | Sort-Object name, version)) {
        if (-not $visited.Contains($dependency.id) -or -not $dependency.source) { continue }
        $sourceRoot = Split-Path -Parent $dependency.manifest_path
        $files = @(Get-ChildItem -LiteralPath $sourceRoot -File -Recurse | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)' -or $_.Name -eq 'UNLICENSE' } | Sort-Object FullName)
        $fallback = $false
        if ($files.Count -eq 0) {
            $sourceRoot = Join-Path $app "distribution/licenses/$($dependency.name)-$($dependency.version)"
            $provenance = Join-Path $sourceRoot 'SOURCE.json'
            if (Test-Path -LiteralPath $provenance) {
                $record = Get-Content -LiteralPath $provenance -Raw -Encoding utf8 | ConvertFrom-Json
                if ($record.crate -ne $dependency.name -or $record.version -ne $dependency.version) { throw 'Fallback license provenance identifies a different crate.' }
                $files = @()
                foreach ($entry in $record.files) {
                    $fileName = if ($entry.PSObject.Properties['path']) { [string]$entry.path } else { [string]$entry.file }
                    if ([IO.Path]::GetFileName($fileName) -ne $fileName) { throw 'Fallback license must be a reviewed plain filename.' }
                    $license = Join-Path $sourceRoot $fileName
                    if ((Hash $license) -ne $entry.sha256) { throw 'Fallback license digest differs from its reviewed source record.' }
                    $files += Get-Item -LiteralPath $license
                }
                Copy-Payload $provenance "licenses/rust-desktop/$($dependency.name)-$($dependency.version)/SOURCE.json"
                $fallback = $true
            }
        }
        if ($files.Count -eq 0) { throw "Missing license text for desktop dependency $($dependency.name) $($dependency.version)." }
        $copied = @()
        for ($i = 0; $i -lt $files.Count; $i++) {
            $file = $files[$i]
            $name = $file.Name -replace '[^a-zA-Z0-9._-]', '_'
            $relative = "licenses/rust-desktop/$($dependency.name)-$($dependency.version)/$i-$name"
            Copy-Payload $file.FullName $relative
            $copied += [ordered]@{path=$relative; source_relative_path=[IO.Path]::GetRelativePath($sourceRoot,$file.FullName).Replace('\','/')}
        }
        $index += [ordered]@{name=$dependency.name; version=$dependency.version; license=$dependency.license; source=$dependency.source; source_archive_url="https://static.crates.io/crates/$($dependency.name)/$($dependency.name)-$($dependency.version).crate"; repository=$dependency.repository; upstream_license_fallback=$fallback; files=$copied}
    }
    $index | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $package 'licenses/rust-desktop-dependencies.json') -Encoding utf8NoBOM
}
$oldFlags = $env:RUSTFLAGS
$oldTarget = $env:CARGO_TARGET_DIR
$oldToolchain = $env:RUSTUP_TOOLCHAIN
$stage = Join-Path $CacheDirectory ('desktop-build-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $stage
try {
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    $env:RUSTUP_TOOLCHAIN = $toolchain
    # Existing core checks and dependency verification remain authoritative for
    # compatibility. This intermediate CLI package is never a desktop release.
    & (Join-Path $PSScriptRoot 'build-windows.ps1') -CheckOnly:$CheckOnly -OutputDirectory (Join-Path $stage 'core') -CacheDirectory $CacheDirectory
    Invoke-DesktopCargo -Arguments @('fmt','--manifest-path',$manifest,'--all','--','--check')
    Invoke-DesktopCargo -Arguments @('clippy','--manifest-path',$manifest,'--locked','--all-targets','--target',$target,'--target-dir',$targetDirectory,'--','-D','warnings')
    Invoke-DesktopCargo -Arguments @('test','--manifest-path',$manifest,'--locked','--all-targets','--target',$target,'--target-dir',$targetDirectory)
    & node --check (Join-Path $app 'web/workbench.js')
    if ($LASTEXITCODE -ne 0) { throw 'Desktop frontend syntax check failed.' }
    & node (Join-Path $PSScriptRoot 'test-workbench-ui.cjs')
    if ($LASTEXITCODE -ne 0) { throw 'Synthetic desktop UI contract tests failed.' }
    if ($CheckOnly) { Write-Host 'Core and desktop Windows checks passed.'; return }
    $metadataText = & cargo "+$toolchain" metadata --manifest-path $manifest --locked --format-version 1 --filter-platform $target
    if ($LASTEXITCODE -ne 0) { throw 'Desktop cargo metadata failed.' }
    $metadata = ($metadataText -join "`n") | ConvertFrom-Json
    $root = @($metadata.packages | Where-Object { $_.id -eq $metadata.resolve.root })[0]
    $version = [string]$root.version
    if ($version -notmatch '^\d+\.\d+\.\d+([+-][0-9A-Za-z.-]+)?$') { throw 'Invalid desktop version.' }
    $name = "pdf-workbench-$version-windows-x64"
    $zip = Join-Path $OutputDirectory "$name.zip"
    $installer = Join-Path $OutputDirectory "$name-setup.exe"
    $releaseManifest = Join-Path $OutputDirectory "$name-release.json"
    foreach ($path in @($zip,$installer,$releaseManifest,"$zip.sha256","$installer.sha256")) { if (Test-Path -LiteralPath $path) { throw "Release output exists: $path" } }
    $coreZip = @(Get-ChildItem (Join-Path $stage 'core') -File -Filter '*.zip')
    if ($coreZip.Count -ne 1) { throw 'Expected one verified intermediate core archive.' }
    $examples = Join-Path $app "target/windows-dist/$target/release/examples"
    & (Join-Path $PSScriptRoot 'smoke-windows-launcher.ps1') -ZipPath $coreZip[0].FullName -FixtureExecutable (Join-Path $examples 'package_fixture.exe') -ProbeExecutable (Join-Path $examples 'startup_probe.exe')
    $unpacked = Join-Path $stage 'unpacked'
    Expand-Archive -LiteralPath $coreZip[0].FullName -DestinationPath $unpacked
    $corePackage = @(Get-ChildItem -LiteralPath $unpacked -Directory)[0].FullName
    $coreInfo = Get-Content (Join-Path $corePackage 'package-manifest.json') -Raw -Encoding utf8 | ConvertFrom-Json
    $package = Join-Path $stage $name
    Move-Item -LiteralPath $corePackage -Destination $package
    foreach ($oldEntry in @('voucher-workbench.exe','启动凭证工作台.cmd','package-manifest.json')) { Remove-Item -LiteralPath (Join-Path $package $oldEntry) }
    # The shipped binary uses the desktop lock; do not carry a second, obsolete
    # core dependency closure in the customer package.
    Remove-Item -LiteralPath (Join-Path $package 'licenses/rust') -Recurse -Force
    Remove-Item -LiteralPath (Join-Path $package 'licenses/rust-dependencies.json')
    Copy-DesktopLicenses $metadata
    foreach ($file in @(Get-ChildItem (Join-Path $app 'web/pdfjs') -File -Recurse | Where-Object { $_.Name -match '^(LICENSE|NOTICE)' -or $_.Name -eq 'manifest.json' })) {
        Copy-Payload $file.FullName ('licenses/pdfjs/' + [IO.Path]::GetRelativePath((Join-Path $app 'web/pdfjs'),$file.FullName).Replace('\','/'))
    }
    Copy-Payload (Join-Path $app 'desktop/package-lock.json') 'licenses/tauri-cli-package-lock.json'
    Copy-Payload (Join-Path $desktop 'Cargo.lock') 'licenses/desktop-Cargo.lock'
    $commit = if ($env:GITHUB_SHA -match '^[0-9a-f]{40}$') { $env:GITHUB_SHA } else { $null }
    $source = [ordered]@{schema_version=1; name='pdf-workbench'; version=$version; source_commit=$commit; rust_toolchain=$toolchain; rust_crt='static'; binary_patching='disabled-no-updater'; core_cargo_lock_sha256=(Hash (Join-Path $app 'Cargo.lock')); desktop_cargo_lock_sha256=(Hash (Join-Path $desktop 'Cargo.lock')); build_tools=$settings.desktop; runtime=$settings.runtime; models=$coreInfo.models; pdfjs=(Get-Content (Join-Path $app 'web/pdfjs/manifest.json') -Raw -Encoding utf8 | ConvertFrom-Json)}
    $source | ConvertTo-Json -Depth 16 | Set-Content (Join-Path $package 'release-source.json') -Encoding utf8NoBOM
    & npm ci --prefix $desktop --ignore-scripts --no-audit --no-fund --cache (Join-Path $CacheDirectory 'npm')
    if ($LASTEXITCODE -ne 0) { throw 'Pinned Tauri CLI installation failed.' }
    $resources = [ordered]@{}
    foreach ($directory in @('models','runtime','licenses')) { $resources[(Join-Path $package "$directory/")] = "$directory/" }
    foreach ($file in @('使用说明.txt','LICENSE','THIRD_PARTY_NOTICES.md','release-source.json')) { $resources[(Join-Path $package $file)] = $file }
    $config = Join-Path $stage 'bundle-resources.json'
    @{bundle=@{resources=$resources}} | ConvertTo-Json -Depth 8 | Set-Content $config -Encoding utf8NoBOM
    $env:CARGO_TARGET_DIR = $targetDirectory
    Push-Location -LiteralPath $desktop
    try {
        # Tauri otherwise stamps the NSIS payload and restores the original EXE
        # afterwards. No updater is used; keep portable and installed bytes equal.
        & node node_modules/@tauri-apps/cli/tauri.js build --ci --target $target --bundles nsis --no-binary-patching --config $config -- --locked
        if ($LASTEXITCODE -ne 0) { throw 'Tauri NSIS build failed.' }
    } finally { Pop-Location }
    Copy-Payload (Join-Path $targetDirectory "$target/release/pdf-workbench.exe") 'PDF工作台.exe'
    $installers = @(Get-ChildItem (Join-Path $targetDirectory "$target/release/bundle/nsis") -File -Filter '*-setup.exe')
    if ($installers.Count -ne 1) { throw 'Expected exactly one current-user NSIS installer.' }
    Copy-Item -LiteralPath $installers[0].FullName -Destination $installer
    $payload = @(Get-ChildItem -LiteralPath $package -File -Recurse | Sort-Object FullName | ForEach-Object { [ordered]@{path=[IO.Path]::GetRelativePath($package,$_.FullName).Replace('\','/'); bytes=$_.Length; sha256=(Hash $_.FullName)} })
    $packageInfo = [ordered]@{schema_version=2; name='pdf-workbench'; ui='tauri'; executable='PDF工作台.exe'; version=$version; target=$target; source_commit=$commit; created_at=[DateTimeOffset]::UtcNow.ToString('o'); rust_toolchain=$toolchain; rust_crt='static'; cargo_lock_sha256=$source.desktop_cargo_lock_sha256; runtime=$settings.runtime; models=$coreInfo.models; files=$payload}
    $packageInfo | ConvertTo-Json -Depth 16 | Set-Content (Join-Path $package 'package-manifest.json') -Encoding utf8NoBOM
    & (Join-Path $PSScriptRoot 'verify-windows-package.ps1') -PackageDirectory $package
    Compress-Archive -LiteralPath $package -DestinationPath $zip -CompressionLevel Optimal
    Write-Checksum $zip
    Write-Checksum $installer
    $offline = Download-OfflineRuntime
    $offlineDestination = Join-Path $OutputDirectory $settings.desktop.webview2.filename
    if (Test-Path -LiteralPath $offlineDestination) { throw 'Offline runtime attachment already exists.' }
    Copy-Item -LiteralPath $offline -Destination $offlineDestination
    Write-Checksum $offlineDestination
    $attachments = @($zip,$installer,$offlineDestination) | ForEach-Object { [ordered]@{name=[IO.Path]::GetFileName($_); bytes=(Get-Item -LiteralPath $_).Length; sha256=(Hash $_)} }
    [ordered]@{schema_version=1; product='PDF工作台'; version=$version; source=$source; executable_sha256=(Hash (Join-Path $package 'PDF工作台.exe')); attachments=$attachments; validation='See desktop-smoke-report.json from this workflow; customer Windows 10 double-click acceptance remains separate.'} | ConvertTo-Json -Depth 18 | Set-Content $releaseManifest -Encoding utf8NoBOM
    Write-Checksum $releaseManifest
    Write-Host "Desktop portable, installer, and independent WebView2 attachment created: $name"
} finally {
    $env:RUSTFLAGS = $oldFlags
    $env:CARGO_TARGET_DIR = $oldTarget
    $env:RUSTUP_TOOLCHAIN = $oldToolchain
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
}
