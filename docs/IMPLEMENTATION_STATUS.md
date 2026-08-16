# Stato implementazione rispetto agli ADR

Verifica aggiornata al 2026-08-03. La tabella distingue ciò che è nel codice da
ciò che resta una decisione architetturale: “parziale” non significa che il
driver non funzioni, ma che non soddisfa ancora tutte le invarianti dell’ADR.

La preparazione della release è dichiarata come **RC del solo componente** in
[`assurance/RELEASE_CANDIDATE_SCOPE.md`](assurance/RELEASE_CANDIDATE_SCOPE.md).
Il manifest cita `plenora-contracts@v2.0-rc14` e la revisione esatta, distingue
la wire version dall'ICD e registra le sezioni candidate e la deroga di
emissione. Il gate della catena a tre componenti resta esplicitamente
`not_satisfied`.
RC4 è pubblicata e immutabile al tag `v0.1.0-rc.4`, target `8d3f25f`. La CI
finale `30607124206` e la CI del tag `30607373426` sono verdi sui quattro job.
Il claim resta `verified_internally`; review indipendente, RC di sistema e
certificazione avionica non sono dichiarate.

RC4 comprende reader bounded XLSX/KML/DXF, pushdown OpenFileGDB tramite fork
governato `gdal 0.17.1`, matrice nativa Windows GDAL 3.10.3/OpenFileGDB e le
correzioni Shapefile su WKT proiettato e precisione DBF. Tutti i workstream
tecnici hanno superato i rispettivi test e benchmark. La revisione indipendente
resta un attributo non bloccante; un claim `verified_independently` resta
vietato finché la review non viene completata.

Lo sviluppo `0.1.0-rc.5` partiva dalla baseline immutabile RC4. Il primo
incremento rende machine-readable i quattro assi della busta d'errore CLI e
conserva `delay_ms` nella variante retry `after`; non modifica driver o formati
su disco. Il secondo espone separatamente `read_loss` e `write_loss` nel
documento `convert` e aggiunge `conversion_fidelity`, eliminando l'ambiguo
campo writer-only `loss.lossless`. La superficie candidata alla compatibilità
1.x è limitata alle sei buste JSON della CLI; le API Rust restano interne.
La baseline immutabile `v1.0.0-rc.1`, target `6e3a942`, conserva la propria
qualifica esterna `83/84` roundtrip e `27/28` nella catena. La candidata
`1.0.0-rc.2` è congelata sulla revisione `63a8253`, con CI candidata
`30625336681` e decisione di release `2d5d606`/`30627985036` verdi. Il delta
implementa R4.1.1 contro `v2.0-rc14`; la qualifica della RC.1 non si trasferisce
alla RC.2. La revisione pre-tag `40212ad` ha CI `30632956628` verde. Il tag
annotato `v1.0.0-rc.2` punta a `9804d77`; la CI finale `30633367104` e quella
del tag `30633636716` sono verdi. La matrice esterna dovrà essere rieseguita
sul nuovo tag immutabile.
Il ramo `main` successivo alla RC.2 implementa R2.8, il confronto conservativo
fra EPSG radice di WKT/PROJJSON, `crs_id` e SRID, e i budget condivisi
R7.5-R7.7. Il reader Shapefile conserva inoltre come `Int64` i campi DBF
`N(width>=10,0)`, rifiuta i nomi duplicati prima del collasso nella mappa e
non ripete la scansione geometrica durante `open` per i tipi XY/M. Questi
incrementi non appartengono al tag RC.2 e richiedono una
nuova baseline immutabile prima di diventare evidenza di release. Il gate di
sistema resta separato e non soddisfatto.

La metadata candidate `1.0.0` usa come evidence base la revisione
`938dab99567fffde6510bb3c3e5e944e6bff42df`, qualificata same-SHA nel run
`30692495395`. Il diff di versione e manifesti richiede una nuova CI sulla
propria revisione prima di qualsiasi tag. Il tag `v1.0.0` non esiste e i crate
restano `publish = false`; la compatibilità 1.x resta limitata alla CLI JSON.
Le evidenze e i record `v1.0.0-rc.2` non vengono riattribuiti.

