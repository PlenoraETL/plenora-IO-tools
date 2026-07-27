# Stato implementazione rispetto agli ADR

Verifica aggiornata al 2026-07-27. La tabella distingue ciò che è nel codice da
ciò che resta una decisione architetturale: “parziale” non significa che il
driver non funzioni, ma che non soddisfa ancora tutte le invarianti dell’ADR.

| ADR | Stato corrente | Implementato | Gap principali |
|---|---|---|---|
| ADR-IO 1 — lifecycle e `WritePlan` | Parziale avanzato | Trait e ciclo `open`/reader, `create`/writer/`finish`; handle esplicitamente `Send + Sync`; writer multi-layer; nomi layer obbligatori e unici validati in comune; `SingleActiveReader` imposto con lease atomico ed errore tipizzato su KML/DXF/XLSX e backend FileGDB; matrice reale single/multiple; gate trasversale su piano vuoto/multi-layer/duplicati e abort senza publish né residui per tutti i writer pure-Rust | Alcuni `open` materializzano righe; il backend FileGDB richiede ancora un test di concorrenza feature-on con GDAL; restano da uniformare cancellazione e streaming dei parser che materializzano |
| ADR-IO 2 — publish atomico | Parziale avanzato | Tempfile same-directory e no-clobber per file singoli; GeoPackage multi-layer pubblicato come singolo file; Shapefile disponibile come `ShapefileDirectoryDataset` forte (`*.shp.d`, un rename) e come loose set compatibile (`*.shp`, companion ordinati); sequenza durable condivisa con `fsync` fail-closed dell'intero staging tree prima del rename ed esito post-rename tipizzato | Il loose set resta deliberatamente non atomico in senso forte; mancano fault injection del crash di processo, test cross-filesystem e conferma multipiattaforma delle garanzie di durabilità |
| ADR-IO 3 — capability-check | Parziale avanzato | `FormatWriteCapabilities` machine-readable su tutti i driver scrivibili; policy nomi/tipi/attributi/geometria/CRS/nullability/multi-layer; validatore statico prima della creazione e guardia runtime comune sui payload WKB; errori `CapabilityReason` tipizzati; matrice negativa derivata dai descrittori per limiti, nomi, tipi, encoding, dimensioni, semantica e geometrie miste | Alcuni vincoli dipendenti dai valori restano nel primo `write`; il modello di coercion/report va raffinato; i rami `AttributeWriteSupport::{None, NamedSubset}` e `NoNulls` sono pronti nel gate ma non sono oggi dichiarati da alcun driver reale |
| ADR-IO 4 — CRS | Parziale avanzato | `CrsResolution::{Resolved, DeclaredButUnresolved, Missing}` e `RawCrs`; CSV/XLSX richiedono `assume_crs`; CRS fissi KML/GeoJSON validati in `create`; GPKG/SHP/GeoParquet/IPC propagano metadati CRS; DXF legge e scrive `GEODATA.coordinate_system_definition`, risolve gli identificativi EPSG riconoscibili e fallisce chiuso senza fallback esplicito; gate di scrittura su ogni capability `Embedded` e `Fixed` dichiarata | Non tutti i parser conservano ancora il CRS grezzo non risolto nel contratto; restano da uniformare i test di lettura del `RawCrs` e degli ordini assi; la serializzazione DXF di un authority id non-WGS84 richiede ancora la definizione WKT/XML completa |
| ADR-IO 5 — fedeltà | Parziale avanzato | `Fidelity` nel descrittore; `FidelityAssessment` bounded e serializzabile restituito da `open`/`create` e nel `Published`; motivi tipizzati per vincoli, attributi, coercion, nullability, struttura, precisione e metadati; il wrapper comune collega il contratto e promuove automaticamente l'esito a `Approximating` quando il `LossReport` osservato non è vuoto; DXF registra tassellazioni, esplosioni INSERT, conversioni di testo/solidi, entità non gestite e attributi non rappresentati | I profili dei driver `Conditional` sono ancora conservativi e vanno raffinati per singolo tipo/valore; alcuni driver conditional producono report operativi vuoti; oracoli indipendenti e corpus reali non sono uniformi |
| ADR-IO 6 — projection e pruning | Parziale avanzato su GeoParquet, iniziale altrove | Contratto `ReadRequest`, schema effettivo dal reader, projection/pruning e test conservativi GeoParquet | `ProjectionMode::Required` non è fail-closed in tutti i driver; `target_bytes` è poco applicato; IPC e formati non colonnari devono dichiarare/applicare esplicitamente il comportamento |

## Contratti trasversali introdotti

