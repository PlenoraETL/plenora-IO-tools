# Change impact analysis — baseline e metadati geometrici

Data: 2026-07-27
Stato evidenza: verifica interna; revisione indipendente non eseguita.

## Identificazione della modifica

Questa CIA copre l’incremento che:

- distingue un metadato dimensionale assente da `unknown` esplicito;
- rifiuta i metadati geometrici namespaced presenti ma malformati;
- fissa Rust `1.92.0` e tutte le dipendenze dirette a versioni esatte;
- introduce un gate che impedisce nuovi requisiti caret o dipendenze Git
  mobili;
- sostituisce la copia locale dell’ICD con un riferimento alla fonte
  autorevole.
- chiude il panic della dipendenza `kml 0.14.0` su un `Point` senza coordinate,
  emerso durante la campagna fuzz di verifica, mediante pre-validazione bounded.

Riferimento contrattuale committato: `plenora-contracts` tag annotato
`v2.0-rc1`, commit `d9547b7ca9d1cb4172c95e61ae9a6e79df874b76`. È stata
ispezionata anche la working copy locale `2.0-rc2`, ma il suo stato non
committato non è usato come baseline.

## Requisiti e hazard interessati

| Requisito | Hazard | Impatto |
|---|---|---|
| PLN-ASR-007; ADR-IO 3 e 5 | H-01 | `unknown` esplicito non viene più convertito in XY; valori metadata ignoti, SRID non `i32`, tipi ignoti o duplicati falliscono con `PlenoraError::Contract` |
| PLN-ASR-002 | H-04 | i `Point` KML vuoti sono rifiutati prima del parser dipendente che usa una rimozione indicizzata panicking |
| PLN-ASR-010; ICD R13.1–R13.5 | H-07 | toolchain e requisiti di versione diretti sono immutabili; ogni futuro scostamento fallisce in CI |
| PLN-ASR-012 | H-09 | la modifica a dipendenze, toolchain e contratto è accompagnata da questa CIA |
| ICD §15.1 | H-07/H-09 | eliminata la seconda copia dell’ICD; resta un solo repository autorevole |

Le API condivise delle sezioni ancora `proposta` non sono introdotte.

## Contratti e failure mode

- Assenza di `plenora.geometry.dimensions`: il percorso legacy mantiene il
  default documentato WKB XY.
- Valore esplicito `unknown`: resta `CoordinateDimensions::Unknown`; nessuna
  informazione viene inventata.
- Metadato namespaced presente ma non riconosciuto: errore tipizzato prima che
  il writer venga restituito o il dataset IPC venga esposto.
- Il parsing è transazionale: in caso di errore il contratto passato dal
  chiamante non resta parzialmente aggiornato.
- Più campi GeoArrow senza un contratto geometrico esplicito: errore di
  contratto anziché selezione del primo o disattivazione silenziosa della
  validazione.
- Tipi geometrici duplicati nel contratto statico: rifiutati dal capability
  gate, così un writer IPC non può emettere metadata che il proprio reader
  respingerebbe.

La firma pubblica di `read_geometry_contract_metadata` ora restituisce
`Result<GeometryMetadataPresence>` e `with_write_validation` restituisce
`Result<Box<dyn FormatWriter>>`. Tutti i call site del workspace sono migrati.
I crate hanno versione `0.0.0` e `publish = false`; non esiste una release
pubblica cui garantire compatibilità binaria o semantica.

## Dipendenze e toolchain

I pin sono quelli già risolti nei lockfile:

| Dipendenza | Versione |
|---|---:|
| `geo-types` | 0.7.19 |
| `shapefile` | 0.6.0 |
| `rust_xlsxwriter` | 0.79.4 |
| `gdal` | 0.17.1 |
| `serde` | 1.0.229 |
| `serde_json` | 1.0.151 |
| `tempfile` | 3.27.0 |
| `libc` | 0.2.189 |
| `libfuzzer-sys` (workspace fuzz separato) | 0.4.13 |

`sha2`, dichiarata ma non usata, è rimossa. `libc` è spostata nel punto unico
`[workspace.dependencies]`. Il lockfile fuzz, che era rimasto incoerente con i
manifest del workspace principale, è riallineato: `kml` passa da 0.8.7 a 0.14.0
e `quick-xml` da 0.37.5 a 0.41.0; vengono incluse le dipendenze target-specific
del publish (`rustix` e `atomicwrites`). Il workspace principale non cambia
versioni risolte.

Il file `rust-toolchain.toml` fissa Rust `1.92.0`, profilo `minimal` e i
componenti usati dalla verifica (`rustfmt`, `clippy`, `llvm-tools-preview`).

## Piattaforme e verifica

Piattaforme interessate: tutte; il parser metadata e i manifest non hanno rami
specifici di sistema operativo. La verifica richiesta comprende:

- test unitari positivi, negativi e di non-mutazione del parser metadata;
- regressione KML sul finding
  `crash-8256b8deaf88c49bddab5efffa5d1c8e5ee33415` e nuova esecuzione
  coverage-guided del target;
- suite workspace e Clippy safety gate su Linux;
- suite workspace su Windows;
- test publish e Clippy su macOS;
- build release, audit dipendenze e coverage;
- risoluzione `--locked` del workspace principale e di `fuzz/`.

Verifica locale eseguita in container Linux x86_64 con Rust `1.92.0`:

- `cargo fmt --all -- --check`: superato;
- gate dipendenze: superato su 18 manifest;
- `cargo metadata --locked`: superato sia per il workspace principale sia per
  `fuzz/`;
- test workspace completi: superati;
- Clippy completo e Clippy safety gate: superati con warning negati;
- build release del workspace: superata;
- suite FileGDB/GDAL: 21 test superati, 2 helper ignorati;
- riproduzione del corpus KML originale dopo il fix: nessun panic;
- nuova campagna KML: 196.332 esecuzioni in 31 secondi, nessun crash o timeout.

Restano demandati alla CI associata al commit di baseline: Windows, macOS,
coverage e `cargo audit`, non disponibile nell'immagine di verifica locale.

## Residui e limiti della dichiarazione

- Le GitHub Actions restano riferite a major tag e non a commit SHA:
  PLN-ASR-010 resta `Parziale`.
- Le dipendenze transitive e native non sono qualificate singolarmente.
- Il toolchain, Clippy, fuzzing e coverage non sono strumenti qualificati
  DO-330.
- Non sono disponibili revisione indipendente, MC/DC, object-code verification,
  WCET o una matrice hardware/filesystem completa.
- La working copy `2.0-rc2` di `plenora-contracts` deve essere committata,
  revisionata e taggata prima di poter diventare una baseline citabile.
- Le API comuni di cancellazione, budget, errore e capability attendono la
  ratifica delle rispettive sezioni dell’ICD.

Questa evidenza rafforza una libreria destinabile a un’integrazione avionica;
non costituisce certificazione né dichiarazione di conformità DO-178C.