Il worktree post-candidato adotta `plenora-row-diagnostics-v1` per i rifiuti
row-scoped e impone la validazione runtime sui confini read/write di tutti i
driver. Le geometrie non valide non entrano nei batch;
dopo il primo rifiuto il parser continua soltanto per ottenere conteggi completi
ed esempi bounded, conservando l'indice assoluto zero-based solo quando la
provenance è attestabile. La CLI aggiunge `error.row_diagnostics` solo quando
applicabile, mappa i rifiuti `DataMapping` normativi a exit 2 e la cancellazione
del chiamante a exit 130. Chiave e valore sono assenti
per default e richiedono campo e policy `emit`/`redact` espliciti. Questo
incremento non appartiene alle evidenze congelate precedenti e richiede suite,
review, CI same-SHA e qualifica cross-component prima di una release.

Il profilo safety per un possibile impiego aeronautico è definito in
[`assurance/AERONAUTICAL_PROFILE.md`](assurance/AERONAUTICAL_PROFILE.md), con
requisiti, hazard, prove e gap nella
[`assurance/TRACEABILITY.md`](assurance/TRACEABILITY.md). È una baseline di
assurance, non una dichiarazione DO-178C/ED-12C. La CI vieta ora `unsafe` e le
primitive esplicite di panic in tutti i target di libreria.

