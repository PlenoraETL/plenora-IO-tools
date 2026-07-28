# Change impact analysis — lifecycle file e modulo geometrico GeoJSON

Data: 2026-07-28

## Baseline e incrementi

Baseline precedente:
`1d69d80bbcc75d237ed6c34dd56fe51657b16717`.

Incrementi analizzati:

- `68566933f01659f7bf9925428752ed5450accde2` — lifecycle `StagedFile`
  condiviso e migrazione dei writer pure-Rust a file singolo;
- `3cbda13087cacd8e087ca408f7bd199474946239` — isolamento e
  semplificazione del codec geometrico GeoJSON.

Non cambiano dipendenze, manifest, lockfile, toolchain, descrittori, capability
o formati wire.

## Modifiche funzionali

### Lifecycle a file singolo

`StagedFile` incapsula in un unico stato:

- tempfile adiacente alla destinazione;
- destinazione;
- profilo durable;
- limite `max_output_bytes`;
- transizione terminale di publish.

CSV, GeoJSON, GeoParquet, Arrow IPC e GeoPackage non mantengono più copie
separate di questi campi e non ricostruiscono localmente la sequenza
`take`/limite/publish. GeoPackage usa `StagedFile::with_suffix` e continua a
chiudere SQLite prima del publish o della rimozione dello staging.

Il primo tentativo di publish consuma lo staging anche se fallisce: un writer
non può ritentare su uno stato potenzialmente ambiguo. Il drop prima di
`finish` rimuove lo staging e non rende visibile la destinazione.

### Geometria GeoJSON

Il codec GeoJSON ↔ AST WKB è separato dal parser di feature e dal writer
attributivo. Le API pubbliche usate dal fuzz harness
(`wkb_from_gj_value`, `write_geo_geojson`) restano nello stesso namespace
tramite re-export.

La conversione di `MultiLineString` e `MultiPolygon` costruisce direttamente
gli elementi WKB da slice prese in prestito. Sono rimossi i `clone` delle
coordinate e la creazione di varianti `geojson::Value` temporanee.

La validazione resta fail-closed: sono accettate soltanto posizioni finite XY o
XYZ, dimensionalità uniforme e collezioni non vuote. Durante la verifica una
prima implementazione locale con `then_some(ordinates[2])` è stata rilevata
dai round-trip XY perché l'argomento è valutato eager; la versione verificata
usa accesso opzionale e non indicizza la terza ordinata per XY.

## Compatibilità e failure mode

`StagedFile` è un'API additiva. I writer conservano la stessa semantica
osservabile: no-clobber, controllo dimensione prima della visibilità, esito di
durabilità tipizzato e abort senza output.

I byte WKB e il JSON emessi per input validi restano invariati. Non sono state
allargate né ristrette capability o semantiche di accettazione.

| Evento | Esito verificato |
|---|---|
| secondo publish sullo stesso `StagedFile` | errore `Contract` |
| limite output superato | errore `LimitExceeded`, destinazione assente, stato terminale |
| drop senza publish | staging rimosso, destinazione assente |
| GeoJSON XY/XYZ valido | round-trip invariato |
| quarta ordinata, non-finito, vuoto o dimensioni miste | errore fail-closed |

## Hazard e controlli

- H-01: il codec geometrico comune non inventa dimensioni o ordinate;
- H-02: staging, limite e publish sono una singola macchina a stato terminale;
- H-03: nessuna copia delle coordinate multipart durante la conversione;
- H-04: safety Clippy e regressioni XY impediscono l'indicizzazione panicking
  osservata durante lo sviluppo;
- H-08: test di lifecycle, round-trip GeoJSON e harness fuzz compilato;
- H-09: baseline, incrementi, impatto e prove sono collegati da questo record.

## Evidenza locale

Ambiente: Rust 1.92.0, Linux x86_64; per FileGDB immagine locale con GDAL 3.6.2
e target directory isolata.

- `cargo fmt --all -- --check`: superato;
- registro fallback: 82/82, invariato;
- Clippy workspace `--all-targets --all-features --locked -D warnings`:
  superato;
- safety Clippy workspace `--lib --all-features --locked`: superato con divieti
  `unsafe`, `unwrap`, `expect`, `panic`, `unreachable`, `todo`,
  `unimplemented`;
- test workspace `--all-targets --locked`: 172 superati;
- FileGDB `gdal-backend`: 21 superati, 2 helper ignorati ed eseguiti dai test
  di sottoprocesso;
- test reale cross-filesystem `/dev/shm`: superato;
- build workspace release `--locked`: superata;
- test mirati dei sei crate interessati dal lifecycle: 77 superati;
- test GeoJSON e compilazione test del fuzz harness: superati;
- Clippy mirato GeoJSON/fuzz e safety Clippy GeoJSON: superati.

## Residui

- `StagedFile` copre file singoli; directory dataset, loose set e recovery
  FileGDB mantengono lifecycle distinti perché hanno primitive e failure mode
  diversi;
- KML, DXF e XLSX restano materializzanti durante `open`;
- la cancellazione dei worker resta cooperativa;
- manca revisione indipendente e la matrice di durabilità su filesystem reali.

Questa evidenza non costituisce certificazione né dichiarazione di conformità
DO-178C/ED-12C.
