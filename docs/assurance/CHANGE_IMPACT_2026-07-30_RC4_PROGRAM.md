# Change impact analysis — avvio del programma component RC 0.1.0-rc.4

Data: 2026-07-30.

## Baseline

RC4 parte dal tag annotato e immutabile `v0.1.0-rc.3`, target
`ea0de79677e8fc794d96ac3d95c5bc2c6e30358c`. La baseline di implementazione
contenuta nella release è
`3f3562a4707995549ff5eb8dc03f9e37f2cde355`.

Il workspace passa a `0.1.0-rc.4` come versione di sviluppo. Questo avvio non
crea una nuova RC e mantiene `component_rc`, `system_rc` e
`avionic_certification` a `false`.

## Programma

I tre gruppi differiti da RC3 restano indipendenti:

1. ridurre la materializzazione dei reader KML, DXF e XLSX senza inventare
   schema o semantica e senza superare i veti prestazionali;
2. applicare projection fisica OpenFileGDB soltanto attraverso una API safe o
   un fork governato, senza introdurre `unsafe` nel componente;
3. ottenere un ambiente GDAL/OpenFileGDB Windows riproducibile e rieseguire le
   matrici native, crash/recovery e filesystem senza peggiorare il percorso
   narrow oltre il 5%.

## Primo incremento: XLSX bounded con spool temporaneo

`calamine 0.36.1`, già pinnato, espone `Xlsx::worksheet_cells_reader`, un
reader lazy delle celle usate con coordinate esplicite. Il primo prototipo
riapriva il workbook per una seconda passata: riduceva l'RSS, ma il benchmark
interlacciato ha misurato una regressione mediana del throughput del 37,71% e
ha attivato il veto del 5%. Quel percorso è stato rimosso.

L'implementazione mantenuta usa l'API lazy senza cambiare il contratto:

- una sola scansione XLSX durante `open` ricava intestazione, inferenza monotona
  dei tipi, geometria, dimensioni e tipi WKB;
- i valori già normalizzati vengono scritti in uno spool temporaneo binario
  bounded da `limits.max_input_bytes`, mai trattenuti come dataset Arrow in
  memoria;
- il `LayerReader` legge sequenzialmente lo spool e produce `RecordBatch`
  bounded da `batch_target`, con canale a capacità limitata;
- righe e celle sparse vengono ricostruite senza cambiare l'ordine o inventare
  valori;
- cancellazione verificata durante scansione ed emissione;
- errori del parser e valori incompatibili restano fail-closed.

Lo spool è deliberato: una singola passata che emettesse Arrow prima della fine
dell'inferenza sceglierebbe i tipi senza avere osservato tutto il foglio.
L'artefatto temporaneo viene eliminato al rilascio del dataset; se la sua
espansione supera il limite di input, l'apertura fallisce esplicitamente con
`LimitExceeded`.

Il secondo benchmark interlacciato, cinque campioni per revisione sulla stessa
fixture da 100.000 righe e sullo stesso container, ha dato:

- throughput mediano: 442.733 → 543.076 righe/s (`+22,66%`);
- peak RSS mediano: 48.668.672 → 10.014.720 byte (`-79,42%`);
- allocazioni per campione: 1.108.260 → 708.423 (`-36,08%`);
- byte allocati: 108.748.567 → 33.941.119 (`-68,79%`).

Il veto del 5% è superato e il descrittore XLSX passa a
`ReadMode::StreamingSequential`.

## Secondo incremento: matrice GDAL/OpenFileGDB Windows riproducibile

La matrice Windows usa un ambiente OSGeo4W fissato e verificato prima
dell'estrazione. `scripts/windows-gdal-lock.json` registra URL ufficiale,
dimensione e SHA-256 di 49 archivi; il runtime risultante è GDAL 3.10.3 e
`ogrinfo --formats` deve dichiarare OpenFileGDB `rw+v`. L'installer rifiuta
dimensioni o digest discordanti, membri d'archivio non sicuri e ambienti
incompleti.

La dipendenza Rust resta `gdal 0.17.1`. Il runtime e le dichiarazioni FFI sono
registrati separatamente: le DLL sono GDAL 3.10.3, mentre `gdal-sys 0.10.0`
seleziona le binding precompilate 3.6 già usate dalla baseline RC3. Questo è
un vincolo esplicito, non una dichiarazione falsa sulla versione caricata:
`PLENORA_GDAL_RUNTIME_VERSION=3.10.3` descrive il runtime e
`GDAL_VERSION=3.6.0` seleziona le dichiarazioni ABI.

