# Roadmap `1.1.0` — release di correttezza

Bozza al 2026-08-15. Documento non normativo: rappresenta l'inquadramento
delle finding non applicabili alla candidate patch `1.0.1` e propone
lotti di lavoro, prerequisiti di ADR e criteri di uscita. Nessun impegno
di calendario. Nessuna modifica ai manifesti `release/*.json` in questa
fase.

Fonte: `docs/REVIEW-2026-08-15.md`.

## Perche' non e' una patch

La candidate `1.0.1` dichiara `functional_delta.contracts = "unchanged"`,
`error_semantics = "unchanged"`, `cleanup = "behavior_preserving"`
(vedi `release/1.0.1.json`) e vive sull'evidence base
`966005d67b6f2d4fcfe5d62e58fced17881eff06`. Le finding qui elencate
cambiano almeno uno dei tre attributi. Applicarle in patch richiederebbe:

- riqualificare l'evidence base;
- rifare la CI same-SHA di ogni commit metadata;
- rieseguire la catena PostgreSQL/MySQL Database → Data → IO;
- rieseguire la comparativa Plenora post-freeze delle tre librerie.

E' meno costoso e piu' onesto raccogliere il delta in una minor bump.

## Prerequisiti trasversali

### PR-1. ADR-IO 7 — streaming vs operation-atomicity

Blocca #2. Il contratto pubblico di `LayerReader::next_batch` non
dichiara oggi che il primo batch richiede lo scan completo della
sorgente. L'ADR deve stabilire:

- se l'atomicita' operativa (nessun prefisso accepted quando esiste una
  violazione anywhere nella sorgente) e' un invariante da conservare;
