# ADR-IO 6 — `ReadRequest`: pruning vs filtering

**Stato:** Accettato (Fase 0). Vincola D8 di `Architetture.md`.

## Contesto

Per non leggere dati inutili (soprattutto da Parquet) serve un pushdown nella
richiesta di lettura. Ma il pushdown non deve trasformare plenora-IO-tools in un
query engine: la semantica generale del filtro appartiene a `plenora-data-tools`.

## Decisione

```rust
struct ReadRequest {
    layer: LayerId,
    projected_fields: Option<Vec<FieldId>>,        // projection pushdown
    pruning_predicate: Option<PruningPredicate>,   // suggerimento di pruning via metadati
    spatial_pruning_hint: Option<Bbox>,            // pruning spaziale via indice/statistiche
    batch_target: BatchTarget,                     // target_batch_bytes / max_rows
}
```

**1. `projected_fields` — projection pushdown.** Un driver colonnare
(Parquet/IPC) legge solo le colonne richieste.

- **La geometria NON è sempre forzata.** Viene inclusa quando è **richiesta
  dalla projection**, **necessaria per lo `spatial_pruning_hint`**, oppure
  **richiesta dal contratto del consumatore**. Una lettura **puramente tabellare**
  da GeoParquet deve poter **evitare** una colonna geometria pesante (era una
  regressione forzarla sempre).

**1b. `ProjectionMode`** — la projection non è solo best-effort:

```rust
enum ProjectionMode { Required, BestEffort }
```

- **`Required`**: il driver deve produrre **esattamente** la projection; se non
  può (non colonnare), **fallisce all'apertura del reader**. È spesso più sicuro
  per l'integrazione stretta con `plenora-data-tools`, che si aspetta uno schema
  preciso.
- **`BestEffort`**: il driver può restituire colonne aggiuntive; lo **schema
  effettivo del reader resta autoritativo**.

**1c. Lo schema effettivo è esposto direttamente.** Il consumatore non deve
inferirlo: `LayerReader` espone il proprio contratto.

```rust
trait LayerReader {
    fn contract(&self) -> &LayerContract;                 // schema effettivo, autoritativo
    fn next_batch(&mut self) -> Result<Option<RecordBatch>>;
}
```

**2. `pruning_predicate` / `spatial_pruning_hint` — solo PRUNING, mai
filtering.** Sono **suggerimenti**: onorati **solo** se il driver ha una
capacità nativa chiaramente equivalente e documentata (min/max dei row group
Parquet, indice spaziale/`gpkg_rtree`, partizioni). Altrimenti **ignorati** (il
reader restituisce tutte le righe del layer).

**3. Invariante di correttezza del pruning: over-return, mai under-return.** Il
pruning può **escludere solo blocchi sicuramente incompatibili** dai metadati e
può quindi restituire **falsi positivi** (righe che poi il filtro esatto
scarterà); non deve **mai** escludere una riga che *potrebbe* corrispondere
(nessun falso negativo). Un driver che non può garantirlo per un dato predicato
lo **ignora**. **Vietato approssimare un filtro.**

**4. Il filtering riga-per-riga resta a `plenora-data-tools`.** Il predicato
esatto viene applicato dallo step `table.filter`/`geo.*`: plenora-IO-tools riduce
il volume letto, data-tools decide la semantica finale.

**5. `batch_target`** è un obiettivo best-effort in byte e righe (V7 di
`Prestazioni.md`); per le geometrie prevale il limite in byte.

## Stato dell'implementazione (2026-07-27)

- Il descrittore v3 espone `projection_support = exact | none`.
- GeoParquet e Arrow IPC dichiarano `exact` e producono un contratto proiettato;
  IPC applica la projection direttamente al `FileReader`.
- I driver non exact rifiutano `ProjectionMode::Required` all'apertura con
  l'errore tipizzato `ProjectionUnsupported`; `BestEffort` continua a poter
  restituire lo schema completo.
- CSV, GeoJSON, GeoPackage, Shapefile, FileGDB e GeoParquet combinano
  `max_rows` con una stima conservativa di `target_bytes` per scegliere la
  dimensione dei batch incrementali. La stima non è un limite rigido di memoria.
- Arrow IPC e i reader materializzati KML, DXF e XLSX passano attraverso
  `with_batch_target`, che suddivide i `RecordBatch` senza copiare né riordinare
  le righe e conserva il lifecycle del reader sottostante. Lo slicing condivide
  i buffer Arrow: riduce la cardinalità esposta, non la memoria già allocata.
- Le versioni dei driver nel catalogo sono state incrementate affinché il
  fingerprint distingua questa semantica di lettura.
- Il pruning conservativo effettivo resta al momento specifico di GeoParquet.

## Conseguenze

- Il pushdown migliora le prestazioni (meno row group letti) senza duplicare la
  semantica del filtro: nessun rischio di divergenza fra il "filtro" dell'I/O e
  quello del motore.
- Il consumatore deve leggere lo schema effettivo del reader: con `Required`
  coincide con la projection, con `BestEffort` può contenere colonne extra.
- Test obbligatori: Parquet con pruning effettivo (row group saltati > 0) che
  però restituisce falsi positivi corretti; un predicato non supportato che
  viene ignorato (tutte le righe passano); nessun falso negativo su un dataset
  di riferimento; `Required` rifiutata fail-closed da ogni driver non exact;
  matrice catalogo/implementazione e projection esatta sui driver che la
  dichiarano.

## Alternative scartate

- **Filtering esatto nel driver**: renderebbe IO-tools un mini query engine, con
  duplicazione e rischio di divergenza dalla semantica di data-tools.
- **Pruning che può escludere falsi negativi**: produrrebbe risultati errati; il
  pruning deve essere conservativo per definizione.