Un primo candidato generava binding 3.10 con `bindgen`. L'installazione e il
caricamento OpenFileGDB erano corretti, ma MSVC ha rilevato 58 incompatibilità
di tipo fra le binding generate e le assunzioni sorgente di `gdal 0.17.1`
(inclusi enum `i32`/`u32` e il puntatore dell'errore
`OSRGetSemiMajor`). Il run `30543014741` ha quindi respinto quel percorso. La
feature e l'intero grafo `bindgen` sono stati rimossi dal manifest e dal
lockfile.

Il run finale `30543947947`, sulla revisione
`4c9eddb0e4f02f1e3420896980dee0b4f092a752`, è verde sui quattro job. Il job
Windows ha ricostruito da zero i 49 pacchetti fissati, eseguito 22 test
FileGDB nativi con due soli helper marcati ignored, incluso crash/recovery, e
ha superato il test cross-volume reale e la suite workspace.

Il benchmark narrow costruisce sia il tag immutabile `v0.1.0-rc.3` sia la
revisione RC4 nello stesso job, con lo stesso runtime e lo stesso target
MSVC. Sette coppie interlacciate sulla fixture da 50.000 righe e 64 campi
hanno dato:

- baseline RC3: mediana 432,839 ms;
- candidato RC4: mediana 437,440 ms;
- delta: `+1,063%`, entro il veto del 5%.

L'artefatto `windows-filegdb-narrow-benchmark.json` ha SHA-256
`34829261a315b90f00119a1fb9dc41ec1d7975fce15a65f33ae54913d3e42e8a`.
Checksum, righe e configurazione sono inclusi nell'oracolo machine-readable.

## Terzo incremento: verifica Shapefile su dati patrimoniali reali

Un corpus deterministico di 3.000 particelle EPSG:3003 in Shapefile ha
individuato due difetti di osservabilità nel bordo di lettura.

Il riconoscimento del CRS verificava `GEOGCS[` prima di `PROJCS[`. Poiché un
WKT proiettato contiene il CRS geografico di base, ogni definizione WKT1
proiettata veniva classificata come geografica e perdeva l'ordine assi
`EastingNorthing`. La precedenza è ora `PROJCS`/`PROJCRS` prima di
`GEOGCS`/`GEOGCRS`. Un fixture `.prj` EPSG:3003 reale, che contiene il
`GEOGCS` annidato, verifica direttamente identificativo, `CrsKind::Projected`
e ordine assi.

I campi DBF `Numeric` vengono esposti dalla dipendenza come `f64`. Per valori
interi con valore assoluto almeno `2^53`, l'identità lessicale originaria non
è più verificabile: valori DBF distinti possono essere già collassati prima
che il driver li osservi. Il reader registra ora
`dbf_numeric_integer_precision_unverifiable` nel `LossReport`, una occorrenza
per valore osservato nella zona a rischio e un solo esempio bounded per campo.
La valutazione di fedeltà del dataset diventa quindi `Approximating` con
motivo `precision_changed`, invece di dichiarare un rapporto vuoto.

Il conteggio è intenzionalmente conservativo: dichiara le occorrenze per cui
la precisione intera unitaria non è dimostrabile, non ricostruisce né inventa
il numero di collisioni avvenute. Preservare il testo numerico DBF originale
richiederebbe un reader lessicale diverso; non è implicato da questa
correzione.

La verifica sul corpus reale produce `EPSG:3003`, `kind=projected`,
`axis_order=easting_northing` e 3.000 occorrenze dichiarate. Un test con due
record DBF lessicalmente distinti (`9007199254740992` e
`9007199254740993`) dimostra inoltre il collasso nello stesso `f64` e verifica
che il report sia disponibile sia nella valutazione del dataset sia nel
`LayerReader`. Il comportamento del driver cambia senza modificare lo schema
del descrittore; `driver_version` passa da 7 a 8 e `descriptor_version` resta
6.

## Quarto incremento: pushdown nativo OpenFileGDB

