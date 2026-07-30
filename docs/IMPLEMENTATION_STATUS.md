# Stato implementazione rispetto agli ADR

Verifica aggiornata al 2026-07-30. La tabella distingue ciò che è nel codice da
ciò che resta una decisione architetturale: “parziale” non significa che il
driver non funzioni, ma che non soddisfa ancora tutte le invarianti dell’ADR.

La preparazione della release è dichiarata come **RC del solo componente** in
[`assurance/RELEASE_CANDIDATE_SCOPE.md`](assurance/RELEASE_CANDIDATE_SCOPE.md).
Il manifest cita `plenora-contracts@v2.0-rc8` e la revisione esatta, distingue
la wire version dall'ICD e registra le sezioni candidate e la deroga di
emissione. Il gate della catena a tre componenti resta esplicitamente
`not_satisfied`.
La baseline tecnica candidata `3f3562a` è congelata con claim previsto
`verified_internally` per `v0.1.0-rc.3`. La CI candidata `30500304709` e la CI
della decisione `30501136176` sono verdi. La revisione indipendente resta
aperta come attributo non bloccante. RC2 resta l'ultima release pubblicata e
immutabile; RC3 ha superato la CI pre-tag `30501904391` sulla revisione
`ab330f8` ed è descritta come `component_rc_tagged` in
`release/rc3-development.json`. Il record finale diventa il target del tag
soltanto dopo la propria CI verde. Un claim
`verified_independently` resta vietato finché la review non viene completata.

Il programma component-only `0.1.0-rc.4` è ora aperto dalla baseline immutabile
`v0.1.0-rc.3` ed è registrato in `release/rc4-development.json`, senza claim di
nuova RC. Il reader XLSX bounded con spool temporaneo e la matrice nativa
Windows GDAL 3.10.3/OpenFileGDB sono implementati e hanno superato i rispettivi
benchmark. KML, DXF e pushdown OpenFileGDB restano subordinati ai prerequisiti
e ai veti già registrati.

Il profilo safety per un possibile impiego aeronautico è definito in
[`assurance/AERONAUTICAL_PROFILE.md`](assurance/AERONAUTICAL_PROFILE.md), con
requisiti, hazard, prove e gap nella
[`assurance/TRACEABILITY.md`](assurance/TRACEABILITY.md). È una baseline di
assurance, non una dichiarazione DO-178C/ED-12C. La CI vieta ora `unsafe` e le
primitive esplicite di panic in tutti i target di libreria.

