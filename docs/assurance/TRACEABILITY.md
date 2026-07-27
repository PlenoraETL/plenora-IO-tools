# Matrice di tracciabilità assurance

Stato: `Soddisfatto`, `Parziale`, `Aperto`. La matrice descrive evidenza nel
repository, non compliance DO-178C.

| ID | Hazard controllato | Requisito | Evidenza | Stato |
|---|---|---|---|---|
| PLN-ASR-001 | H-04 arresto del processo | Nessun `unsafe` nel codice distribuibile | `#![forbid(unsafe_code)]`; gate CI `-D unsafe-code` sui target `lib` | Soddisfatto |
| PLN-ASR-002 | H-04 arresto del processo | Nessuna primitiva esplicita di panic nei target `lib`; una terminazione anomala dei parser in background non è un falso EOF | Gate Clippy `unwrap/expect/panic/unreachable/todo/unimplemented`; `spawn_batch_reader` intercetta panic/disconnessione e restituisce `PlenoraIoError` | Soddisfatto |
| PLN-ASR-003 | H-01 corruzione geometrica | WKB/EWKB troncati, dimensionalmente incoerenti o oltre limite sono rifiutati | `plenora-io-model::{wkb,wkb_lossless}`; test core e fuzz; regressione Shape Z/M mancanti | Soddisfatto |
| PLN-ASR-004 | H-03 esaurimento risorse | Ogni operazione applica limiti caller-controlled prima di crescita non bounded | `Limits`, `WkbLimits`, wrapper reader/writer e test limite | Parziale |
| PLN-ASR-005 | H-02 sovrascrittura output | Publish atomico, same-filesystem e no-clobber autorevole | ADR-IO 2; test cross-filesystem, TOCTOU, crash e recovery; job macOS su `renameatx_np(RENAME_EXCL)` | Parziale: soddisfatto Linux/Android, Windows e macOS; directory publish non disponibile sui BSD |
| PLN-ASR-006 | H-05 falsa conferma di durabilità | La durabilità non verificabile è riportata nell'esito | `PublishOutcome`; Windows restituisce `PublishedButDurabilityUnconfirmed` | Parziale: matrice filesystem aperta |
| PLN-ASR-007 | H-01 perdita dati silenziosa | Conversioni e coercion sono fail-closed o rendicontate | ADR-IO 3 e 5; capability gate; `FidelityAssessment` e `LossReport`; errori dei worker preservati per variante; parser dei metadati geometrici chiuso; default legacy applicato prima del parsing e distinzione assente/`unknown` conforme alla direzione R3.4; nomi wire conformi a R3.1 | Parziale per driver |
| PLN-ASR-008 | H-06 interpretazione CRS errata | CRS assente, irrisolto e axis order non sono confusi | ADR-IO 4; `CrsResolution`, `RawCrs`, test axis order; SHP/GPKG/DXF fail-closed su resolved senza ID | Parziale per resolver |
| PLN-ASR-009 | H-01/H-02 stato parziale | Lifecycle writer invalida dopo errore e pubblica soltanto a `finish` | ADR-IO 1; test poison, abort, concorrenza e crash FileGDB | Soddisfatto nel profilo corrente |
| PLN-ASR-010 | H-07 baseline non riproducibile | Toolchain e grafo dipendenze sono fissati e sottoposti ad audit | `rust-toolchain.toml` 1.92.0; dipendenze dirette con pin esatti e punto unico; GitHub Actions fissate a commit SHA; gate manifest/workflow; `Cargo.lock`, `--locked`, `cargo audit --deny warnings`; eccezioni motivate in `DEPENDENCY_EXCEPTIONS.md` | Parziale: immagini runner e pacchetti nativi installati da `apt` restano mobili |
| PLN-ASR-011 | H-08 verifica insufficiente | Regressioni hanno test deterministici e coverage misurata | CI Linux/Windows/macOS, FileGDB feature-on, LCOV e fuzz target; test worker su terminale esplicito, errore tipizzato e panic non-EOF | Parziale: MC/DC assente |
| PLN-ASR-012 | H-09 modifica non analizzata | Ogni modifica registra impatto ed evidenza | Template PR, questa matrice e change record in `docs/assurance/CHANGE_IMPACT_*.md` | Parziale: revisione indipendente non disponibile |
| PLN-ASR-013 | H-01 valore inventato | Ogni fallback semantico non-panicking è censito e non può crescere senza review | `FALLBACK_REGISTER.md`; gate `check_assurance_fallbacks.sh`; regressioni CSV/CRS/FileGDB/XLSX | Parziale: registro semantico, non prova formale |
| PLN-ASR-014 | H-01/H-07 identità di confine ambigua | Package e tipi pubblici locali non collidono con componenti Plenora distinti | package `plenora-io-model`; errore `PlenoraIoError`; gate `check_public_identity.py`; confronto con `plenora-data-tools` | Soddisfatto nel perimetro corrente; crate condiviso subordinato alla ratifica di §15.3 |

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
