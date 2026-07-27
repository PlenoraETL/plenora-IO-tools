# Registro dei fallback `unwrap_or*`

Data del censimento: 2026-07-27.

Il lint contro `unwrap`/`expect` impedisce panic espliciti, ma non dimostra che
un valore assente non venga sostituito con un dato inventato. Per questo
`unwrap_or`, `unwrap_or_else` e `unwrap_or_default` sono trattati come decisioni
semantiche soggette a H-01.

Il censimento iniziale contava 103 occorrenze nell'intero workspace. Questa
revisione elimina undici fallback che potevano nascondere righe CSV incomplete,
coordinate malformate, CRS privi di ID, layer FileGDB senza geometria e numeri
XLSX non rappresentabili. Tre nuovi `unwrap_or_else`, esclusivamente diagnostici
nei test di conformance, portano il saldo netto a 95 occorrenze:

- 54 nei sorgenti dei crate distribuibili, includendo conservativamente i loro
  moduli `#[cfg(test)]`;
- 41 nei target esclusi dal componente (`plenora-io-cli` 19,
  `plenora-bench` 16, `plenora-fuzz` 6).

`scripts/check_assurance_fallbacks.sh` blocca in CI ogni variazione di tutte le
95 occorrenze, inclusi i target non distribuibili. L'aggiornamento del registro
non è una deroga automatica: richiede la revisione della nuova semantica e una
change impact analysis.

## Censimento del componente distribuibile

| Crate | Conteggio | Decisione verificata |
|---|---:|---|
| `driver-csv` | 4 | delimitatore di default dichiarato; nome/path diagnostici; `Unknown` solo quando il set dimensionale non è singleton |
| `driver-dxf` | 15 | classificazione CRS senza default operativo; terminazione parser; nome/path; Z=0 soltanto per geometrie XY; assi OCS e scale di default definiti dal modello DXF e coperti dai test geometrici |
| `driver-filegdb` | 3 | path/stem di staging non semantici; un fallback `custom` è confinato al costruttore dei test |
| `driver-geojson` | 3 | nome/path non semantici; XY è usato soltanto per una geometria senza coordinate da cui inferire dimensioni |
| `driver-geoparquet` | 5 | nome/path; pruning fail-open; dimensioni eterogenee → `Unknown`; codice CRS conservato se PROJJSON non serializza |
| `driver-gpkg` | 5 | path; `undefined` richiesto dalla tabella SRS quando manca WKT; raw CRS conserva almeno l'ID; tipo `GEOMETRY` e dimensioni `Unknown` sono valori nativi espliciti |
| `driver-ipc` | 3 | nome/path; projection best-effort conserva il campo originale se non esiste una sostituzione |
| `driver-kml` | 6 | nome/path; collezioni/coordinate vuote non forniscono Z; flag assente=false; eterogeneità dimensionale → `Unknown` |
| `driver-shp` | 3 | nome/path; stringa vuota usata solo dalla classificazione di una definizione opzionale, non come CRS operativo |
| `driver-xls` | 4 | path; celle fisicamente assenti diventano blank/null; eterogeneità dimensionale → `Unknown` |
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

## Regola di riesame

Ogni fallback che tocca coordinate, CRS, precisione numerica, null, attributi o
metadati nativi deve essere eliminato oppure accompagnato da:

1. origine normativa o contratto che definisce il valore di default;
2. test che distingue assenza, valore invalido e valore di default;
3. rendicontazione nel `LossReport` se la sostituzione può perdere informazione.