| ADR | Stato corrente | Implementato | Gap principali |
|---|---|---|---|
| ADR-IO 1 — lifecycle e `WritePlan` | Parziale avanzato | Trait e ciclo `open`/reader, `create`/writer/`finish`; handle esplicitamente `Send + Sync`; writer multi-layer; nomi layer obbligatori e unici validati in comune; `SingleActiveReader` imposto con lease atomico ed errore tipizzato su KML/DXF/XLSX e backend FileGDB; matrice reale single/multiple inclusa la concorrenza FileGDB feature-on; CSV/GeoJSON/Shapefile/FileGDB usano un worker bounded comune con terminale esplicito, errori tipizzati e panic/disconnessione fail-closed invece di falso EOF; gate trasversale su piano vuoto/multi-layer/duplicati; ogni errore di write invalida il writer e vieta `finish`; abort senza publish né residui verificato sui writer pure-Rust e su FileGDB/GDAL; token R11 propagato da opzioni/richieste a probe, confini batch, write e pre-publish, con deadline, gerarchia e rilascio del lease alla cancellazione; KML/DXF/XLSX ricontrollano dopo le chiamate sincrone e ogni 1.024 elementi nei loop propri, con veto prestazionale pre/post superato | KML, DXF e XLSX materializzano durante `open`: una singola chiamata sincrona interna alla dipendenza non è interrompibile in modo preemptive e lo slicing successivo non libera i buffer |
| ADR-IO 2 — publish atomico | Parziale avanzato | Tempfile same-directory e no-clobber per file singoli; `StagedFile` centralizza nel core staging, destinazione, profilo durable, limite fisico e transizione terminale per CSV, GeoJSON, GeoParquet, IPC e GeoPackage; helper distinti coprono file, file con suffisso e directory; directory e loose set usano rename no-replace autorevoli su Linux/Android, Windows e macOS anche contro destinazioni apparse dopo il preflight (`renameat2`, move esclusiva Windows, `renameatx_np(RENAME_EXCL)`); job macOS dedicato per file/directory e safety lint; symlink e oggetti non regolari sono rifiutati in ogni profilo; GeoPackage multi-layer pubblicato come singolo file; Shapefile disponibile come `ShapefileDirectoryDataset` forte (`*.shp.d`, un rename) e come loose set compatibile (`*.shp`, companion ordinati); sequenza durable condivisa con `fsync` fail-closed dell'intero staging tree prima del rename ed esito post-rename tipizzato, incluso `PublishedButDurabilityUnconfirmed` su Windows dove il sync della directory padre non è portabile; preflight cross-filesystem esplicito su Unix e Windows con test reale `/dev/shm`; FileGDB chiude GDAL e ripulisce lo staging con guardia RAII, usa staging univoci con sidecar lockati, recupera solo gli orfani e verifica con crash reali di sottoprocesso che la destinazione sia assente prima del rename o completa dopo il rename | Il loose set resta deliberatamente non atomico in senso forte; su FreeBSD/NetBSD/OpenBSD e altri target senza una primitiva integrata il file singolo usa hard-link no-clobber, ma il publish di directory fallisce esplicitamente come `Unsupported`; manca FileGDB/GDAL nativo Windows e resta da confermare la durabilità su una matrice di filesystem reali |
| ADR-IO 3 — capability-check | Parziale avanzato | `FormatWriteCapabilities` machine-readable su tutti i driver scrivibili; policy nomi/tipi/attributi/geometria/CRS/nullability/multi-layer; `GeometryType` modella e serializza i 16 valori canonici R3.1; il codec lossless RC3 decodifica, ispeziona e ricodifica tutti i 15 tipi geometrici concreti, inclusi curve, superfici, TIN e Triangle, mentre `Unknown` resta uno stato dichiarativo non materializzabile; gli adattatori e i formati ristretti rifiutano esplicitamente i tipi estesi invece di linearizzarli; validatore statico prima della creazione e guardia runtime comune sui payload WKB; errori `CapabilityReason` tipizzati; matrice negativa derivata dai descrittori per limiti, nomi, tipi Arrow e geometrici, encoding, dimensioni, semantica e geometrie miste; test unitari diretti esercitano anche `AttributeWriteSupport::{None, NamedSubset}` e `NoNulls` | I descrittori dei driver restano limitati ai tipi che il formato può conservare; i vincoli topologici dipendenti dai valori restano nel primo `write`; il modello di coercion/report va raffinato; nessun driver reale dichiara ancora i tre rami capability verificati direttamente |
| ADR-IO 4 — CRS | Parziale avanzato trasversale | `CrsResolution::{Resolved, DeclaredButUnresolved, Missing}`, `RawCrs`, formato della definizione e `AxisOrder` esplicito; gli schemi Arrow emettono e rileggono `crs_id`, `crs_resolution`, `crs_definition`, `crs_definition_format` e `axis_order`; `OGC:CRS84` lon/lat distinto da `EPSG:4326` lat/lon; il bordo di lettura confronta sintatticamente `EPSG:<codice>` con `plenora.geometry.srid`, preserva entrambe le rappresentazioni discordanti e le dichiara come `inconsistent_crs_representations` nel `LossReport`; CSV/XLSX richiedono `assume_crs`; CRS fissi KML/GeoJSON validati; SHP/DXF/GPKG/GeoParquet conservano il raw e falliscono con `CRS_UNRESOLVED` redatto quando il metadato dichiarato non è risolvibile; IPC rappresenta il CRS assente come `Missing`; Shapefile, GeoPackage e DXF rifiutano ora anche l'invariante impossibile di un `ResolvedCrs` senza ID invece di etichettarlo `unknown`, `OGC:CRS84` o `DXF:GEODATA`; FileGDB feature-on preserva WKT/authority/axis tramite GDAL, supporta `assume_crs`, rifiuta coppie id/definizione incoerenti e non rietichetta CRS84; matrice test raw/axis e gate di scrittura `Embedded`/`Fixed` | L’authority/axis resolver pure-Rust resta intenzionalmente limitato agli identificativi e WKT riconosciuti; DXF non-WGS84 richiede la definizione WKT/XML completa per serializzare senza perdita |
| ADR-IO 5 — fedeltà | Parziale avanzato | `Fidelity` nel descrittore; `FidelityAssessment` bounded e serializzabile restituito da `open`/`create` e nel `Published`; motivi tipizzati per vincoli, attributi, coercion, nullability, struttura, precisione e metadati; il wrapper comune collega il contratto e promuove automaticamente l'esito a `Approximating` quando il `LossReport` osservato non è vuoto; un test trasversale verifica che tutti i cinque writer pure-Rust `Conditional` restino `Conditional` anche quando il report osservato è correttamente vuoto; DXF registra tassellazioni, esplosioni INSERT, conversioni di testo/solidi, entità non gestite e attributi non rappresentati; FileGDB è `Conditional` e fail-closed sul profilo GDAL 3.6 verificato (`Int32`/`Float64`/`Utf8`, WKB XY/XYZ/XYM/XYZM nelle famiglie native Point/Multi*), conserva feature con geometria nulla e pubblica tipo/dimensioni OGR più tipo/width/precision degli attributi nei metadati del contratto | I profili dei driver `Conditional` restano conservativi e vanno raffinati per singolo tipo/valore; un report vuoto significa “nessuna perdita osservata”, non `Lossless`; EWKB, temporali, booleani, binari e interi 64-bit FileGDB richiedono un modello nativo senza cambio di contratto e una matrice multi-versione GDAL; oracoli indipendenti e corpus reali non sono uniformi |
| ADR-IO 6 — projection e pruning | Parziale avanzato trasversale | Contratto `ReadRequest` e schema effettivo; capability machine-readable per projection, pruning numerico e spaziale, determinismo e tipi geometrici; projection esatta CSV, GeoJSON, Shapefile, FileGDB, GeoParquet, IPC e GeoPackage, inclusa projection vuota e geometria non forzata per richieste tabellari; `Required` fail-closed su DXF, KML e XLSX; CSV/GeoJSON/SHP/FileGDB saltano parsing, conversione e costruzione delle colonne escluse; `NumericComparison` GeoParquet precision-safe su statistiche min/max, con legacy `Opaque` fail-open; pruning spaziale GeoParquet e GeoPackage tramite RTree registrato/conforme; `target_bytes`/`max_rows` su tutti i reader e correzione adattiva condivisa sui byte Arrow osservati per CSV, GeoJSON, Shapefile, FileGDB e GeoPackage | Il pruning attributivo resta disponibile solo dove esistono statistiche a blocchi (GeoParquet): usare indici B-tree degli altri formati sarebbe filtering esatto; KML, DXF e XLSX restano non exact e lo slicing dei loro batch già materializzati non libera i buffer sottostanti |

