# Materializza il runtime GDAL fissato per Windows, dal lock e senza solver.
#
# # Che cosa e' cambiato, e perche'
#
# La catena precedente era OSGeo4W. Portava GDAL 3.10.3, mentre `gdal-sys 0.10.0`
# spedisce binding pre-costruiti soltanto fino a 3.9 -- e per farla compilare
# questo script dichiarava `GDAL_VERSION=3.6.0`. Si compilavano cosi' i binding
# di una serie contro la libreria di un'altra: funziona finche' funziona, e
# quando smette non lo dice. Nessun gate lo vedeva, perche' la forzatura
# mascherava proprio la condizione che avrebbe fatto fermare la build.
#
# Ora Windows usa la stessa catena di Linux e di macOS: conda-forge, GDAL 3.9.3,
# binding 3.9 **veri**, e la versione dichiarata a `gdal-sys` e' quella spedita.
# Un artefatto che porta la stessa versione da tre catene diverse non ha la
# stessa identita'; con una catena sola quella domanda non si pone.
#
# # Che cosa questo script **non** ha ancora
#
# Non e' mai stato eseguito. Non esiste in questo lotto un runner Windows su cui
# provarlo, e uno script PowerShell che non ha girato e' una dichiarazione, non
# una verifica. Il blocco corrispondente sta in `blocchi_aperti` nella matrice di
# distribuzione, e va chiuso su `windows-2022` misurando -- non rileggendo.

[CmdletBinding()]
param(
    [string]$Destination = (Join-Path (Get-Location) "target\windows-gdal"),
    [string]$CacheDirectory = (Join-Path ([System.IO.Path]::GetTempPath()) "plenora-conda-cache"),
    [switch]$ExportGitHubEnvironment
)

$ErrorActionPreference = "Stop"

function Assert-Archive {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][long]$ExpectedSize,
        [Parameter(Mandatory = $true)][string]$ExpectedSha256
    )
    $item = Get-Item -LiteralPath $Path
    if ($item.Length -ne $ExpectedSize) {
        throw "Size mismatch for $Path (expected $ExpectedSize, found $($item.Length))"
    }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
    if ($actual -ne $ExpectedSha256.ToLowerInvariant()) {
        throw "SHA-256 mismatch for $Path"
    }
}

$manifestPath = Join-Path $PSScriptRoot "windows-gdal-lock.json"
$manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json

if ($manifest.piattaforma -ne "windows-x86_64" -or $manifest.subdir -ne "win-64") {
    throw "Il lock non e' quello di Windows: $($manifest.piattaforma)/$($manifest.subdir)"
}
# I binding devono essere della serie della libreria spedita. E' la pretesa che
# la forzatura precedente aggirava, ed e' scritta qui perche' valga anche a chi
# esegue lo script a mano.
$serieLibreria = ($manifest.gdal_version -split '\.')[0..1] -join '.'
$serieBinding = ($manifest.binding_version -split '\.')[0..1] -join '.'
if ($serieLibreria -ne $serieBinding) {
    throw "Binding $($manifest.binding_version) contro libreria $($manifest.gdal_version): serie diverse"
}

if (-not [Environment]::Is64BitOperatingSystem) {
    throw "Il runtime fissato richiede Windows a 64 bit"
}

$destinationPath = New-Item -ItemType Directory -Force -Path $Destination | Select-Object -ExpandProperty FullName
$cachePath = New-Item -ItemType Directory -Force -Path $CacheDirectory | Select-Object -ExpandProperty FullName

# --- micromamba, fissato come tutto il resto -------------------------------
#
# Lo strumento che materializza fa parte di cio' che va fissato: uno strumento
# che cambia da solo rende non riproducibile cio' che produce, e cio' che
# produce e' esattamente l'albero che poi si spedisce.
$pin = $manifest.risolto_con
$micromambaArchive = Join-Path $cachePath "micromamba.tar.bz2"
if (-not (Test-Path -LiteralPath $micromambaArchive)) {
    Invoke-WebRequest -Uri $pin.url -OutFile $micromambaArchive -UseBasicParsing
}
Assert-Archive -Path $micromambaArchive -ExpectedSize $pin.dimensione -ExpectedSha256 $pin.sha256
$micromambaRoot = Join-Path $cachePath "micromamba"
New-Item -ItemType Directory -Force -Path $micromambaRoot | Out-Null
& tar -xf $micromambaArchive -C $micromambaRoot
if ($LASTEXITCODE -ne 0) { throw "Estrazione di micromamba fallita" }
$micromamba = Join-Path $micromambaRoot "Library\bin\micromamba.exe"
if (-not (Test-Path -LiteralPath $micromamba)) {
    $micromamba = Join-Path $micromambaRoot "bin\micromamba.exe"
}
if (-not (Test-Path -LiteralPath $micromamba)) {
    throw "micromamba non trovato dopo l'estrazione"
}

