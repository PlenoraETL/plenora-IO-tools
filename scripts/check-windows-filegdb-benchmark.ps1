[CmdletBinding()]
param(
    [ValidateRange(1, 1000000)][int]$Rows = 50000,
    [ValidateRange(3, 1024)][int]$Fields = 64,
    [ValidateRange(1, 100)][int]$Samples = 7,
    [string]$OutputPath = (Join-Path (Get-Location) "target\windows-filegdb-narrow-benchmark.json")
)

$ErrorActionPreference = "Stop"
$culture = [System.Globalization.CultureInfo]::InvariantCulture
$workRoot = if ($env:RUNNER_TEMP) {
    Join-Path $env:RUNNER_TEMP "plenora-filegdb-benchmark"
} else {
    Join-Path (Get-Location) "target\windows-filegdb-benchmark"
}
$sharedTarget = Join-Path $workRoot "cargo-target"
$candidateBinary = Join-Path $workRoot "projection-bench-candidate.exe"
$fixturePath = Join-Path $workRoot "wide.gdb"

if (Test-Path -LiteralPath $fixturePath) {
    throw "Benchmark fixture already exists: $fixturePath"
}
New-Item -ItemType Directory -Path $workRoot -Force | Out-Null

function Invoke-CargoBuild {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SourceDirectory,
        [Parameter(Mandatory = $true)]
        [string]$OutputBinary
    )

    Push-Location $SourceDirectory
    try {
        & cargo build `
            -p driver-filegdb `
            --example projection_bench `
            --release `
            --features gdal-backend `
            --locked `
            --target-dir $sharedTarget
        if ($LASTEXITCODE -ne 0) {
            throw "Benchmark build failed in $SourceDirectory"
        }
    } finally {
        Pop-Location
    }

    Copy-Item `
        -LiteralPath (Join-Path $sharedTarget "release\examples\projection_bench.exe") `
        -Destination $OutputBinary
}

function Invoke-NarrowSample {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Variant,
        [Parameter(Mandatory = $true)]
        [string]$Binary
    )

    $output = @(& $Binary read $fixturePath 1 narrow 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw "$Variant benchmark process failed: $($output -join [Environment]::NewLine)"
    }
    $line = $output | Where-Object { $_ -match "^run=1 mode=narrow " } | Select-Object -Last 1
    if (
        -not $line -or
        $line -notmatch "rows=(\d+) elapsed_ms=([0-9.]+) rows_per_second=([0-9.]+) checksum=(-?\d+)"
    ) {
        throw "Cannot parse $Variant benchmark output: $($output -join [Environment]::NewLine)"
    }

    [pscustomobject]@{
        variant = $Variant
        rows = [long]$matches[1]
        elapsed_ms = [double]::Parse($matches[2], $culture)
        rows_per_second = [double]::Parse($matches[3], $culture)
        checksum = [long]$matches[4]
    }
}

function Get-Median {
    param(
        [Parameter(Mandatory = $true)]
        [double[]]$Values
    )

    $ordered = @($Values | Sort-Object)
    $middle = [int][Math]::Floor($ordered.Count / 2)
    if ($ordered.Count % 2 -eq 1) {
        return $ordered[$middle]
    }
    return ($ordered[$middle - 1] + $ordered[$middle]) / 2.0
}

Invoke-CargoBuild -SourceDirectory (Get-Location).Path -OutputBinary $candidateBinary

foreach ($binary in @($candidateBinary)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "Benchmark binary not found: $binary"
    }
}

& $candidateBinary generate $fixturePath $Rows $Fields
if ($LASTEXITCODE -ne 0) {
    throw "Cannot generate the FileGDB benchmark fixture"
}

$candidateSamples = @()
for ($sample = 0; $sample -lt $Samples; $sample++) {
    $candidateSamples += Invoke-NarrowSample -Variant "current" -Binary $candidateBinary
}