## Contratti trasversali introdotti

- L’identità locale del modello è `plenora-io-model` e il suo errore pubblico è
  `PlenoraIoError`: non collidono più con il package `plenora-core` e il
  `PlenoraError` strutturalmente diverso di data-tools. Un gate CI rende
  irreversibile la correzione R8.1/R8.4. Il futuro crate condiviso
  `plenora-contracts` non viene anticipato finché §15.3 resta proposta.
- `PlenoraIoError` è ora un record serializzabile a quattro assi indipendenti:
  categoria, fase, effetto remoto e disposizione di retry. Il codice locale
  dettagliato resta separato, i messaggi di input esterno sono redatti e tutti
  i driver, incluso FileGDB feature-on, compilano contro il nuovo modello.
- Gli schemi prodotti dichiarano `plenora.contract.version=1`; i consumer
  accettano i contratti legacy senza versione e rifiutano versioni future.
  `types_declaration` distingue `exact`, `mixed` e `unresolved`, con invarianti
  fail-closed rispetto all’elenco dei tipi.
- `CancellationToken` è parte di `ReadOptions`, `WriteOptions` e `ReadRequest`;
  supporta richiesta, deadline e figli e viene osservato durante probe,
  lettura per batch, scrittura e prima del publish. I parser sincroni
  materializzanti restano cooperativi al confine della chiamata.
- Le action JavaScript della CI sono rinnovate a versioni Node 24 e tutte le
  action remote restano fissate a SHA immutabile; l’impatto è registrato nella
  CIA del 2026-07-28.