- Il contratto geometrico distingue `Wkb`/`Ewkb`, `Xy`/`Xyz`/`Xym`/`Xyzm`,
  `Geometry`/`Geography`, SRID, precisione, tipi geometrici e metadati nativi.
  Il core ora decodifica e ricodifica senza perdita WKB ISO ed EWKB, incluse
  dimensioni Z/M, SRID, endianess e geometrie annidate, con limiti e rifiuto dei
  byte residui. IPC conserva payload e contratto completi; GeoPackage conserva
  payload ISO WKB, SRID e flag nativi Z/M; GeoParquet conserva WKB dimensionali,
  emette `geometry_types` Z/M e calcola il bbox sulle sole coordinate XY;
  Shapefile converte direttamente le varianti Shape/ShapeM/ShapeZ in WKB
  XY/XYM/XYZ/XYZM per punti, multipunti, linee e poligoni, conserva il sentinel
  ESRI `NO_DATA` nelle misure e pubblica `shp.shape_type` nei metadati nativi;
  `Multipatch` resta fail-closed perché non ha una corrispondenza WKB univoca;
  GeoJSON conserva XY/XYZ direttamente tra coordinate JSON e WKB, rifiutando
  una quarta ordinata dalla semantica M ambigua; KML conserva XY/XYZ direttamente
  tramite i tipi nativi con quota e rifiuta M e le `GeometryCollection` omogenee
  ambigue rispetto ai tipi `Multi*`; DXF usa ora direttamente l'AST WKB,
  conserva XY/XYZ e le trasformazioni 3D di primitive, OCS e blocchi INSERT,
  tassella in 3D SPLINE ed ELLIPSE inclinate e pubblica tipi geometrici esatti
  nel contratto. Poiché DXF non distingue formalmente una quota Z esplicita
  uguale a zero da una coordinata 2D, il dataset è dichiarato XYZ se almeno una
  quota è non nulla, altrimenti XY; M ed EWKB con SRID embedded sono rifiutati.
  Una pre-validazione XML limitata protegge
  inoltre il parser KML da token malformati che possono impedirne l'avanzamento.
  Una guardia runtime comune decodifica i
  payload prima dei writer e rifiuta dimensioni, SRID, tipo o nullability diversi
  dal contratto, inclusi i vecchi contratti con il solo marker GeoArrow. CSV e
  XLSX convertono ora il WKT direttamente nell'AST WKB lossless e conservano
  XY/XYZ/XYM/XYZM, tipo geometrico e precisione `f64`; dichiarano la dimensione
  esatta per colonne omogenee e `Unknown` per colonne miste. Le colonne X/Y
  restano intenzionalmente Point XY; EWKB con SRID e semantica geography sono
  rifiutati perché non rappresentabili dal WKT semplice.
- `Limits` è parte di `ReadOptions` e `WriteOptions`. La dimensione complessiva
  dell’input è verificata prima del parser; righe e colonne sono conteggiate da
  un wrapper comune dei writer; i writer geometrici v1 usano i limiti WKB
  forniti dal chiamante. `max_vertices` restringe il numero effettivo di
  componenti WKB e `max_output_bytes` è verificato sullo staging prima del
  publish per file singoli, loose set Shapefile e directory FileGDB. Il reader
  DXF applica inoltre `max_rows`, `max_columns` e un budget cumulativo
  `max_vertices` durante la materializzazione e mantiene limiti separati su
  annidamento ed esplosione degli INSERT.
- Il catalogo espone le capability di scrittura con `descriptor_version = 2`.
- I comandi `inspect`, `layers`, `read` e `convert` espongono la valutazione di
  fedeltà concreta. `create` la rende interrogabile sul writer prima del primo
  batch; `finish` la aggiorna con le categorie realmente presenti nel
  `LossReport`. Le motivazioni sono deduplicate e limitate a 64 elementi.

## Decisione sui fuzz test

Gli smoke fuzz restano utili a ogni modifica dei parser e del core e coprono ora
anche l'invariante decode/encode/decode lossless di WKB ISO ed EWKB e la
conversione dimensionale WKB ↔ Shape ESRI, oltre al round-trip WKT ↔ WKB di
CSV/XLSX e al parser/walker DXF → WKB XY/XYZ. I target libFuzzer
coverage-guided sono ora sei e il target DXF parte da un seed ASCII 3D minimo.
Una campagna breve ha individuato e chiuso l'accettazione di coordinate
WKT non finite prodotte da overflow numerico. La
precondizione geometrica e il gate trasversale negativo su capability, CRS di
scrittura e lifecycle dei writer pure-Rust sono ora presenti. Una campagna
lunga generale può quindi essere eseguita come gate della prossima milestone,
affiancata da campagne mirate sul core geometrico. Restano separati i test di
concorrenza dei reader, di lettura del `RawCrs` e di crash durante il publish:
il fuzz non sostituisce questi test di contratto.