# --- i pacchetti, verificati prima che micromamba li veda ------------------
#
# Che poi controlli anche lui e' una seconda rete, non la prima: la verifica
# sullo sha256 resta nostra, e avviene prima di consegnargli il file.
$explicit = Join-Path $cachePath "explicit.txt"
$righe = New-Object System.Collections.Generic.List[string]
$righe.Add("@EXPLICIT")
foreach ($pacchetto in $manifest.pacchetti) {
    $file = Join-Path $cachePath ([System.IO.Path]::GetFileName(([uri]$pacchetto.url).LocalPath))
    if (-not (Test-Path -LiteralPath $file)) {
        Invoke-WebRequest -Uri $pacchetto.url -OutFile $file -UseBasicParsing
    }
    Assert-Archive -Path $file -ExpectedSize $pacchetto.dimensione -ExpectedSha256 $pacchetto.sha256
    $righe.Add("file:///$($file -replace '\\', '/')#$($pacchetto.sha256)")
}
Set-Content -LiteralPath $explicit -Value $righe -Encoding utf8

# In modalita' esplicita non c'e' solver, non c'e' interrogazione del canale e
# non c'e' metadata mobile -- e in cambio si hanno le rilocazioni di conda,
# l'ordine di link e la gestione delle collisioni.
& $micromamba create --yes --prefix $destinationPath --offline --file $explicit
if ($LASTEXITCODE -ne 0) { throw "Materializzazione fallita" }

foreach ($relativePath in @("Library\bin\gdal.dll", "Library\share\gdal", "Library\share\proj")) {
    $requiredPath = Join-Path $destinationPath $relativePath
    if (-not (Test-Path -LiteralPath $requiredPath)) {
        throw "Il runtime fissato e' incompleto: $requiredPath"
    }
}

$libraryRoot = Join-Path $destinationPath "Library"
$binPath = Join-Path $libraryRoot "bin"
$env:PATH = "$binPath;$env:PATH"
$env:GDAL_HOME = $libraryRoot
# La versione dichiarata a `gdal-sys` e' quella **spedita**. Non e' piu' una
# scelta indipendente, ed e' il punto di tutta questa riscrittura.
$env:GDAL_VERSION = [string]$manifest.gdal_version
$env:PLENORA_GDAL_RUNTIME_VERSION = [string]$manifest.gdal_version
$env:GDAL_DATA = Join-Path $libraryRoot "share\gdal"
$env:PROJ_DATA = Join-Path $libraryRoot "share\proj"

# La capability non segue dalla versione: la si interroga.
$reportedVersion = (& (Join-Path $binPath "gdalinfo.exe") --version 2>&1 | Out-String).Trim()
$atteso = "^GDAL " + [regex]::Escape($manifest.gdal_version) + ","
if ($LASTEXITCODE -ne 0 -or $reportedVersion -notmatch $atteso) {
    throw "Runtime GDAL inatteso: $reportedVersion"
}
$formats = (& (Join-Path $binPath "ogrinfo.exe") --formats 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0 -or $formats -notmatch "(?m)^\s+OpenFileGDB .*\(rw\+v\):") {
    throw "Il runtime fissato non espone OpenFileGDB in scrittura"
}

if ($ExportGitHubEnvironment) {
    if (-not $env:GITHUB_ENV -or -not $env:GITHUB_PATH) {
        throw "Esportazione dell'ambiente GitHub richiesta fuori da GitHub Actions"
    }
    @(
        "GDAL_HOME=$libraryRoot",
        "GDAL_VERSION=$($manifest.gdal_version)",
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
    package_count = @($manifest.pacchetti).Count
}