| ADR | Stato corrente | Implementato | Gap principali |
|---|---|---|---|
| ADR-IO 1 — lifecycle e `WritePlan` | Parziale avanzato | Trait e ciclo `open`/reader, `create`/writer/`finish`; handle esplicitamente `Send + Sync`; writer multi-layer; nomi layer obbligatori e unici validati in comune; `SingleActiveReader` imposto con lease atomico ed errore tipizzato su KML/DXF/XLSX e backend FileGDB; matrice reale single/multiple inclusa la concorrenza FileGDB feature-on; CSV/GeoJSON/Shapefile/FileGDB usano un worker bounded comune con terminale esplicito, errori tipizzati e panic/disconnessione fail-closed invece di falso EOF; gate trasversale su piano vuoto/multi-layer/duplicati; ogni errore di write invalida il writer e vieta `finish`; abort senza publish né residui verificato sui writer pure-Rust e su FileGDB/GDAL; token R11 propagato da opzioni/richieste a probe, confini batch, write e pre-publish, con deadline, gerarchia e rilascio del lease alla cancellazione; KML usa parsing event-based e spool bounded, DXF un reader progressivo con spill bounded e XLSX l'API lazy con spool; i tre percorsi hanno superato il veto prestazionale | L'inferenza completa di schema/contratto richiede ancora una scansione durante `open` per KML, DXF e XLSX; la cancellazione è cooperativa fra eventi/celle/entità, non preemptive dentro una singola chiamata della dipendenza |
| ADR-IO 2 — publish atomico | Parziale avanzato | Tempfile same-directory e no-clobber per file singoli; `StagedFile` centralizza nel core staging, destinazione, profilo durable, limite fisico e transizione terminale per CSV, GeoJSON, GeoParquet, IPC e GeoPackage; helper distinti coprono file, file con suffisso e directory; directory e loose set usano rename no-replace autorevoli su Linux/Android, Windows e macOS anche contro destinazioni apparse dopo il preflight (`renameat2`, move esclusiva Windows, `renameatx_np(RENAME_EXCL)`); job macOS dedicato per file/directory e safety lint; symlink e oggetti non regolari sono rifiutati in ogni profilo; GeoPackage multi-layer pubblicato come singolo file; Shapefile disponibile come `ShapefileDirectoryDataset` forte (`*.shp.d`, un rename) e come loose set compatibile (`*.shp`, companion ordinati); sequenza durable condivisa con `fsync` fail-closed dell'intero staging tree prima del rename ed esito post-rename tipizzato, incluso `PublishedButDurabilityUnconfirmed` su Windows dove il sync della directory padre non è portabile; preflight cross-filesystem esplicito su Unix e Windows con test reale `/dev/shm`; FileGDB chiude GDAL e ripulisce lo staging con guardia RAII, usa staging univoci con sidecar lockati, recupera solo gli orfani e verifica con crash reali di sottoprocesso che la destinazione sia assente prima del rename o completa dopo il rename | Il loose set resta deliberatamente non atomico in senso forte; su FreeBSD/NetBSD/OpenBSD e altri target senza una primitiva integrata il file singolo usa hard-link no-clobber, ma il publish di directory fallisce esplicitamente come `Unsupported`; manca FileGDB/GDAL nativo Windows e resta da confermare la durabilità su una matrice di filesystem reali |
| ADR-IO 3 — capability-check | Parziale avanzato | `FormatWriteCapabilities` machine-readable su tutti i driver scrivibili; policy nomi/tipi/attributi/geometria/CRS/nullability/multi-layer; `GeometryType` modella e serializza i 16 valori canonici R3.1; il codec lossless RC3 decodifica, ispeziona e ricodifica tutti i 15 tipi geometrici concreti, inclusi curve, superfici, TIN e Triangle, mentre `Unknown` resta uno stato dichiarativo non materializzabile; gli adattatori e i formati ristretti rifiutano esplicitamente i tipi estesi invece di linearizzarli; validatore statico prima della creazione e guardia runtime comune sui payload WKB; errori `CapabilityReason` tipizzati; matrice negativa derivata dai descrittori per limiti, nomi, tipi Arrow e geometrici, encoding, dimensioni, semantica e geometrie miste; test unitari diretti esercitano anche `AttributeWriteSupport::{None, NamedSubset}` e `NoNulls` | I descrittori dei driver restano limitati ai tipi che il formato può conservare; i vincoli topologici dipendenti dai valori restano nel primo `write`; il modello di coercion/report va raffinato; nessun driver reale dichiara ancora i tre rami capability verificati direttamente |
| ADR-IO 4 — CRS | Parziale avanzato trasversale | `CrsResolution::{Resolved, DeclaredButUnresolved, Missing}`, `RawCrs`, formato della definizione e `AxisOrder` esplicito; R4.1.1 accetta ora `declared_unresolved` con il solo `crs_id`, senza inventare una definizione; gli schemi Arrow emettono e rileggono `crs_id`, `crs_resolution`, `crs_definition`, `crs_definition_format` e `axis_order`; `OGC:CRS84` lon/lat distinto da `EPSG:4326` lat/lon; il bordo di lettura confronta sintatticamente `EPSG:<codice>` con `plenora.geometry.srid`, preserva entrambe le rappresentazioni discordanti e le dichiara come `inconsistent_crs_representations` nel `LossReport`; CSV/XLSX richiedono `assume_crs`; CRS fissi KML/GeoJSON validati; SHP/DXF/GPKG/GeoParquet conservano il raw e falliscono con `CRS_UNRESOLVED` redatto quando il metadato dichiarato non è risolvibile; IPC rappresenta il CRS assente come `Missing`; Shapefile, GeoPackage e DXF rifiutano ora anche l'invariante impossibile di un `ResolvedCrs` senza ID invece di etichettarlo `unknown`, `OGC:CRS84` o `DXF:GEODATA`; FileGDB feature-on preserva WKT/authority/axis tramite GDAL, supporta `assume_crs`, rifiuta coppie id/definizione incoerenti e non rietichetta CRS84; matrice test raw/axis e gate di scrittura `Embedded`/`Fixed` | L’authority/axis resolver pure-Rust resta intenzionalmente limitato agli identificativi e WKT riconosciuti; DXF non-WGS84 richiede la definizione WKT/XML completa per serializzare senza perdita |
| ADR-IO 5 — fedeltà | Parziale avanzato | `Fidelity` nel descrittore; `FidelityAssessment` bounded e serializzabile restituito da `open`/`create` e nel `Published`; motivi tipizzati per vincoli, attributi, coercion, nullability, struttura, precisione e metadati; il wrapper comune collega il contratto e promuove automaticamente l'esito a `Approximating` quando il `LossReport` osservato non è vuoto; un test trasversale verifica che tutti i cinque writer pure-Rust `Conditional` restino `Conditional` anche quando il report osservato è correttamente vuoto; Shapefile legge dal testo DBF originale gli interi `N(width>=10,0)` e rifiuta descrittori omonimi invece di perdere precisione o colonne; DXF registra tassellazioni, esplosioni INSERT, conversioni di testo/solidi, entità non gestite e attributi non rappresentati; FileGDB è `Conditional` e fail-closed sul profilo GDAL 3.6 verificato (`Int32`/`Float64`/`Utf8`, WKB XY/XYZ/XYM/XYZM nelle famiglie native Point/Multi*), conserva feature con geometria nulla e pubblica tipo/dimensioni OGR più tipo/width/precision degli attributi nei metadati del contratto | I profili dei driver `Conditional` restano conservativi e vanno raffinati per singolo tipo/valore; un report vuoto significa “nessuna perdita osservata”, non `Lossless`; il writer DBF basato su `Numeric(f64)` non ha ancora una prova di conservazione degli `Int64` oltre `2^53`; EWKB, temporali, booleani, binari e interi 64-bit FileGDB richiedono un modello nativo senza cambio di contratto e una matrice multi-versione GDAL; oracoli indipendenti e corpus reali non sono uniformi |
| ADR-IO 6 — projection e pruning | Parziale avanzato trasversale | Contratto `ReadRequest` e schema effettivo; capability machine-readable per projection, pruning numerico e spaziale, determinismo e tipi geometrici; projection esatta CSV, GeoJSON, Shapefile, FileGDB, GeoParquet, IPC e GeoPackage, inclusa projection vuota e geometria non forzata per richieste tabellari; `Required` fail-closed su DXF, KML e XLSX; CSV/GeoJSON/SHP/FileGDB saltano parsing, conversione e costruzione delle colonne escluse; FileGDB applica inoltre il pushdown fisico degli attributi e della geometria esclusi tramite `OGR_L_SetIgnoredFields` nel fork governato `gdal 0.17.1`; `NumericComparison` GeoParquet precision-safe su statistiche min/max, con legacy `Opaque` fail-open; pruning spaziale GeoParquet e GeoPackage tramite RTree registrato/conforme; `target_bytes`/`max_rows` su tutti i reader e correzione adattiva condivisa sui byte Arrow osservati per CSV, GeoJSON, Shapefile, FileGDB e GeoPackage | Il pruning attributivo resta disponibile solo dove esistono statistiche a blocchi (GeoParquet): usare indici B-tree degli altri formati sarebbe filtering esatto; KML, DXF e XLSX restano non exact e non applicano pushdown fisico |
| ADR-IO 7 — streaming vs operation-atomicity | Parziale avanzato | **Stato corrente.** Operation-atomicity conservata (opzione A, ratificata il 2026-08-16). L'accumulo in RAM e' sostituito dallo `StagedSpool` (`plenora-io-core::driver::spool`): i batch verificati restano in RAM sotto una soglia derivata dalla capacita' **effettiva** — minimo fra quota locale e pool — e oltre quella migrano su un file temporaneo senza nome in Arrow IPC, da cui non tornano. Il picco e' `soglia + batch corrente`, indipendente dalla dimensione dell'input. La memoria di ogni batch e' una `InternalMemoryLease` viva: l'adapter prenota largo, misura, riduce con `shrink_to` all'ingombro reale piu' l'overhead strutturale, e **sposta la stessa lease** nello spool: fra materializzazione e custodia non c'e' un istante in cui il batch sia in RAM senza che nessuno lo conti. La quota di spill segue i byte realmente scritti, con lease RAII restituite alla chiusura del file. Il preflight enumera la sorgente addebitando ogni voce al context e pubblica il footprint spendendo un permit one-shot; il modello budget e' unico, senza rami legacy. **La cronaca dei dodici sottopassi — decisioni, difetti trovati, mutazioni — e' nella CIA `CHANGE_IMPACT_2026-08-16_LOTTO_0_BUDGET_MODEL.md`, non qui: questa colonna descrive cio' che e', non come ci si e' arrivati.** | I tre campi descriptor (`native_read_mode`, `effective_delivery`, `buffering`) sono dichiarati solo in S8, quindi il wire `catalog` non riflette ancora la semantica reale del bordo. |

