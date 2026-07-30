[CmdletBinding()]
param(
    [string]$Destination = (Join-Path (Get-Location) "target\windows-gdal"),
    [string]$CacheDirectory = (Join-Path ([System.IO.Path]::GetTempPath()) "plenora-osgeo4w-cache"),
    [switch]$ExportGitHubEnvironment
)

$ErrorActionPreference = "Stop"

function Assert-Archive {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [long]$ExpectedSize,
        [Parameter(Mandatory = $true)]
        [string]$ExpectedSha256
    )

    $item = Get-Item -LiteralPath $Path
    if ($item.Length -ne $ExpectedSize) {
        throw "Size mismatch for $Path (expected $ExpectedSize, found $($item.Length))"
    }
    $actualSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
    if ($actualSha256 -ne $ExpectedSha256.ToLowerInvariant()) {
        throw "SHA-256 mismatch for $Path"
    }
}

$manifestPath = Join-Path $PSScriptRoot "windows-gdal-lock.json"
$manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
if ($manifest.manifest_version -ne 1) {
    throw "Unsupported Windows GDAL lock manifest version: $($manifest.manifest_version)"
}
if ($manifest.gdal_version -ne "3.10.3" -or $manifest.binding_version -ne "3.6.0") {
    throw "Unexpected Windows GDAL runtime/binding contract"
}
if ($manifest.architecture -ne "x86_64" -or -not [Environment]::Is64BitOperatingSystem) {
    throw "The pinned OSGeo4W environment requires 64-bit Windows"
}

$destinationPath = [System.IO.Path]::GetFullPath($Destination)
$cachePath = [System.IO.Path]::GetFullPath($CacheDirectory)
if (Test-Path -LiteralPath $destinationPath) {
    $existing = @(Get-ChildItem -LiteralPath $destinationPath -Force)
    if ($existing.Count -ne 0) {
        throw "Destination must be absent or empty: $destinationPath"
    }
}
New-Item -ItemType Directory -Path $destinationPath -Force | Out-Null
New-Item -ItemType Directory -Path $cachePath -Force | Out-Null

foreach ($package in $manifest.packages) {
    $relativePath = [string]$package[0]
    $expectedSize = [long]$package[1]
    $expectedSha256 = [string]$package[2]
    $archiveName = Split-Path -Leaf $relativePath
    $archivePath = Join-Path $cachePath $archiveName

    if (-not (Test-Path -LiteralPath $archivePath)) {
        $partialPath = "$archivePath.$([Guid]::NewGuid().ToString('N')).partial"
        $uri = [Uri]::new([Uri]$manifest.base_url, $relativePath)
        Write-Host "Downloading $archiveName"
        Invoke-WebRequest -Uri $uri -OutFile $partialPath
        Assert-Archive -Path $partialPath -ExpectedSize $expectedSize -ExpectedSha256 $expectedSha256
        Move-Item -LiteralPath $partialPath -Destination $archivePath
    }
    Assert-Archive -Path $archivePath -ExpectedSize $expectedSize -ExpectedSha256 $expectedSha256

    $members = @(tar -tf $archivePath)
    if ($LASTEXITCODE -ne 0) {
        throw "Cannot list archive $archivePath"
    }
    foreach ($member in $members) {
        $normalized = $member.Replace("\", "/")
        if (
            $normalized.StartsWith("/") -or
            $normalized -match "^[A-Za-z]:" -or
            $normalized -match "(^|/)\.\.(/|$)"
        ) {
            throw "Unsafe member '$member' in $archivePath"
        }
    }

    tar -xf $archivePath -C $destinationPath
    if ($LASTEXITCODE -ne 0) {
        throw "Cannot extract archive $archivePath"
    }
}

foreach ($relativePath in @(
    "bin\gdal310.dll",
    "bin\gdalinfo.exe",
    "bin\ogrinfo.exe",
    "include\gdal.h",
    "lib\gdal_i.lib"
)) {
    $requiredPath = Join-Path $destinationPath $relativePath
    if (-not (Test-Path -LiteralPath $requiredPath -PathType Leaf)) {
        throw "Pinned GDAL environment is incomplete: $requiredPath"
    }
}

$binPath = Join-Path $destinationPath "bin"
$env:PATH = "$binPath;$env:PATH"
$env:OSGEO4W_ROOT = $destinationPath
$env:GDAL_HOME = $destinationPath
$env:GDAL_VERSION = [string]$manifest.binding_version
$env:PLENORA_GDAL_RUNTIME_VERSION = [string]$manifest.gdal_version
$env:GDAL_DATA = Join-Path $destinationPath "share\gdal"
$env:PROJ_DATA = Join-Path $destinationPath "share\proj"

$reportedVersion = (& (Join-Path $binPath "gdalinfo.exe") --version 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $reportedVersion -notmatch "^GDAL 3\.10\.3,") {
    throw "Unexpected GDAL runtime: $reportedVersion"
}
$formats = (& (Join-Path $binPath "ogrinfo.exe") --formats 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0 -or $formats -notmatch "(?m)^\s+OpenFileGDB .*\(rw\+v\):") {
    throw "The pinned runtime does not expose writable OpenFileGDB"
}

if ($ExportGitHubEnvironment) {
    if (-not $env:GITHUB_ENV -or -not $env:GITHUB_PATH) {
        throw "GitHub environment export requested outside GitHub Actions"
    }
    @(
        "OSGEO4W_ROOT=$destinationPath",
        "GDAL_HOME=$destinationPath",
        "GDAL_VERSION=$($manifest.binding_version)",
        "PLENORA_GDAL_RUNTIME_VERSION=$($manifest.gdal_version)",
        "GDAL_DATA=$($env:GDAL_DATA)",
        "PROJ_DATA=$($env:PROJ_DATA)"
    ) | ForEach-Object {
        Add-Content -LiteralPath $env:GITHUB_ENV -Value $_ -Encoding utf8
    }
    Add-Content -LiteralPath $env:GITHUB_PATH -Value $binPath -Encoding utf8
}

[pscustomobject]@{
    destination = $destinationPath
    gdal_version = $reportedVersion
    binding_version = [string]$manifest.binding_version
    openfilegdb = "rw+v"
    package_count = @($manifest.packages).Count
}
