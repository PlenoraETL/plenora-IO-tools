[CmdletBinding()]
param(
    [int]$Rows = 50000,
    [int]$Fields = 64,
    [int]$Pairs = 7,
    [double]$MaximumRegressionPercent = 5.0,
    [string]$OutputPath = (Join-Path (Get-Location) "target\windows-filegdb-narrow-benchmark.json")
)

$ErrorActionPreference = "Stop"
$culture = [System.Globalization.CultureInfo]::InvariantCulture
$workRoot = if ($env:RUNNER_TEMP) {
    Join-Path $env:RUNNER_TEMP "plenora-filegdb-benchmark"
} else {
    Join-Path (Get-Location) "target\windows-filegdb-benchmark"
}
$baselineSource = Join-Path $workRoot "baseline-source"
$sharedTarget = Join-Path $workRoot "cargo-target"
$baselineBinary = Join-Path $workRoot "projection-bench-baseline.exe"
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

$baselineTag = "v0.1.0-rc.3"
$baselineRevision = (& git rev-list -n 1 $baselineTag).Trim()
if ($LASTEXITCODE -ne 0 -or -not $baselineRevision) {
    throw "Cannot resolve immutable baseline tag $baselineTag"
}
$baselineArchive = Join-Path $workRoot "baseline.tar"
& git archive --format=tar --output=$baselineArchive $baselineTag
if ($LASTEXITCODE -ne 0) {
    throw "Cannot archive immutable baseline tag $baselineTag"
}
New-Item -ItemType Directory -Path $baselineSource -Force | Out-Null
tar -xf $baselineArchive -C $baselineSource
if ($LASTEXITCODE -ne 0) {
    throw "Cannot extract immutable baseline tag $baselineTag"
}

Invoke-CargoBuild -SourceDirectory $baselineSource -OutputBinary $baselineBinary
Invoke-CargoBuild -SourceDirectory (Get-Location).Path -OutputBinary $candidateBinary

foreach ($binary in @($baselineBinary, $candidateBinary)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "Benchmark binary not found: $binary"
    }
}

& $candidateBinary generate $fixturePath $Rows $Fields
if ($LASTEXITCODE -ne 0) {
    throw "Cannot generate the FileGDB benchmark fixture"
}

$baselineSamples = @()
$candidateSamples = @()
for ($pair = 0; $pair -lt $Pairs; $pair++) {
    if ($pair % 2 -eq 0) {
        $baselineSamples += Invoke-NarrowSample -Variant "rc3" -Binary $baselineBinary
        $candidateSamples += Invoke-NarrowSample -Variant "rc4" -Binary $candidateBinary
    } else {
        $candidateSamples += Invoke-NarrowSample -Variant "rc4" -Binary $candidateBinary
        $baselineSamples += Invoke-NarrowSample -Variant "rc3" -Binary $baselineBinary
    }
}

$allSamples = @($baselineSamples) + @($candidateSamples)
$expectedChecksum = [long]$allSamples[0].checksum
foreach ($sample in $allSamples) {
    if ($sample.rows -ne $Rows -or $sample.checksum -ne $expectedChecksum) {
        throw "Benchmark oracle mismatch in $($sample.variant)"
    }
}

$baselineMedian = Get-Median -Values @($baselineSamples | ForEach-Object { $_.elapsed_ms })
$candidateMedian = Get-Median -Values @($candidateSamples | ForEach-Object { $_.elapsed_ms })
$deltaPercent = (($candidateMedian / $baselineMedian) - 1.0) * 100.0
$status = if ($deltaPercent -le $MaximumRegressionPercent) { "passed" } else { "vetoed" }
$revision = (& git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "Cannot resolve benchmark source revision"
}

$result = [ordered]@{
    schema_version = 1
    source_revision = $revision
    platform = "windows-x86_64-msvc"
    runtime = @{
        gdal = "3.10.3"
        rust_gdal = "0.17.1"
        gdal_sys = "0.10.0"
        bindings = "prebuilt-3.6"
    }
    fixture = @{
        rows = $Rows
        fields = $Fields
        mode = "narrow"
        projected_attributes = 3
        checksum = $expectedChecksum
    }
    protocol = @{
        pairs = $Pairs
        order = "interlaced"
        warmup_per_sample = 1
        maximum_regression_percent = $MaximumRegressionPercent
    }
    baseline = @{
        tag = $baselineTag
        source_revision = $baselineRevision
        elapsed_ms = @($baselineSamples | ForEach-Object { $_.elapsed_ms })
        median_elapsed_ms = $baselineMedian
    }
    candidate = @{
        source_revision = $revision
        elapsed_ms = @($candidateSamples | ForEach-Object { $_.elapsed_ms })
        median_elapsed_ms = $candidateMedian
    }
    delta_percent = $deltaPercent
    status = $status
}

$resolvedOutputPath = [System.IO.Path]::GetFullPath($OutputPath)
$outputDirectory = Split-Path -Parent $resolvedOutputPath
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
$result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $resolvedOutputPath -Encoding utf8
$result | ConvertTo-Json -Depth 6

if ($status -ne "passed") {
    throw "FileGDB narrow-path regression is $($deltaPercent.ToString('F2', $culture))%, above the $MaximumRegressionPercent% veto"
}