Lo sviluppo RC5 aggiunge alla capability di scrittura CRS i tre stati
`preserved`, `absent` e `derived` per `crs_id`, `srid` e `crs_definition`.
Il preflight candidato R4.6.5 consente una coppia `crs_id`/`srid` discordante
soltanto quando entrambe le rappresentazioni sono preservate
indipendentemente: IPC passa, Shapefile fallisce chiuso durante `Validate`.
Il `LossReport` writer distingue inoltre rappresentazione e stato invece della
precedente categoria CRS generica. R4.1.1 è implementata contro
`plenora-contracts v2.0-rc14`: `RawCrs::definition` e il relativo formato sono
opzionali e il solo identificatore dichiarato mantiene lo stato distinto da
`Missing`. Sul ramo successivo alla RC.2, R2.8 riconosce anche le sole chiavi
canoniche e rifiuta metadati incompleti o un'estensione esterna discordante.

## Incrementi post-RC.2

- Shapefile/DBF: `N(width>=10,0)` è letto lessicalmente come `Int64`; collisioni
  dei nomi sono rifiutate prima della `HashMap`; per XY/M il contratto usa il
  tipo dell'header e la geometria viene decodificata una sola volta nel reader.
  Impatto, test e diagnosi del bind mount sono registrati nella CIA dedicata.
