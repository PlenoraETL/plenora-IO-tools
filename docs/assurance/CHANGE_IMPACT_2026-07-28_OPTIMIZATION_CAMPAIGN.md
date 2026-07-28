# Change impact analysis — campagna di ottimizzazione mirata

Data: 2026-07-28

## Baseline, regola di decisione e perimetro

Baseline di codice:
`68e4108a7f49d84fd0265c375cee47679a4e25ca`.

La campagna ha esaminato otto interventi. Una modifica al percorso dati viene
mantenuta soltanto se i test di contratto restano verdi e il percorso
modificato non perde oltre il 5% di throughput o RSS rispetto alla misura
comparabile. I prototipi che non superano il veto vengono rimossi integralmente.

Non cambiano dipendenze, lockfile, toolchain, formato WKB/EWKB o chiavi Arrow.
Il descrittore GeoPackage cambia in modo osservabile: la projection passa da
`None` a `Exact`; `driver_version` e `descriptor_version` passano da 5 a 6.

## Esito degli otto punti

| Punto | Decisione | Evidenza |
|---|---|---|
| 1. Writer XLSX senza doppio buffering | Respinto e ripristinato | Scrittura diretta: throughput -14,11%, RSS -6,35%; con `BufWriter`: throughput -12,15%, RSS -6,44%. Il risparmio di memoria non compensa la regressione. |
| 2. KML pull-streaming bounded | Respinto e ripristinato | Il parser pubblico `kml` 0.14 espone il documento completo, non un iteratore. Il prototipo per frammenti ha ridotto RSS da circa 406 a 212 MB ma il throughput è sceso da circa 169k a 40k righe/s (-76%). |
| 3. DXF progressivo | Non implementato, prerequisito esterno | `dxf` 0.6.1 espone solo `Drawing::load*`; parser di entità e iteratore dei code-pair sono `pub(crate)`. Duplicare un parser non qualificato è escluso dal profilo aeronautico. Serve un iteratore upstream o un fork verificato. |
| 4. Reader XLSX incrementale | Non implementato, prerequisito di contratto | `calamine` 0.36.1 offre `worksheet_cells_reader`, ma il contratto corrente inferisce lo schema dall'intero foglio prima di restituire il reader. Senza `schema_hint`, una lettura esatta richiede due passate o inventa tipi. |
| 5. Copie GeoPackage | Mantenuto | Parametri SQLite presi in prestito per testo/blob, buffer geometrico riusato. Allocazioni write 400.241 → 100.245 (-75%), byte allocati 29,84 → 22,55 MB, throughput +4,11%, RSS +0,14%. |
| 6. Buffer WKB riusabile | Mantenuto | CSV usa `encode_wkb_into` nei percorsi WKT e XY. Allocazioni read 908.365 → 608.368 (-33%), throughput +7,55%. |
| 7. Visitor WKB | Mantenuto | `inspect_wkb` valida struttura, limiti, tipo, dimensioni e SRID senza materializzare l'AST. Su un milione di Polygon: mediana circa 58,8M contro 8,3M geometrie/s; 0 contro 2.000.000 allocazioni. |
| 8. Projection e batch adattivi GeoPackage | Mantenuto | La query seleziona solo le colonne richieste, supporta projection vuota e riindicizza il contratto. Su 500.000 righe, 7 coppie alternate: mediana 643k → 686k righe/s (+6,8%), CPU -21%, allocazioni -72%, RSS circa -26%, batch massimo 3,93 → 1,05 MB. |

## Modifiche mantenute

### Allocazioni e copie

Il writer GeoPackage passa a SQLite valori `ToSql` presi in prestito per
stringhe e blob. La durata dei riferimenti resta confinata alla singola
`execute`; il buffer del blob geometrico è svuotato e riusato alla riga
successiva. Non vengono conservati riferimenti oltre la chiamata SQLite.

Il reader CSV riusa il buffer WKB già associato al builder invece di sostituirlo
con una nuova `Vec` per ogni geometria.

### Visitor WKB differenziale

`inspect_wkb` usa lo stesso modello di header e gli stessi limiti del decoder
lossless, ma salta i byte delle coordinate dopo avere verificato conteggi,
profondità, tipo dei figli, dimensioni e trailing bytes. Il core usa il visitor
per il capability-check runtime, dove non è necessario possedere le coordinate.

Lo smoke fuzz confronta ora accettazione/rifiuto e metadati del visitor con il
decoder autoritativo. Con seed `20260728`, 10 secondi hanno eseguito 19.920.000
iterazioni senza finding.

### Projection GeoPackage e dimensionamento adattivo

La projection è applicata alla lista `SELECT`, non soltanto al `RecordBatch`.
Gli ID richiesti vengono validati, deduplicati e ordinati secondo lo schema
nativo. La geometria non viene letta per richieste tabellari, neppure con
spatial pruning RTree, perché l'indice basta al pruning conservativo.

Il primo batch usa la stima statica comune; i successivi usano i byte Arrow
effettivamente osservati, limitati da `target_bytes` e `max_rows`. Un batch
senza colonne conserva correttamente il numero di righe.

## Compatibilità e failure mode

Il passaggio GeoPackage a `ProjectionSupport::Exact` è additivo per i
consumatori, ma modifica il catalogo pubblico. `ProjectionMode::Required` ora
riesce per ID validi e fallisce all'apertura per ID fuori range; `BestEffort`
ignora gli ID non validi come gli altri driver exact. Lo schema del reader resta
autoritativo.

Il visitor non sostituisce il decoder nelle conversioni: viene usato solo
quando il chiamante richiede ispezione/validazione. Il fuzz differenziale
impedisce che le due superfici divergano silenziosamente.

## Hazard e controlli

- H-01: nessun tipo, CRS o valore mancante viene sintetizzato; projection vuota
  e geometria esclusa sono rappresentate nel contratto effettivo.
- H-03: memoria per batch controllata dai byte osservati, con limite minimo di
  una riga e massimo richiesto dal consumatore.
- H-06: SRID e dimensioni del visitor sono confrontati con il decoder lossless.
- H-08: test unitari, Clippy, smoke differenziale e benchmark riproducibile.
- H-09: interventi mantenuti, respinti e bloccati sono registrati con la causa.

## Evidenze di verifica

- `cargo test --workspace --all-targets --all-features --locked`: superato,
  inclusi i test FileGDB/GDAL e i test exact/empty projection GeoPackage;
- `cargo clippy --workspace --all-targets --all-features --locked -- -D
  warnings`: superato;
- gate Clippy delle librerie con divieto di `unsafe`, `unwrap`, `expect`,
  `panic`, `unreachable`, `todo` e `unimplemented`: superato;
- pin Action, identità pubbliche, provenienza RC, dipendenze esatte, grafi
  locked e registro dei fallback: superati; fallback revisionati invariati a
  88;
- build release dell'intero workspace e test cross-filesystem `/dev/shm`:
  superati;
- smoke differenziale WKB, seed `20260728`: 19.920.000 iterazioni, zero
  finding.

## Residui

- KML richiede un parser event-based diretto o un iteratore upstream.
- DXF richiede accesso pubblico al parser progressivo e una passata preliminare
  verificata sulle block table.
- XLSX richiede un `schema_hint` nel contratto di lettura prima della conversione
  a una passata.
- Il batch adattivo è implementato nel reader GeoPackage; estenderlo agli altri
  reader va deciso per driver e misurato, perché lo slicing di batch già
  materializzati non libera i buffer sottostanti.

Questa evidenza non costituisce certificazione né dichiarazione di conformità
DO-178C/ED-12C.
