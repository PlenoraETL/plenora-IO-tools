# Change impact analysis — seconda campagna di ottimizzazione

Data: 2026-07-28

## Baseline e fonti

- baseline di codice:
  `c14658e28b11c94201f465cdb289ac4bcd38ce0c`;
- ICD ispezionato: `plenora-contracts@v2.0-rc8`, revisione
  `62b12e3496466d2c908dac3cc098640b99b52e21`;
- ambiente di misura: Rust 1.92.0, Linux x86_64 in container, GDAL 3.10.3;
- regola di veto: nessun intervento sul percorso dati viene mantenuto se perde
  oltre il 5% di throughput o peggiora materialmente la memoria sulla misura
  comparabile.

L'ICD resta parzialmente ratificato. L'aggiornamento da rc3 a rc8 non promuove
la RC di componente a RC di sistema e non modifica la wire version
`plenora.contract.version=1`. La modifica sostanziale intervenuta fra le due
revisioni è l'emendamento di §15.4: l'emissione delle chiavi candidate è ammessa
con deroga registrata (`DER-ICD-002`).

## Interventi e decisioni

| Area | Decisione | Effetto |
|---|---|---|
| DXF writer | Mantenuto | Serializzazione diretta su `BufWriter` da 1 MiB, con limite verificato durante la scrittura invece di un `Vec` contenente l'intero file |
| KML reader file-backed | Respinto e ripristinato | I prototipi con buffer da 8 KiB e 1 MiB riducevano RSS ma perdevano rispettivamente fino al 52% e al 22% in lettura |
| KML writer | Mantenuto | `KmlWriter` scrive direttamente su `BufWriter` da 4 MiB; eliminato il buffer completo per batch |
| Projection | Mantenuto | CSV, GeoJSON, Shapefile e FileGDB passano a `Exact`; le colonne escluse non vengono parse, convertite o costruite |
| Batch adattivi | Mantenuto | Un unico `AdaptiveBatchSizer` usa la stima iniziale e poi i byte Arrow osservati in CSV, GeoJSON, SHP, FileGDB e GPKG |
| Shapefile writer | Mantenuto | Il decoder cede l'ownership delle coordinate alla shape ESRI; eliminate le copie profonde di linee e anelli |
| `schema_hint` XLSX | Non implementato | La nozione non compare nell'ICD 2.0-rc8; aggiungerla unilateralmente cambierebbe la superficie pubblica senza contratto |

## Projection e contratto effettivo

`project_layer_contract` centralizza validazione, deduplicazione e ordinamento
degli ID, schema effettivo e ri-indicizzazione della geometria. In modalità
`Required` un ID fuori range fallisce all'apertura; in `BestEffort` viene
ignorato. La projection vuota conserva il numero di righe mediante
`RecordBatchOptions::with_row_count`.

Le versioni catalogo cambiano per rendere osservabile la nuova semantica:

- CSV e GeoJSON: `driver_version/descriptor_version = 6/6`;
- Shapefile: `7/6`;
- FileGDB: `9/8`.

Un nuovo gate di conformance esercita la projection vuota su tutti i reader
pure-Rust che dichiarano `Exact`. Il gate ha individuato anche un difetto
preesistente di GeoParquet: il retag di un batch senza colonne perdeva il numero
di righe. Il percorso usa ora anch'esso `RecordBatchOptions`.

## Evidenza prestazionale

Le mediane sono calcolate su cinque ripetizioni, 100.000 righe, stesso host e
stessa build release salvo la variante A/B indicata.

### DXF

- write throughput: **+42,75%**;
- RSS: **+0,21%**;
- byte allocati: 485.097.086 → 452.591.238.

La lettura non è stata modificata; la variazione osservata lì è rumore di
host/cache e non viene attribuita all'intervento.

### KML

- writer diretto: throughput **+3,18%**, RSS **−70,00%**;
- reader finale: invariato, perché i due prototipi file-backed non hanno
  superato il veto e sono stati rimossi.