- R2.8: IPC riconosce una geometria dalle sole chiavi canoniche; metadati
  canonici incompleti, versione schema assente e un'eventuale estensione
  discordante falliscono chiuso.
- R4.3.1/R4.6: il confronto comprende l'EPSG dichiarato alla radice di WKT e
  PROJJSON, senza confonderlo con gli EPSG dei CRS base annidati. Il bordo di
  lettura preserva e dichiara; il preflight writer applica la capability a tre
  stati. Definizioni senza identificatore EPSG radice restano fuori dal
  resolver conservativo e non vengono interpretate per supposizione.
- R7.5-R7.7: il budget e' un `PipelineContext` unico per operazione, con
  `OperationBudget` per i contatori cumulativi — righe, colonne, componenti
  geometrici, byte di uscita — e lease RAII per le occupazioni: memoria,
  spill, concorrenza. Le prime sono consumo definitivo, le seconde tornano al
  gauge quando l'oggetto che le tratteneva muore, e la distinzione e' nei tipi
  (`CountedLease` contro `InternalMemoryLease`/`SpillLease`). Reader e writer
  di una stessa conversione condividono il context e tengono contatori
  distinti: una riga non consuma due volte la stessa quota, ma memoria e
  deadline sono contate una volta sola. Quote per singola geometria — cella,
  profondita', componenti — restano nei limiti della pipeline e non nei
  contatori. La CPU è governata tramite durata, non tramite un contatore CPU.
- KML, DXF e XLSX effettuano una sola scansione della sorgente durante `open` e
  riusano lo spool bounded nel reader. Ridurla ulteriormente richiederebbe un
  contratto/schema fornito dal chiamante: senza quello, fermarsi prima della
  fine renderebbe l'inferenza dipendente dal campione.
- FileGDB `Int64` è stato provato e non esposto: GDAL/OpenFileGDB 3.6.2 ha
  riaperto `OFTInteger64` come `OFTReal`, quindi il round-trip oltre 2^53 non è
  simmetrico. La capability resta fail-closed su `Int32`/`Float64`/`Utf8`.
- La CI aggiunge una matrice Linux su Ubuntu 22.04 e 24.04, registrando versione
  GDAL e filesystem e ripetendo FileGDB e publish su filesystem runner/tmpfs.
  Il claim multi-ambiente nasce solo dopo l'esecuzione verde dei job.

## Contratti trasversali introdotti

- L’identità locale del modello è `plenora-io-model` e il suo errore pubblico è
  `PlenoraIoError`: non collidono più con il package `plenora-core` e il
  `PlenoraError` strutturalmente diverso di data-tools. Un gate CI rende
  irreversibile la correzione R8.1/R8.4. Il futuro crate condiviso
  `plenora-contracts` non viene anticipato finché §15.3 resta proposta.