- Il contratto geometrico distingue `Wkb`/`Ewkb`, `Xy`/`Xyz`/`Xym`/`Xyzm`,
  `Geometry`/`Geography`, SRID, precisione, tipi geometrici e metadati nativi.
  Il core ora usa un solo codec autoritativo per decodificare e ricodificare
  WKB ISO ed EWKB senza perdita; l'API `geo-types` XY è un adattatore e non un
  secondo parser. Sono incluse
  dimensioni Z/M, SRID, endianess e geometrie annidate, con limiti e rifiuto dei
  byte residui. IPC conserva payload e contratto completi; GeoPackage conserva
  payload ISO WKB, SRID e flag nativi Z/M; GeoParquet conserva WKB dimensionali,
  emette `geometry_types` Z/M e calcola il bbox sulle sole coordinate XY;
  Shapefile converte direttamente le varianti Shape/ShapeM/ShapeZ in WKB
  XY/XYM/XYZ/XYZM per punti, multipunti, linee e poligoni, conserva il sentinel
  ESRI `NO_DATA` nelle misure e pubblica `shp.shape_type` nei metadati nativi;
  `Multipatch` resta fail-closed perché non ha una corrispondenza WKB univoca;
  GeoJSON conserva XY/XYZ direttamente tra coordinate JSON e WKB tramite un
  modulo geometrico isolato che costruisce i multipart da slice senza clonare
  le coordinate, rifiutando
  una quarta ordinata dalla semantica M ambigua e le geometrie prive di
  coordinate invece di inventare XY; KML conserva XY/XYZ direttamente
  tramite i tipi nativi con quota e rifiuta M, geometrie vuote e le
  `GeometryCollection` omogenee ambigue rispetto ai tipi `Multi*`; DXF usa ora direttamente l'AST WKB,
  conserva XY/XYZ e le trasformazioni 3D di primitive, OCS e blocchi INSERT,
  tassella in 3D SPLINE ed ELLIPSE inclinate e pubblica tipi geometrici esatti
  nel contratto. Poiché DXF non distingue formalmente una quota Z esplicita
  uguale a zero da una coordinata 2D, il dataset è dichiarato XYZ se almeno una
  quota è non nulla, altrimenti XY; M ed EWKB con SRID embedded sono rifiutati.
  FileGDB crea il feature class dal tipo e dalla dimensionalità del contratto,
  anche per layer vuoti o con sole geometrie nulle; il reader ricostruisce
  XY/XYZ/XYM/XYZM e il tipo dal geometry field OGR. Il profilo di scrittura verificato
  con GDAL 3.6.2 conserva esattamente `Int32`, `Float64`, `Utf8`, null
  attributivi, geometrie nulle e misure M anche nei payload multipart;
  i campi Arrow portano metadati namespaced per tipo OGR, larghezza e
  precisione, riapplicati e verificati durante la scrittura;
  `Int64`, EWKB, `GeometryCollection`, booleani, binari e temporali sono
  rifiutati invece di essere coerciti o scartati. `Date32` resta escluso perché
  il formato lo riapre come `DateTime`.
  Anche `LineString` e `Polygon` sono fail-closed perché il feature class li
  normalizza rispettivamente a `MultiLineString` e `MultiPolygon`.
  Una pre-validazione XML limitata protegge
  inoltre il parser KML da token malformati che possono impedirne l’avanzamento
  e da `Point` privi di coordinate, sui quali la dipendenza `kml 0.14.0`
  eseguirebbe una rimozione indicizzata panicking.
  Una guardia runtime comune decodifica i
  payload prima dei writer e rifiuta dimensioni, SRID, tipo o nullability diversi
  dal contratto, inclusi i vecchi contratti con il solo marker GeoArrow. Il
  parser dei metadati geometrici distingue ora l’assenza dal valore esplicito
  `unknown`, rifiuta valori non canonici senza mutare parzialmente il contratto
  e impedisce che più campi GeoArrow disattivino silenziosamente la guardia. CSV e
  XLSX convertono ora il WKT direttamente nell'AST WKB lossless e conservano
  XY/XYZ/XYM/XYZM, tipo geometrico e precisione `f64`; dichiarano la dimensione
  esatta per colonne omogenee e `Unknown` per colonne miste. Le colonne X/Y
  restano intenzionalmente Point XY; XLSX rifiuta coordinate non numeriche,
  non finite, incomplete o non rappresentabili esattamente come `f64`, invece
  di convertirle in geometria nulla. EWKB con SRID e semantica geography sono
  rifiutati perché non rappresentabili dal WKT semplice.
- CSV, GeoJSON e Shapefile condividono un'unica inferenza monotona e gli stessi
  builder Arrow per gli attributi. Un valore non nullo incompatibile con lo
  schema inferito produce errore invece di `null`; interi oltre `i64` e
  combinazioni intero/float non rappresentabili senza perdita sono conservati
  come testo.
- La terza campagna prestazionale elimina le allocazioni ripetute delle chiavi
  durante l'inferenza GeoJSON, riusa il buffer WKT del writer CSV e usa il
  visitor WKB bounded per il solo metadato dei tipi GeoParquet. Su 250.000
  Polygon e sette coppie A/B intercalate i delta mediani sono rispettivamente
  +6,59%, +12,59% e +48,64%, senza crescita della RSS. Baseline, veto e
  invarianti sono registrati nella CIA del 2026-07-29.
