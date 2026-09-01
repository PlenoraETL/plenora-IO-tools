# Il relocation smoke Windows: l'artefatto funziona dove non e' stato costruito?
#
# # Le condizioni, e quale manca
#
# Le stesse di Linux, tranne una:
#
# 1. Si costruisce in A e si archivia.
# 2. **A viene cancellata.** Senza questo passo una DLL che si risolvesse
#    ancora la' resterebbe risolvibile, e non lo vedremmo.
# 3. Si estrae in una B con percorso di lunghezza diversa da A.
# 4. Si esegue da una terza directory, che non e' ne' A ne' B.
# 5. `PATH` ridotto a `System32`: il caricatore di Windows cerca accanto
#    all'eseguibile e poi nel `PATH`, e lasciarvi le directory del runner
#    vorrebbe dire permettergli di trovare altrove cio' che l'artefatto deve
#    portarsi dietro.
# 6. `GDAL_DATA`, `PROJ_DATA`, `PROJ_LIB` e `GDAL_DRIVER_PATH` preimpostate a
#    sentinelle inesistenti: se il binario le lasciasse stare, fallirebbe.
#    `PROJ_LIB` e' il nome storico di `PROJ_DATA` -- PROJ lo legge fino alla 9.0
#    -- e avvelenarlo invece di azzerarlo chiude una via che resterebbe aperta.
# 7. Si scrive e si rilegge un dataset -- un FileGDB con un CRS per il profilo
#    pieno, un GeoParquet per il base -- verificando schema e geometria.
#
# **La condizione 8 di Linux non c'e'.** Li' `strace` dice *quali* file il
# binario ha toccato, e da quello si dimostra che non ha guardato dentro A. Su
# Windows un equivalente non c'e' in CI: Process Monitor non e' installato e
# non gira senza interazione.
#
# Che cosa resta al posto suo: A **cancellata** e `PATH` ridotto. Se il binario
# cercasse una DLL in A non la troverebbe, e se la cercasse nel `PATH` del
# runner non ci sarebbe. E' una garanzia diversa -- si dimostra per assenza di
# alternative invece che per osservazione -- e vale la pena dirlo invece di
# lasciar credere che sia la stessa.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Archivio,
    [Parameter(Mandatory = $true)][string]$DirectoryA,
    [string]$Lavoro = (Join-Path ([System.IO.Path]::GetTempPath()) "relocation"),
    [string]$Referto = ""
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Archivio = (Resolve-Path -LiteralPath $Archivio).Path
$DirectoryA = (Resolve-Path -LiteralPath $DirectoryA).Path

if ($Archivio.StartsWith($DirectoryA, [StringComparison]::OrdinalIgnoreCase)) {
    throw "L'archivio sta dentro A, che sto per cancellare."
}

# B ha un percorso di lunghezza diversa da A: un percorso rilocato male puo'
# funzionare per caso quando le lunghezze coincidono.
$B = Join-Path $Lavoro "b-installazione-con-un-percorso-deliberatamente-piu-lungo"
$Terza = Join-Path $Lavoro "terza"
foreach ($d in @($B, $Terza)) {
    if (Test-Path -LiteralPath $d) { Remove-Item -Recurse -Force -LiteralPath $d }
    New-Item -ItemType Directory -Force -Path $d | Out-Null
}

Write-Host "== 1-2. A viene cancellata"
Remove-Item -Recurse -Force -LiteralPath $DirectoryA
if (Test-Path -LiteralPath $DirectoryA) { throw "A esiste ancora." }
Write-Host "   A cancellata: $DirectoryA"

Write-Host "== 3. estrazione in B"
& tar -xf $Archivio -C $B
if ($LASTEXITCODE -ne 0) { throw "Estrazione fallita" }
$radice = (Get-ChildItem -LiteralPath $B -Directory | Select-Object -First 1).FullName
$binario = Join-Path $radice "bin\plenora-io.exe"
if (-not (Test-Path -LiteralPath $binario)) { throw "$binario non c'e'" }
Write-Host "   $radice"
Write-Host "   lunghezza A=$($DirectoryA.Length)  B=$($radice.Length)"
if ($DirectoryA.Length -eq $radice.Length) {
    throw "A e B hanno la stessa lunghezza: un percorso rilocato male passerebbe per caso."
}

$manifesto = Get-Content -Raw -LiteralPath (Join-Path $radice "MANIFEST.json") | ConvertFrom-Json
$profilo = [string]$manifesto.profilo
Write-Host "   profilo: $profilo"