- `PlenoraIoError` è un record serializzabile a quattro assi indipendenti:
  categoria, fase, effetto remoto e disposizione di retry. La CLI pubblica gli
  assi come campi `snake_case`; `retry` è un oggetto taggato e `After` conserva
  `delay_ms`. Il codice locale resta separato e `message` è soltanto testo
  diagnostico redatto.
- `PlenoraIoError` può includere `row_diagnostics` conforme a
  `plenora-row-diagnostics-v1`. Il report è opzionale, bounded, usa indici
  sorgente zero-based e resta separato dagli assi dell'errore e dagli effetti
  remoti. Il reader Shapefile produce la diagnostica nativa completa; il bordo
  comune valida schema, nullability e geometria di tutti i reader e writer.
  Gli indici sono emessi soltanto per i percorsi che attestano una relazione
  fisica uno-a-uno; pruning, righe deleted ed espansioni DXF falliscono senza
  inventare provenance. La qualifica cross-component resta aperta.
  GeoPackage non attesta indici fisici generici: l'ordinale Arrow non coincide
  necessariamente con `rowid` quando esistono gap. La CLI determina
  `input_total` esatto a EOF per un layer alla volta, lo dichiara prima della
  relativa scrittura e non accumula tutti i layer; EOF corto o righe extra
  impediscono `finish` e publish. Un'interruzione terminale conserva categoria,
  codice, fase e retry originali, aggiungendo soltanto diagnostica partial.
  FileGDB/GDAL resta pending per una qualifica live su dataset e runtime reali:
  i test feature-gated non sono presentati come evidenza live.
  Il routing CLI `.xls` e' stato rimosso come capability drop esplicita: il
  driver supporta esclusivamente `.xlsx`, non il contenitore binario BIFF.
- `read --limit` usa la scope typed `ReadScope::AcceptedRows`: arresto reale
  dopo il batch che raggiunge la soglia, overshoot per batch invariato e
  diagnostica non completa sullo stop volontario. `convert` usa
  `ReadScope::Complete` e continua a validare fino a EOF prima del publish.
- `plenora-io-convert-v1` separa i report osservati in `read_loss` e
  `write_loss`; `conversion_fidelity` combina i due assessment senza presentare
  il solo esito del writer come giudizio sull'intera conversione.
- La capability CRS del writer distingue conservazione indipendente, assenza e
  derivazione. Le rappresentazioni non conservate producono categorie
  machine-readable specifiche nel `LossReport`.
- Gli schemi prodotti dichiarano `plenora.contract.version=1`; i consumer
  accettano i contratti legacy senza versione e rifiutano versioni future.
  `types_declaration` distingue `exact`, `mixed` e `unresolved`, con invarianti
  fail-closed rispetto all’elenco dei tipi.
- `CancellationToken` è parte di `ReadOptions`, `WriteOptions` e `ReadRequest`;
  supporta richiesta, deadline e figli e viene osservato durante probe,
  lettura per batch, scrittura e prima del publish. I percorsi KML, DXF e XLSX
  controllano inoltre il token durante la scansione bounded; le singole
  chiamate delle dipendenze restano cooperative e non preemptive.
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
- Le quote vivono in `PipelineLimits`, dentro il `PipelineContext` che le
  opzioni trasportano; `ReadOptions` e `WriteOptions` non hanno `Default`,
  perche' un budget nasce da una costruzione che puo' fallire. La dimensione
  complessiva dell’input è verificata dal preflight, che enumera la sorgente
  addebitando ogni voce al context e pubblica il footprint spendendo un
  permit one-shot; righe e colonne sono conteggiate da un wrapper comune dei
  writer; i writer geometrici v1 usano i limiti WKB
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
  Shapefile `9/7` e FileGDB `10/8`, così il fingerprint distingue la nuova
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
coverage-guided sono ora tredici e il target DXF parte da un seed ASCII 3D
minimo.
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

