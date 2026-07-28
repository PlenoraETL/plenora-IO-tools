# Registro dei fallback `unwrap_or*`

Data del censimento: 2026-07-28.

Il lint contro `unwrap`/`expect` impedisce panic espliciti, ma non dimostra che
un valore assente non venga sostituito con un dato inventato. Per questo
`unwrap_or`, `unwrap_or_else` e `unwrap_or_default` sono trattati come decisioni
semantiche soggette a H-01.

Il censimento iniziale contava 103 occorrenze nell'intero workspace. La prima
revisione aveva portato il totale a 95. La pulizia profonda successiva elimina
altri tredici fallback: nove risoluzioni duplicate della directory di staging,
il default XY per geometrie GeoJSON/KML senza coordinate e altri default
booleani/dimensionali KML non più necessari. Il saldo è ora 82 occorrenze:

- 41 nei sorgenti dei crate distribuibili, includendo conservativamente i loro
  moduli `#[cfg(test)]`;
- 41 nei target esclusi dal componente (`plenora-io-cli` 19,
  `plenora-bench` 16, `plenora-fuzz` 6).

`scripts/check_assurance_fallbacks.sh` blocca in CI ogni variazione di tutte le
82 occorrenze, inclusi i target non distribuibili. L'aggiornamento del registro
non è una deroga automatica: richiede la revisione della nuova semantica e una
change impact analysis.

La centralizzazione del lifecycle `StagedFile` e l'estrazione del codec
geometrico GeoJSON del 2026-07-28 non introducono né rimuovono fallback: il
totale verificato resta 82.

## Censimento del componente distribuibile

| Crate | Conteggio | Decisione verificata |
|---|---:|---|
| `driver-csv` | 3 | delimitatore di default dichiarato; nome diagnostico; `Unknown` solo quando il set dimensionale non è singleton |
| `driver-dxf` | 14 | classificazione CRS senza default operativo; terminazione parser; nome diagnostico; Z=0 soltanto per geometrie XY; assi OCS e scale di default definiti dal modello DXF e coperti dai test geometrici |
| `driver-filegdb` | 3 | path/stem di staging non semantici; un fallback `custom` è confinato al costruttore dei test |
| `driver-geojson` | 1 | nome diagnostico; geometrie senza coordinate sono ora rifiutate |
| `driver-geoparquet` | 4 | nome; pruning fail-open; dimensioni eterogenee → `Unknown`; codice CRS conservato se PROJJSON non serializza |
| `driver-gpkg` | 4 | `undefined` richiesto dalla tabella SRS quando manca WKT; raw CRS conserva almeno l'ID; tipo `GEOMETRY` e dimensioni `Unknown` sono valori nativi espliciti |
| `driver-ipc` | 2 | nome; projection best-effort conserva il campo originale se non esiste una sostituzione |
| `driver-kml` | 2 | nome diagnostico; eterogeneità dimensionale → `Unknown`; geometrie vuote sono rifiutate |
| `driver-shp` | 2 | nome diagnostico; stringa vuota usata solo dalla classificazione di una definizione opzionale, non come CRS operativo |
| `driver-xls` | 3 | celle attributive fisicamente assenti diventano blank/null; eterogeneità dimensionale → `Unknown`; le coordinate assenti, invalide o lossy sono distinte |
| `plenora-io-model` | 1 | metadato GeoArrow assente significa “non è un campo geometrico” |
| `plenora-io-core` | 2 | un path senza parent esplicito usa la directory corrente, senza modificare dati o contratti |

## Target non distribuibili

| Crate | Conteggio | Decisione verificata |
|---|---:|---|
| `plenora-io-cli` | 19 | default CLI dichiarati (`layer=0`, estensione assente); presentazione di CRS unresolved; fallback di serializzazione confinati al protocollo diagnostico; messaggi `panic!` esclusivamente nei test di conformance |
| `plenora-bench` | 16 | configurazione e soglie benchmark documentate; metriche mancanti rappresentate come zero o `?` soltanto nell'output del runner, senza entrare nei driver |
| `plenora-fuzz` | 6 | directory, durata e seed di campagna riproducibili; serializzazione best-effort confinata alla generazione del corpus, senza entrare nel componente distribuibile |

## Decisioni fail-closed introdotte

- CSV: le righe ragged non vengono più completate con stringhe vuote; coordinate
  X/Y mancanti o non numeriche producono errore.
- Shapefile, GeoPackage e DXF: un `ResolvedCrs` senza identificatore non viene
  etichettato rispettivamente `unknown`, `OGC:CRS84` o `DXF:GEODATA`.
- FileGDB: un layer senza campo geometrico non diventa implicitamente
  `wkbUnknown`.
- XLSX: un numero non convertibile in `f64` non diventa `0.0`.
- GeoJSON/KML: una geometria senza coordinate non viene dichiarata XY.
- XLSX XY: celle non numeriche/non finite o una sola ordinata presente non
  diventano geometria nulla; gli interi oltre la precisione esatta di `f64`
  sono rifiutati.
- Lo staging same-filesystem è creato da un unico helper del core; i driver non
  inventano più localmente il parent `"."`.

## Regola di riesame

Ogni fallback che tocca coordinate, CRS, precisione numerica, null, attributi o
metadati nativi deve essere eliminato oppure accompagnato da:

1. origine normativa o contratto che definisce il valore di default;
2. test che distingue assenza, valore invalido e valore di default;
3. rendicontazione nel `LossReport` se la sostituzione può perdere informazione.