- se s`i, come si concilia con la promessa implicita di streaming
  (spool bounded, spill-to-disk, o hard cap dichiarato sul dataset);
- se no, come si dichiara nel `LossReport`/`FidelityAssessment` il
  cambio di semantica.

`docs/assurance/STREAMING_READER_DECISION.md` copre KML/DXF/XLSX ma non
l'adapter comune; il nuovo ADR va scritto separatamente.

### PR-2. Unificazione `Limits` / `ResourceLimits`

Blocca #3, semplifica #6, #9. Oggi convivono due sistemi di quote
(`plenora-io-model/src/limits.rs` e `plenora-io-model/src/resource.rs`)
con default divergenti. La CIA per il rimappaggio deve:

- definire una singola sorgente di verita' per righe/colonne/byte;
- separare quote cumulative (`max_rows`) da gauge di
  memoria/concorrenza (`ResourceBudget`);
- garantire che il default della CLI sia coerente con quanto dichiarato
  nel README.

### PR-3. Modello dichiarativo di `format_options`

Prerequisito soft per #11 (gia' applicato in `1.0.1` cleanup). Un
`FormatOptionsSchema` per driver con chiavi ammesse, tipi, default e
rifiuto delle sconosciute chiuderebbe una classe di finding future e
renderebbe il merge `--opt`/`--in-opt`/`--out-opt` verificabile
staticamente. Non blocca `1.1.0` ma va programmato.

## Lotti di lavoro

### L1 — Panic path IPC/Arrow (finding #1)

Sostituisce ogni `batch.column(index)` con `batch.columns().get(index)`
piu' errore tipizzato, e derivare o verificare `field_id` per uguaglianza
contro l'indice fisico durante `open`. Nuovo fuzz target dedicato:
`ipc_field_id_metadata_out_of_range`.

- Crate: `plenora-io-model`, `plenora-io-core`, `driver-ipc`.
- Test: unit test negativo su `.arrow` con `plenora.field_id` fuori
  intervallo; deve produrre `PlenoraIoError::Contract`, non panic.
- Evidence: nuovo target fuzz aggiunto a `scripts/fuzz-smoke.sh`; rimane
  fuori da `fuzz/quarantine.txt`.

### L2 — Materializzazione `BudgetedReader` (finding #2)

Dipende da PR-1. Due varianti:

- **A**: mantenere l'atomicita' operativa aggiungendo uno spool bounded
  su file temporaneo (nuovo `StagedSpool`); primo batch ritorna dopo la
  validazione ma senza tenere il dataset in RAM.
- **B**: rilasciare l'atomicita' operativa; primo batch ritorna subito,
  errore terminale invalida i batch gia' consegnati e viene segnalato
  come `TerminatedAfterAcceptedBatches`.

La scelta e' dell'ADR. Impatto su `plenora-io-core::driver`
significativo, catena downstream `plenora-data-tools` va rivalutata.

### L3 — CLI limits wiring (finding #3)

Dipende da PR-2. `read_options` e `cmd_convert` costruiscono
`ReadOptions`/`WriteOptions` con `resource_budget` derivato da
`cli.limits`. Modifica confinata al binario CLI se PR-2 espone un
costruttore `ResourceBudget::from_limits(&Limits)`.

### L4 — GeoParquet bbox robusto (finding #4)

- Riconoscere il covering tramite metadata GeoParquet dedicati
  (`covering` sul field geometrico) invece che per nome.
- Rifiutare a `open` una collisione con nomi utente.
- Rifiutare a `create` un `WritePlan` che includa i nomi interni,
  invece di sovrascriverli silenziosamente.

Crate: `driver-geoparquet`.

### L5 — GeoPackage integrita' (finding #5, #7, #12)

- **#5**: cursore keyset `Option<i64>` (prima query senza limite
  inferiore, poi `WHERE rowid > ?1` dal primo rowid osservato).
- **#7**: leggere flag ed SRS ID dall'header per-feature e confrontarli
  con il CRS di tabella; discordanza → `LossReport` con categoria
  dedicata, oppure errore secondo la policy scelta.
- **#12**: `gpkg_contents.last_change` come `strftime('%Y-%m-%dT%H:%M:%fZ',
  'now')`; flag "empty" quando la WKB non ha coordinate reali.

Crate: `driver-gpkg`. Suite: nuove fixture con rowid `<= 0` e con SRS
per-feature discordanti.

### L6 — Bounded parsing (finding #6)

- `parse_wkt` accetta `WktLimits` (o riutilizza `WkbLimits`) e conta
  vertici/componenti/depth in linea durante il tokenizer.
- GeoJSON: streaming del parser sulla singola geometria, con contatori
  applicati prima della materializzazione del `Vec` figlio.

Crate: `driver-common`, `driver-csv`, `driver-geojson`.

### L7 — Fidelity GeoJSON (finding #8)

- Descriptor GeoJSON: `Fidelity::Conditional`, coerente con il principio
  scritto in `IMPLEMENTATION_STATUS.md`.
- Validare `type = FeatureCollection|Feature|Geometry` prima di
  accettare il documento; rifiuto tipizzato altrimenti.
- Registrare in `LossReport` categorie dedicate per `id`, `bbox`,
  foreign members ignorati; il writer li dichiara come `write_loss` se
  presenti in ingresso.

Crate: `driver-geojson`.

### L8 — WKB `max_components` (finding #9)

Il contatore `max_components` addebita ogni sub-geometria di una
collection (Multi*, GeometryCollection) nell'ingresso al loop; il test
di `wkb_lossless` blocca il conteggio contro un payload artificiale
con N geometrie figlie vuote.

Crate: `plenora-io-model`.

### L9 — Shapefile loose atomico (finding #10)

Due opzioni:

- **A**: rendere `finish` del "loose set" fail-closed se non e'
  disponibile un rename directory-scope; documentare che il loose set
  restituisce sempre `ShapefileDirectoryDataset`.
- **B**: implementare rollback dei companion gia' rinominati (best-effort,
  non crash-atomic).

Il contratto pubblico di `FormatWriter::finish` va aggiornato per
riflettere la scelta.

Crate: `plenora-io-core`, `driver-shp`.

### L10 — Refactor `DataContract` validation (opzionale)

Un unico `DataContract::validate(&Schema, &RecordBatch)` che controlli
`field_id`, nome, indice, tipo e metadati. Blocca elegante per L1,
riduce il rischio di regressioni analoghe a #1 su altri driver.

Crate: `plenora-io-model`.

## Ordine consigliato

1. PR-1 (ADR-IO 7), PR-2 (Limits/Resource unificati) — in parallelo.
2. L1, L8 — richiedono solo PR-2/PR-1 per la coerenza dei limiti.
3. L4, L5, L6, L7 — indipendenti; L5 e L6 hanno impatto maggiore sui
   fuzz target esistenti.
4. L2 — dopo PR-1 chiuso.
5. L3, L9, L10 — cleanup finale.

## Criteri di uscita `1.1.0`

- Tutti i lotti L1-L9 chiusi e coperti da test.
- Nuovi fuzz target (almeno L1) verdi nel `scripts/fuzz-smoke.sh`.
- Campagna lunga `scripts/fuzz-campaign.sh` senza finding non
  classificati.
- CI same-SHA verde sulle quattro matrici (Linux, Windows, macOS,
  FileGDB/GDAL).
- Coverage LCOV `>= 80%` sul solo codice di libreria.
- Cross-component roundtrip con `plenora-data-tools` e
  `plenora-database-tools` alla stessa revisione.
- Nuovo `release/1.1.0.json` con evidence base propria, delta
  funzionale dichiarato e supersedes `v1.0.1`.
- `IMPLEMENTATION_STATUS.md` aggiornato: tabella ADR con la nuova ADR-IO
  7 e le finding chiuse rimosse dai "gap principali".

## Impatto atteso sul contratto CLI

- `plenora-io-error-v1`: possibili nuovi `error.category` per rowid
  fuori range, SRS-per-feature discordante, GeoJSON type invalido,
  WKT/WKB bounded parse; nessuna incompatibilita' se aggiunti come
  varianti *nuove*.
- `plenora-io-catalog-v1`: descrittori bump `descriptor_version` per i
  driver toccati (GeoParquet, GPKG, GeoJSON).
- `plenora-io-convert-v1`: `conversion_fidelity` puo' passare da
  `Lossless` a `Conditional` per GeoJSON — cambio dichiarato non
  silenzioso.
- `plenora-io-read-v1` / `plenora-io-inspect-v1`: nessuna modifica
  attesa.

## Rischi

- L2 riscrive un percorso critico; una regressione qui vale piu' di una
  qualunque delle altre finding messe insieme.
- L5.#7 puo' spezzare consumer che oggi ignorano lo shift di SRS
  per-feature (falsa fedelta' inconsapevole). Va comunicato prima del
  tag.
- L9 opzione A rimuove una modalita' compatibile di output; opzione B
  aggiunge complessita' non crash-atomic. La scelta ha impatto su
  utenti reali.

## Fuori scope `1.1.0`

- Refactor L10 se il tempo non lo permette.
- Modello dichiarativo `format_options` (PR-3): puo' scivolare a `1.2`.
- Rimozione dei tre finding upstream `arrow-rs` in
  `fuzz/quarantine.txt`: dipende da `apache/arrow-rs#10575`.