La seconda ondata di target chiude l'asimmetria fra il perimetro fuzzato e il
perimetro realmente esposto a file esterni. Fino a questo punto erano coperti
solo i formati testuali e le conversioni geometriche in memoria; i formati
contenitore — GeoPackage (SQLite), GeoParquet, XLSX (ZIP), Arrow IPC — e i
tabellari CSV entravano in produzione senza alcuna copertura coverage-guided,
pur essendo la superficie dove un file ostile controlla i *metadati* e non solo
i dati. I sei target di lettura aggiunti (`gpkg_geometry`, `gpkg_reader`,
`geoparquet_reader`, `ipc_reader`, `csv_reader`, `xlsx_reader`) esercitano
apertura, contratto e drenaggio completo attraverso l'API pubblica
`FormatDriver`, con l'input materializzato in una directory temporanea
esclusiva: gli eventuali sidecar (`-wal`, `-shm`) non sopravvivono
all'esecuzione. Ogni target ha semi versionati in `fuzz/seeds/`, generati dai
writer del repo stesso, perché senza un contenitore valido un input casuale non
supererebbe il controllo del magic e il target non coprirebbe nulla.

Un settimo target, `ipc_to_gpkg`, copre il percorso di **scrittura**. Arrow IPC
è pass-through, quindi è l'unico ingresso offline che porta al writer un
contratto interamente controllato dall'input — tipi, nullabilità, nomi di
colonna e di layer, metadati di CRS. Il target replica il `convert` della CLI e
raggiunge così la validazione di capability, la coercizione Arrow → SQL, il DDL
della feature table, la registrazione dell'SRS e il publish atomico, che i test
esercitano solo con contratti sintetizzati a mano. Restano scoperti gli altri
writer (shapefile, GeoParquet, XLSX, CSV, KML, DXF) e l'intero tier FileGDB, che
richiede GDAL e non è raggiungibile senza il backend nativo.

La campagna non è più solo locale. `scripts/fuzz-smoke.sh` costruisce ed esegue
tutti i target sotto AddressSanitizer e la CI lo invoca in un job dedicato
(`fuzz` in `.github/workflows/ci.yml`), l'unico del repository che compila con
un sanitizer. È la contromisura al fatto che i driver trascinano dipendenze C
(SQLite bundled, zlib, zstd, bzip2) fuori dal perimetro di `unsafe_code =
"forbid"`: lì un difetto di memoria non produce un errore di tipo, produce un
risultato sbagliato. La lista dei target è derivata da `cargo fuzz list`, quindi
un target nuovo entra nello smoke senza toccare né lo script né la CI. Il budget
è di 60 secondi per target sui push e 300 sulla finestra settimanale; la
persistenza del corpus fra esecuzioni CI resta aperta.

**Sezione storica (2026-07/08).** La prima campagna sui target nuovi ha prodotto cinque finding, tutti nel giro di
secondi e tutti sul percorso di lettura di file esterni. Tre sono panic in
dipendenze: `arrow-ipc` `convert.rs` panica su un valore di enum sconosciuto nel
FlatBuffer dello schema (precisione `FloatingPoint`, riga 354, e `Type::NONE`,
riga 514) e `arrow-buffer` `immutable.rs:288` panica su uno slice con offset
oltre la lunghezza del buffer. Sono raggiungibili da un `.parquet` (i metadati
`ARROW:schema` sono un messaggio IPC incorporato) e da un `.arrow`: un file
ostile termina il processo invece di produrre un `PlenoraIoError`. Gli altri due
sono di risorsa: 32 KiB di GeoPackage portano il reader oltre 2 GiB residenti, e
5,4 KiB di XLSX superano i 15 secondi per una singola lettura — le quote di
allora non li intercettavano.

I due finding di risorsa vanno **rivalutati** contro il modello unificato: da
S4.d la memoria e' governata da un `PipelineContext` con soglia di migrazione
e spool su disco, quindi un reader non dovrebbe piu' poter crescere senza
tetto. La rivalutazione non e' stata eseguita — il container di sviluppo non
ha nightly ne' `cargo-fuzz` — e resta fra i gate non misurabili qui.

Nessuno dei cinque è correggibile senza decidere cosa il driver deve accettare o
rifiutare, quindi restano aperti e i target corrispondenti sono elencati in
`fuzz/quarantine.txt`: compilano sotto sanitizer in CI ma non vengono eseguiti
nello smoke, perché un gate che fallisce sempre smette di essere letto e copre le
regressioni nuove. `scripts/fuzz-smoke.sh --include-quarantined` e la campagna
lunga li eseguono comunque.