Il prerequisito RC3 ammetteva una API safe upstream oppure un fork governato.
Il workspace usa ora un fork locale della stessa crate `gdal 0.17.1`, ricavato
dalla sorgente crates.io con checksum
`82ab834e8be6b54fee3d0141fce5e776ad405add1f9d0da054281926e0d35a9f` e
revisione upstream
`a324724c60dbf1e9bf0fb05203c4e0a1eefbf312`. Provenienza, delta e regola di
aggiornamento sono registrati in `vendor/gdal/PLENORA_FORK.md`.

Il solo delta funzionale espone
`LayerAccess::set_ignored_fields(&[&str])` come wrapper safe e fallibile di
`OGR_L_SetIgnoredFields`. Le `CString` e il vettore di puntatori restano vivi
per l'intera chiamata, la lista è terminata da null e ogni errore OGR viene
restituito senza fallback. L'`unsafe` resta confinato nella crate FFI
governata; `driver-filegdb` continua a soddisfare `forbid(unsafe_code)` e i
gate safety del componente.

Il worker FileGDB valida ancora nome e tipo degli indici selezionati dopo aver
riaperto il dataset. Soltanto dopo la validazione calcola il complemento dei
campi richiesti, aggiunge `OGR_GEOMETRY` quando la geometria è esclusa e
installa la lista prima di leggere la prima feature. Projection vuota,
richiesta in ordine inverso, valori, null e geometria sono verificati su
OpenFileGDB reale.

Il benchmark A/B Linux usa due binari release distinti: baseline
`f9e098082be087881272c665c0a4768d93c906b2` e candidato con il solo delta
pushdown, GDAL 3.10.3, 50.000 righe, 64 attributi `Int32`, un warm-up e sette
coppie alternate:

| Percorso | Baseline mediana | Candidato mediana | Delta tempo |
|---|---:|---:|---:|
| narrow, 3 attributi non contigui | 86,171 ms | 53,311 ms | **−38,13%** |
| full, geometria + 64 attributi | 168,039 ms | 165,394 ms | **−1,57%** |

Ogni campione conserva 50.000 righe. I checksum sono `3754675000` sul narrow
e `80100250000` sul full per entrambe le varianti. Il candidato supera il veto
del 5% su entrambi i percorsi. Questa misura caratterizza throughput sul
runtime osservato, non WCET o schedulabilità real-time.

Il comportamento fisico cambia senza modificare la forma del descrittore:
`driver_version` passa da 9 a 10 e `descriptor_version` resta 8.

La CI `30550393598`, sul commit
`c87a5801794f7e95833bf20a41dc87d8a3848fbd`, è verde sui quattro job. Windows
ha verificato lo stesso fork con GDAL/OpenFileGDB 3.10.3 pinnato, inclusi suite
nativa, benchmark narrow col veto del 5%, cross-volume e workspace. Il gate
`Verify governed GDAL fork` ha inoltre verificato in CI i 64 file e il tree
hash `c35bc68c794bf7b5f27e948e8b139029dfe035c20ac540711ca0c4f5def5d099`.

## Workstream ancora bloccati

- KML richiede un parser event-based semanticamente equivalente e un benchmark
  interlacciato che superi il veto.
- DXF richiede un iteratore upstream pubblico oppure un fork governato.

I residui CRS combinati e l'osservabilità CLI del `LossReport` reader non
vengono assorbiti implicitamente in RC4: restano rispettivamente decisione
dell'owner ed attività esterna registrata.

## Gate

- rustfmt e Clippy safety;
- test workspace e test mirati XLSX;
- equivalenza di schema, payload, righe sparse e batch multipli;
- cancellazione durante la scansione e durante l'emissione;
- benchmark interlacciato baseline/RC4 su tempo, RSS e allocazioni superato
  prima della promozione a `ReadMode::StreamingSequential`;
- ambiente Windows verificato per digest e versione, suite FileGDB nativa,
  crash/recovery, cross-volume e benchmark narrow RC3/RC4 entro il 5%;
- fixture `.prj` proiettato con `GEOGCS` annidato e corpus DBF con interi oltre
  `2^53`, inclusa la propagazione bounded del `LossReport`;
- fork `gdal 0.17.1` verificato per provenienza, projection FileGDB vuota e
  non contigua, suite feature-on e benchmark A/B full/narrow;
- gate di provenienza release con RC3 immutabile e RC4 ancora `development`.