Write-Host "== 4-6. esecuzione dalla terza directory, con ambiente ostile"
Push-Location $Terza
try {
    $sentinella = "Z:\percorso\che\non\esiste\mai"
    # L'ambiente si riduce a cio' che serve per eseguire. `PATH` a System32
    # soltanto: il caricatore cerca accanto all'eseguibile e poi nel `PATH`, e
    # lasciarvi le directory del runner gli permetterebbe di trovare altrove
    # cio' che l'artefatto deve portarsi dietro.
    $ambiente = @{
        "PATH"             = "$env:SystemRoot\system32"
        "SystemRoot"       = $env:SystemRoot
        "TEMP"             = $Terza
        "TMP"              = $Terza
        "GDAL_DATA"        = "$sentinella\gdal"
        "PROJ_DATA"        = "$sentinella\proj"
        # Anche il nome storico: azzerarla invece che avvelenarla lascerebbe una
        # via aperta, e la prova sarebbe piu' debole di quel che dichiara.
        "PROJ_LIB"         = "$sentinella\proj-lib"
        "GDAL_DRIVER_PATH" = "$sentinella\plugins"
    }
    $precedenti = @{}
    foreach ($k in $ambiente.Keys) { $precedenti[$k] = [Environment]::GetEnvironmentVariable($k) }
    foreach ($chiave in @("GDAL_HOME", "CONDA_PREFIX")) {
        $precedenti[$chiave] = [Environment]::GetEnvironmentVariable($chiave)
        [Environment]::SetEnvironmentVariable($chiave, $null)
    }
    foreach ($k in $ambiente.Keys) { [Environment]::SetEnvironmentVariable($k, $ambiente[$k]) }

    # Diagnostica prima della prova: quando questa fallisce, il messaggio parla
    # di `proj.db` e non di cio' che manca davvero. Sapere che cosa c'e'
    # nell'albero e che cosa il processo vede costa quattro righe, e le prime due
    # corse le ho spese a indovinare.
    Write-Host "   albero: $(Get-ChildItem -LiteralPath $radice -Name | Sort-Object)"
    foreach ($atteso in @("share\gdal", "share\proj", "share\proj\proj.db")) {
        $presente = Test-Path -LiteralPath (Join-Path $radice $atteso)
        Write-Host "   $atteso : $presente"
    }
    Write-Host "   PROJ_DATA nell'ambiente: $([Environment]::GetEnvironmentVariable('PROJ_DATA'))"
    Write-Host "   PROJ_LIB  nell'ambiente: $([Environment]::GetEnvironmentVariable('PROJ_LIB'))"

    Write-Host "== 7. conversione e rilettura"
    $csv = Join-Path $Terza "sorgente.csv"
    Set-Content -LiteralPath $csv -Encoding utf8 -Value @(
        "codice,nome,geometry",
        "A-1,alfa,POINT(11.25 43.77)",
        "B-2,beta,POINT(12.49 41.90)"
    )
    $destinazione = if ($profilo -eq "filegdb") {
        Join-Path $Terza "uscita.gdb"
    } else {
        Join-Path $Terza "uscita.parquet"
    }

    $uscita = & $binario convert $csv $destinazione --in-opt wkt_column=geometry --assume-crs EPSG:4326 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "La conversione e' fallita: $uscita" }
    Write-Host "   scritto: $($uscita.Substring(0, [Math]::Min(160, $uscita.Length)))"

    $riletto = & $binario inspect $destinazione 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "La rilettura e' fallita: $riletto" }
    foreach ($atteso in @("nome", "geometry")) {
        if ($riletto -notmatch [regex]::Escape($atteso)) {
            throw "nel dataset riletto manca «$atteso»"
        }
    }
    if ($profilo -eq "filegdb" -and $riletto -notmatch "4326") {
        throw "il CRS non ha attraversato PROJ: EPSG:4326 non e' stato riletto"
    }
    Write-Host "   riletto: schema e geometria presenti$(if ($profilo -eq 'filegdb') { ', con CRS 4326' })"

    $librerie = @(Get-ChildItem -LiteralPath (Join-Path $radice "bin") -Filter *.dll).Count
    if ($Referto) {
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Referto) | Out-Null
        [pscustomobject]@{
            schema_referto = 2
            verifica       = "relocation"
            piattaforma    = [string]$manifesto.piattaforma
            profilo        = $profilo
            canale         = [string]$manifesto.canale
            esito          = "verde"
            misure         = [pscustomobject]@{
                librerie_dall_albero   = $librerie
                lunghezza_a            = $DirectoryA.Length
                lunghezza_b            = $radice.Length
                path_ridotto_a         = "System32"
            }
            errori         = @()
            note           = ("su Windows manca l'equivalente di `strace`: non si dimostra " +
                              "**quali** file il binario abbia toccato. Al suo posto valgono A " +
                              "cancellata e il PATH ridotto a System32 -- una garanzia per " +
                              "assenza di alternative invece che per osservazione.")
        } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Referto -Encoding utf8
    }

    foreach ($k in $precedenti.Keys) { [Environment]::SetEnvironmentVariable($k, $precedenti[$k]) }
}
finally {
    Pop-Location
}

Write-Host ""
Write-Host "RELOCATION SMOKE VERDE"
Write-Host "Dimostra che l'artefatto funziona da una directory che non e' quella in cui e'"
Write-Host "stato costruito, con A cancellata e senza poter trovare nel PATH cio' che deve"
Write-Host "portarsi dietro. Non dimostra quali file abbia toccato: su Windows non c'e' un"
Write-Host "modo di osservarlo in CI, e questa e' una garanzia diversa."
