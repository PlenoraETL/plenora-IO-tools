# Matrice di tracciabilità assurance

Stato: `Soddisfatto`, `Parziale`, `Aperto`. La matrice descrive evidenza nel
repository, non compliance DO-178C.

| ID | Hazard controllato | Requisito | Evidenza | Stato |
|---|---|---|---|---|
| PLN-ASR-001 | H-04 arresto del processo | Nessun `unsafe` nel codice distribuibile | `#![forbid(unsafe_code)]`; gate CI `-D unsafe-code` sui target `lib` | Soddisfatto |
| PLN-ASR-002 | H-04 arresto del processo | Nessuna primitiva esplicita di panic nei target `lib`; una terminazione anomala dei parser in background non è un falso EOF | Gate Clippy `unwrap/expect/panic/unreachable/todo/unimplemented`; `spawn_batch_reader` intercetta panic/disconnessione e restituisce `PlenoraIoError` | Soddisfatto |
| PLN-ASR-003 | H-01 corruzione geometrica | WKB/EWKB troncati, dimensionalmente incoerenti o oltre limite sono rifiutati | Unico codec autoritativo `plenora-io-model::wkb_lossless`, esposto da `wkb`; visitor strutturale senza AST confrontato dal fuzz con il decoder; corpus cross-repository deterministico di 18 payload con hash, limiti, trailing byte, Z/M, SRID e conteggi avversari | Soddisfatto nel perimetro dei sette tipi WKB semplici |
| PLN-ASR-004 | H-03 esaurimento risorse | Ogni operazione applica limiti caller-controlled prima di crescita non bounded | `Limits`, `WkbLimits`, wrapper reader/writer e test limite; dimensionatore adattivo condiviso sui byte Arrow osservati in CSV/GeoJSON/SHP/FileGDB/GPKG; writer DXF limitato durante la serializzazione; benchmark A/B della campagna | Parziale |
| PLN-ASR-005 | H-02 sovrascrittura output | Publish atomico, same-filesystem e no-clobber autorevole | ADR-IO 2; test cross-filesystem, TOCTOU, crash e recovery; job macOS su `renameatx_np(RENAME_EXCL)` | Parziale: soddisfatto Linux/Android, Windows e macOS; directory publish non disponibile sui BSD |
| PLN-ASR-006 | H-05 falsa conferma di durabilità | La durabilità non verificabile è riportata nell'esito | `PublishOutcome`; Windows restituisce `PublishedButDurabilityUnconfirmed` | Parziale: matrice filesystem aperta |
| PLN-ASR-007 | H-01 perdita dati silenziosa | Conversioni e coercion sono fail-closed o rendicontate | ADR-IO 3 e 5; capability gate con set dei tipi geometrici; `FidelityAssessment` e `LossReport`; errori dei worker preservati per variante; parser dei metadati geometrici chiuso; default legacy applicato prima del parsing e distinzione assente/`unknown` conforme alla direzione R3.4; nomi wire conformi a R3.1; geometrie GeoJSON vuote rifiutate prima dell'emissione | Parziale per driver |
| PLN-ASR-008 | H-06 interpretazione CRS errata | CRS assente, irrisolto e axis order non sono confusi | ADR-IO 4; `CrsResolution`, `RawCrs`, test axis order; chiavi schema `crs_id`, `crs_resolution`, `crs_definition`, `crs_definition_format`, `axis_order`; SHP/GPKG/DXF fail-closed su resolved senza ID | Parziale per resolver |
| PLN-ASR-009 | H-01/H-02 stato parziale | Lifecycle writer invalida dopo errore e pubblica soltanto a `finish` | ADR-IO 1; `StagedFile` terminale centralizza staging/limite/publish dei file singoli; test su doppio publish, limite, drop, poison, abort, concorrenza e crash FileGDB | Soddisfatto nel profilo corrente |
| PLN-ASR-010 | H-07 baseline non riproducibile | Toolchain e grafo dipendenze sono fissati e sottoposti ad audit | `rust-toolchain.toml` 1.92.0; dipendenze dirette con pin esatti e punto unico; GitHub Actions Node 24 fissate a commit SHA e CIA del rinnovo; gate manifest/workflow; `Cargo.lock`, `--locked`, `cargo audit --deny warnings`; eccezioni motivate in `DEPENDENCY_EXCEPTIONS.md` | Parziale: immagini runner e pacchetti nativi installati da `apt` restano mobili |
| PLN-ASR-011 | H-08 verifica insufficiente | Regressioni hanno test deterministici e coverage misurata | CI candidata `0dbd4fe` verde su Linux/Windows/macOS; artifact LCOV SHA-256 registrato; filtro librerie 12.675/15.175 linee (83,53%); FileGDB feature-on, fuzz e test worker | Parziale: branch coverage e MC/DC assenti |
| PLN-ASR-012 | H-09 modifica non analizzata | Ogni modifica registra impatto ed evidenza | Template PR, questa matrice e change record in `docs/assurance/CHANGE_IMPACT_*.md` | Parziale: revisione indipendente non disponibile |
| PLN-ASR-013 | H-01 valore inventato | Ogni fallback semantico non-panicking è censito e non può crescere senza review | `FALLBACK_REGISTER.md`; gate a 88 occorrenze workspace, di cui 41 nel componente distribuibile, tramite `check_assurance_fallbacks.sh`; regressioni CSV/CRS/FileGDB, interi, GeoJSON/KML vuoti e XLSX XY | Parziale: registro semantico, non prova formale |
| PLN-ASR-014 | H-01/H-07 identità di confine ambigua | Package e tipi pubblici locali non collidono con componenti Plenora distinti | package `plenora-io-model`; errore `PlenoraIoError`; gate `check_public_identity.py`; confronto con `plenora-data-tools` | Soddisfatto nel perimetro corrente; crate condiviso subordinato alla ratifica di §15.3 |
| PLN-ASR-015 | H-01/H-06 contratto di scambio incompleto | Versione, identità campo, stato dei tipi e CRS nativo attraversano il bordo IO senza perdita silenziosa | `plenora.contract.version`; `plenora.field_id`; `types_declaration`; cinque chiavi CRS; parser fail-closed; golden test model; harness reale IO → data → database su Point XYZ | Parziale: una sola direzione e una sola fixture della matrice di sistema |
| PLN-ASR-016 | H-02/H-03 operazione lunga non interrompibile | Ogni operazione espone cancellazione e deadline ai confini controllati e non pubblica dopo cancellazione | `CancellationToken` in opzioni/richieste; controlli probe/batch/write/finalize; KML/DXF/XLSX ricontrollano dopo la dipendenza e ogni 1.024 elementi nei loop propri; test rilascio lease, intervallo bounded e pre-publish; benchmark pre/post | Parziale: la singola chiamata sincrona interna ai parser materializzanti non è preemptive |
| PLN-ASR-017 | H-01/H-09 errore ambiguo | Categoria, fase, effetto remoto e retry sono indipendenti e serializzabili | `PlenoraIoError`; test combinazioni timeout/commit/recovery; migrazione workspace e FileGDB all-features | Soddisfatto nel modello locale; convergenza cross-repo da verificare |
| PLN-ASR-018 | H-07/H-09 dichiarazione di release ambigua | Una RC identifica ICD, revisione, stato normativo, deroghe e perimetro componente/sistema senza promozioni implicite | `release/contract-provenance.json`; `RELEASE_CANDIDATE_SCOPE.md`; gate CI `check_release_contract.py` | Parziale: freeze e revisione indipendente non ancora eseguiti |
| PLN-ASR-019 | H-01/H-08 codec divergenti non rilevati | WKB/EWKB usa corpus e invarianti condivisi e classifica ogni divergenza fra i due bordi | Replay incrociato di 18 casi: pass, zero divergenze non classificate e due differenze motivate; smoke seed `20260728`, 68.740.000 mutazioni/60 s, zero finding; corpus rigenerabile verificato in CI | Parziale: campagna lunga coverage-guided, retention dei finding e promozione del corpus nel repository condiviso ancora aperte |
| PLN-ASR-020 | H-01/H-07 risultato non riproducibile | Ogni operazione dichiara il livello di determinismo e il catalogo non dipende dall'ordine di registrazione | `DeterminismLevel`; catalogo ordinato; doppio round-trip pure-Rust; doppia scrittura FileGDB confrontata semanticamente su Linux x86_64/GDAL 3.10.3 | Parziale: matrice multi-GDAL e FileGDB nativo Windows non verificati |
| PLN-ASR-021 | H-01/H-06/H-08 proprietà perse nella catena | Il contratto deve attraversare componenti reali, non soli test isolati | Harness compilato contro IO-tools, data-tools e database-tools: IPC reale, trasformazione DAG e scanner EWKB; Point XYZ/EPSG:4326/field_id, 3→2 righe, pass | Parziale: direzione inversa, sette fixture residue e Windows aperti |

## Hazard

| ID | Descrizione |
|---|---|
| H-01 | Corruzione, perdita o reinterpretazione silenziosa dei dati |
| H-02 | Sovrascrittura o pubblicazione parziale di un dataset |
| H-03 | Esaurimento non controllato di memoria, CPU o storage |
| H-04 | Panic, comportamento indefinito o arresto del processo |
| H-05 | Dichiarazione di durabilità non realmente confermata |
| H-06 | CRS o ordine assi interpretato in modo errato |
| H-07 | Artefatto non riproducibile o dipendenza non controllata |
| H-08 | Difetto non rilevato dalla strategia di verifica |
| H-09 | Regressione introdotta senza change impact analysis |