- `Limits` è parte di `ReadOptions` e `WriteOptions`. La dimensione complessiva
  dell’input è verificata prima del parser; righe e colonne sono conteggiate da
  un wrapper comune dei writer; i writer geometrici v1 usano i limiti WKB
  forniti dal chiamante. Per i cinque writer pure-Rust a file singolo,
  `StagedFile` conserva il limite insieme allo staging e lo verifica nella
  transizione terminale di publish. `max_vertices` restringe il numero effettivo di
  componenti WKB e `max_output_bytes` è verificato sullo staging prima del
  publish per file singoli, loose set Shapefile e directory FileGDB. Il reader
  DXF applica inoltre `max_rows`, `max_columns` e un budget cumulativo
  `max_vertices` durante la materializzazione e mantiene limiti separati su
  annidamento ed esplosione degli INSERT.
- Il catalogo espone capability di scrittura, concorrenza reader, projection e
  pruning con descrittori versionati; CSV e GeoJSON usano le versioni `6/6`,
  Shapefile `8/6` e FileGDB `9/8`, così il fingerprint distingue la nuova
  projection esatta dal profilo precedente.
- I comandi `inspect`, `layers`, `read` e `convert` espongono la valutazione di
  fedeltà concreta. `create` la rende interrogabile sul writer prima del primo
  batch; `finish` la aggiorna con le categorie realmente presenti nel
  `LossReport`. Le motivazioni sono deduplicate e limitate a 64 elementi.

## Copertura CI

Il report LCOV conserva l'intera workspace, inclusi CLI, benchmark e harness
fuzz. Il gate quantitativo è invece applicato al solo codice di libreria:
esclude esclusivamente gli entry point `main.rs` di `plenora-io-cli`,
`plenora-bench` e `plenora-fuzz`, che richiedono test end-to-end o campagne
dedicate e falsavano il dato delle librerie. La CI candidata immutabile
`3f3562a` supera il gate fail-closed fissato all'80%; l'artifact LCOV
`8743219769` e il relativo digest sono registrati nel bundle di evidenza.
Raccolta, pubblicazione
dell'artifact e verifica della soglia sono passi distinti, così un eventuale
calo resta diagnosticabile.

## Decisione sui fuzz test

Gli smoke fuzz restano utili a ogni modifica dei parser e del core e coprono ora
anche l'invariante decode/encode/decode lossless di WKB ISO ed EWKB, il
confronto differenziale di accettazione, tipo, dimensioni e SRID fra il decoder
autoritativo e il visitor WKB senza AST, e la
conversione dimensionale WKB ↔ Shape ESRI, oltre al round-trip WKT ↔ WKB di
CSV/XLSX e al parser/walker DXF → WKB XY/XYZ. I target libFuzzer
coverage-guided sono ora sei e il target DXF parte da un seed ASCII 3D minimo.
Una campagna breve ha individuato e chiuso l'accettazione di coordinate
WKT non finite prodotte da overflow numerico. La
precondizione geometrica e il gate trasversale negativo su capability, CRS di
scrittura e lifecycle dei writer pure-Rust sono ora presenti. Una campagna
lunga generale può quindi essere eseguita come gate della prossima milestone,
affiancata da campagne mirate sul core geometrico. Restano separati i test di
crash durante il publish: il fuzz non sostituisce questi test di contratto. La
suite FileGDB termina ora sottoprocessi durante write, prima del rename e subito
dopo il rename, e verifica recovery degli orfani e protezione dei writer attivi.
La matrice deterministica `RawCrs`/axis order e il test feature-on FileGDB sono
parte delle rispettive suite.

La campagna lunga WKB/EWKB non è partita come iniziativa isolata. Il protocollo
condiviso confronta il codec lossless IO con lo scanner database-tools usando
18 payload grezzi deduplicati per SHA-256, manifest comune e oracolo
differenziale. Il replay è verde con zero differenze non classificate; le due
differenze osservate sono registrate e motivate. Uno smoke bounded con seed
`20260728` ha eseguito 68.740.000 mutazioni in 60 secondi senza finding.
Corpus e invarianti sono destinati al repository `plenora-contracts`. La
campagna lunga su harness committati ha eseguito 210.118.046 esecuzioni
libFuzzer e 5.705.840.000 iterazioni strutturate in un'ora, con zero finding e
working tree pulito prima e dopo. Restano aperte la retention/promozione del
corpus condiviso e le campagne future su nuove modifiche.