### Projection CSV e GeoJSON

La variante di riferimento usa `BestEffort` su driver non exact e restituisce
lo schema completo; la variante nuova richiede solo `id` e `val`.

| Driver | Mediana wall | Allocazioni | Byte allocati | Codifiche WKB |
|---|---:|---:|---:|---:|
| CSV prima | 262,49 ms | 608.369 | 19.112.761 | 100.000 |
| CSV dopo | 240,59 ms | 304.035 | 7.920.502 | 0 |
| GeoJSON prima | 520,83 ms | 1.200.828 | 94.408.442 | 100.000 |
| GeoJSON dopo | 516,87 ms | 600.192 | 7.304.998 | 0 |

Il percorso completo, senza projection, è stato interlacciato con la baseline:
CSV e GeoJSON restano entro circa il 2% e quindi non mostrano regressione
attribuibile. Sui batch proiettati il dimensionatore riduce da otto a due batch
e mantiene il massimo osservato vicino a 1 MiB.

### Shapefile

Benchmark A/B su 100.000 poligoni, filesystem nativa del container:

- read throughput: **−0,10%**, RSS **−1,93%**;
- write throughput: **+0,73%**, RSS **−1,42%**;
- allocazioni write: 1.200.243 → 1.100.243.

Il confronto automatico supera il veto. L'esecuzione sulla bind mount Windows
ha inoltre confermato che `renameat2(RENAME_NOREPLACE)` può essere rifiutata con
`EINVAL` da quel filesystem; la misura è stata quindi eseguita sulla filesystem
Linux nativa, senza allentare il fail-closed del publish.

## Hazard e failure mode

- H-01: la projection non inventa colonne o geometrie; lo schema effettivo è
  autoritativo e la projection vuota conserva le righe.
- H-03: il limite DXF è applicato prima di oltrepassare il budget; il
  dimensionatore adattivo resta limitato da `max_rows` e usa aritmetica
  saturating.
- H-04: nessun nuovo panic o `unsafe` nei crate di libreria.
- H-08: test trasversale delle capability, test dimensionali Shapefile,
  benchmark riproducibili e smoke fuzz.
- H-09: i prototipi KML respinti sono registrati insieme alla causa e non
  rimangono nel prodotto.

## Verifica

Superati:

- `cargo test --workspace --all-targets --all-features --locked`;
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`;
- safety Clippy sui target `lib`, incluso il divieto di `unsafe`, `unwrap`,
  `expect`, `panic`, `unreachable`, `todo` e `unimplemented`;
- build release dell'intera workspace;
- test FileGDB/GDAL: 21 superati, 2 helper eseguiti dai test di crash;
- test reale cross-filesystem `/dev/shm`;
- gate Action, identità pubblica, provenienza RC, dipendenze esatte, grafi
  locked e registro dei fallback; fallback invariati a 88;
- smoke fuzz strutturato di 15 secondi: 28.760.000 iterazioni, zero finding;
- `cargo fmt --all -- --check` e `git diff --check`.

`cargo-audit` e i target libFuzzer nightly non sono installati nell'immagine
locale: restano gate della CI e non vengono dichiarati come eseguiti da questa
sessione.

## Residui

- KML, DXF e XLSX restano materializzanti in lettura per limiti delle API
  upstream e del contratto.
- FileGDB applica una projection esatta nel driver, ma il pushdown nativo GDAL
  delle sole colonne richieste richiede una misura dedicata multi-versione.
- `schema_hint` va prima definito e governato dall'ICD; solo dopo è sensato
  riaprire il reader XLSX incrementale a una passata.
- La RC resta del componente. Il round-trip eseguibile IO → data → database e
  ritorno è ancora il gate aperto della catena.

Questa evidenza non costituisce certificazione né dichiarazione di conformità
DO-178C/ED-12C.
