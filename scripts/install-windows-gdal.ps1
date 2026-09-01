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
# # Che cosa la prima esecuzione ha trovato
#
# Fino alla prima corsa questo script era una dichiarazione, non una verifica.
# La prima corsa su `windows-2022` si e' fermata proprio qui: quarantacinque
# minuti sul passo di materializzazione, senza mai arrivare alla costruzione.
#
# La causa era la barra di avanzamento di `Invoke-WebRequest`, che ridisegna la
# console a ogni blocco: su decine di pacchetti il tempo passa da minuti a ore.
# Ora si preferisce `curl.exe` -- presente su Windows dal 2018 e su ogni runner
# -- e la barra e' spenta comunque, perche' `Invoke-WebRequest` resta il
# ripiego. E il passo dice a che punto e', perche' un log che tace per minuti
# non distingue il lavoro dal blocco.

[CmdletBinding()]
param(
    [string]$Destination = (Join-Path (Get-Location) "target\windows-gdal"),
    [string]$CacheDirectory = (Join-Path ([System.IO.Path]::GetTempPath()) "plenora-conda-cache"),
    [switch]$ExportGitHubEnvironment
)

$ErrorActionPreference = "Stop"

# Un diario su file, oltre alla console.
#
# L'output di un passo **in corso** si perde quando il job viene cancellato o
# scade: le prime corse di scoperta si sono fermate qui e il log non conteneva
# una sola riga di questo script. Un file lo si puo' caricare come artefatto
# anche quando il passo non e' arrivato in fondo.
$diarioPath = Join-Path ([System.IO.Path]::GetTempPath()) "install-windows-gdal.log"
function Diario {
    param([Parameter(Mandatory = $true)][string]$Testo)
    $riga = "[{0:HH:mm:ss}] {1}" -f (Get-Date), $Testo
    Write-Host $riga
    Add-Content -LiteralPath $diarioPath -Value $riga -Encoding utf8
}
Set-Content -LiteralPath $diarioPath -Value "" -Encoding utf8
Diario "avvio su $([System.Environment]::OSVersion.VersionString)"

# La barra di avanzamento di `Invoke-WebRequest` costa piu' del trasferimento.
#
# Non e' un dettaglio di stile: con la barra attiva IWR ridisegna la console a
# ogni blocco, e su decine di pacchetti da qualche decina di megabyte il tempo
# passa da minuti a ore. La prima corsa di scoperta si e' fermata proprio qui,
# dopo quarantacinque minuti sul passo di materializzazione -- ed e' il genere
# di cosa che si scopre soltanto eseguendo.
$ProgressPreference = "SilentlyContinue"

function Get-File {
    <#
        Scarica un file, preferendo `curl.exe` quando c'e'.

        `curl.exe` e' presente su Windows dal 2018 e su ogni runner
        `windows-2022`. E' molto piu' veloce di `Invoke-WebRequest` su file
        grandi, e non ha la console di mezzo. `Invoke-WebRequest` resta come
        ripiego, perche' un ambiente senza `curl.exe` deve poter comunque
        materializzare il runtime -- lentamente, ma deve poterlo fare.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$OutFile
    )
    $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
    if ($curl) {
        & $curl.Source --silent --show-error --fail --location --output $OutFile $Uri
        if ($LASTEXITCODE -ne 0) { throw "Scaricamento fallito ($Uri): curl.exe $LASTEXITCODE" }
    }
    else {
        Invoke-WebRequest -Uri $Uri -OutFile $OutFile -UseBasicParsing
    }
}

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
Diario "micromamba dal lock: $($pin.url)"
$micromambaArchive = Join-Path $cachePath "micromamba.tar.bz2"
if (-not (Test-Path -LiteralPath $micromambaArchive)) {
    Get-File -Uri $pin.url -OutFile $micromambaArchive
}
Diario "micromamba scaricato, verifico"
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
    $trovati = (Get-ChildItem -Recurse -LiteralPath $micromambaRoot | Select-Object -First 20 -ExpandProperty FullName) -join "; "
    Diario "micromamba non trovato. Estratto: $trovati"
    throw "micromamba non trovato dopo l'estrazione. Estratto: $trovati"
}
Diario "micromamba: $micromamba"

# --- i pacchetti, verificati prima che micromamba li veda ------------------
#
# Che poi controlli anche lui e' una seconda rete, non la prima: la verifica
# sullo sha256 resta nostra, e avviene prima di consegnargli il file.
$explicit = Join-Path $cachePath "explicit.txt"
$righe = New-Object System.Collections.Generic.List[string]
$righe.Add("@EXPLICIT")
$quanti = @($manifest.pacchetti).Count
$n = 0
# Un passo che tace per minuti non dice se stia lavorando o se sia fermo, e la
# differenza conta quando lo si guarda da un log.
foreach ($pacchetto in $manifest.pacchetti) {
    $n += 1
    Diario "[$n/$quanti] $($pacchetto.nome) $($pacchetto.versione)"
    $file = Join-Path $cachePath ([System.IO.Path]::GetFileName(([uri]$pacchetto.url).LocalPath))
    if (-not (Test-Path -LiteralPath $file)) {
        Get-File -Uri $pacchetto.url -OutFile $file
    }
    Assert-Archive -Path $file -ExpectedSize $pacchetto.dimensione -ExpectedSha256 $pacchetto.sha256
    $righe.Add("file:///$($file -replace '\\', '/')#$($pacchetto.sha256)")
}
Set-Content -LiteralPath $explicit -Value $righe -Encoding utf8

# `MAMBA_ROOT_PREFIX` va impostata, e non e' una comodita'.
#
# Senza, micromamba non sa dove tenere la propria radice e **chiede**: su un
# runner senza console interattiva quella domanda non riceve mai risposta, e il
# passo resta appeso finche' il job non scade. E' cio' che ha bloccato la
# seconda corsa di scoperta -- quindici minuti su un comando che non stava
# lavorando. L'installatore Linux la impostava gia'; questo no, ed era l'unica
# differenza fra i due.
$env:MAMBA_ROOT_PREFIX = Join-Path $cachePath "root"
New-Item -ItemType Directory -Force -Path $env:MAMBA_ROOT_PREFIX | Out-Null

# In modalita' esplicita non c'e' solver, non c'e' interrogazione del canale e
# non c'e' metadata mobile -- e in cambio si hanno le rilocazioni di conda,
# l'ordine di link e la gestione delle collisioni.
Diario "materializzazione di $quanti pacchetti in $destinationPath"
& $micromamba create --yes --prefix $destinationPath --offline --file $explicit
if ($LASTEXITCODE -ne 0) { Diario "materializzazione fallita: $LASTEXITCODE"; throw "Materializzazione fallita" }
Diario "materializzazione conclusa"

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