# La fixture genera field_i = row + i. La proiezione conserva tre attributi.
$expectedChecksum = [long](3 * ([long]$Rows * ($Rows - 1) / 2) + [long]$Rows * ([Math]::Floor($Fields / 2) + $Fields - 1))
foreach ($sample in $candidateSamples) {
    if ($sample.rows -ne $Rows -or $sample.checksum -ne $expectedChecksum) {
        throw "Benchmark oracle mismatch in $($sample.variant)"
    }
}
$candidateMedian = Get-Median -Values @($candidateSamples | ForEach-Object { $_.elapsed_ms })
$revision = (& git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "Cannot resolve benchmark source revision"
}

# Le versioni si leggono dal lock, non si ricopiano qui: una versione scritta a
# mano e' una seconda verita', ed e' cosi' che 3.10.3 e' sopravvissuta al cambio
# di contratto in tre documenti diversi.
$lock = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot "windows-gdal-lock.json") | ConvertFrom-Json
$serieBinding = (([string]$lock.binding_version) -split '\.')[0..1] -join '.'

# Le versioni dei crate si leggono dai lock, per la stessa ragione del runtime:
# fino alla 4.1.1 qui c'erano `0.17.1` e `0.10.0` scritti a mano, e il verbale
# del benchmark ha continuato a dichiararli dopo il passaggio a 0.19.0 / 0.12.0.
$forkGdal = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot "gdal-fork-lock.json") | ConvertFrom-Json
$cargoLock = Get-Content -Raw -LiteralPath (Join-Path (Split-Path -Parent $PSScriptRoot) "Cargo.lock")
function Get-LockedVersion {
    param([Parameter(Mandatory = $true)][string]$Name)
    $trovate = [regex]::Matches($cargoLock, "(?m)^name = `"$([regex]::Escape($Name))`"\r?\nversion = `"([^`"]+)`"")
    if ($trovate.Count -ne 1) {
        throw "Cargo.lock non risolve un'unica versione di $Name"
    }
    return $trovate[0].Groups[1].Value
}
# Il pacchetto si cerca con il nome con cui il fork entra nel grafo: dalla
# 4.2.0 e' un nome proprio (`fork_package`), prima era quello upstream.
$nomeFork = if ($forkGdal.PSObject.Properties.Name -contains 'fork_package') { [string]$forkGdal.fork_package } else { [string]$forkGdal.package }
$rustGdal = Get-LockedVersion -Name $nomeFork
$gdalSys = Get-LockedVersion -Name "gdal-sys"
if ($rustGdal -ne [string]$forkGdal.version) {
    throw "Cargo.lock e gdal-fork-lock.json non concordano sulla versione di gdal"
}

$result = [ordered]@{
    schema_version = 2
    source_revision = $revision
    platform = "windows-x86_64-msvc"
    runtime = @{
        # Letta dal lock e non ricopiata: una versione scritta a mano qui e' una
        # seconda verita', ed e' cosi' che 3.10.3 e' sopravvissuta al cambio di
        # contratto in tre documenti diversi.
        gdal = [string]$lock.gdal_version
        rust_gdal = $rustGdal
        rust_gdal_source = "governed-path-fork"
        rust_gdal_upstream_checksum = [string]$forkGdal.crate_sha256
        gdal_sys = $gdalSys
        bindings = "prebuilt-$serieBinding"
    }
    fixture = @{
        rows = $Rows
        fields = $Fields
        mode = "narrow"
        projected_attributes = 3
        checksum = $expectedChecksum
    }
    protocol = @{
        samples = $Samples
        warmup_per_sample = 1
    }
    current = @{
        source_revision = $revision
        elapsed_ms = @($candidateSamples | ForEach-Object { $_.elapsed_ms })
        median_elapsed_ms = $candidateMedian
    }
    status = "passed"
}

$resolvedOutputPath = [System.IO.Path]::GetFullPath($OutputPath)
$outputDirectory = Split-Path -Parent $resolvedOutputPath
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
$result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $resolvedOutputPath -Encoding utf8
$result | ConvertTo-Json -Depth 6
