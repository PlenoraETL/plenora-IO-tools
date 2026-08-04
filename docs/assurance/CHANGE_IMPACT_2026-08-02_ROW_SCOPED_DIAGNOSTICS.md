# Change impact analysis — diagnostica row-scoped bounded

Data: 2026-08-02.

## Baseline e perimetro

Lo sviluppo parte da `main` `509f14e85958372cbf9ab9e25eff9ea2d9457ff0`,
candidato congelato per il successivo incremento di Plenora IO Tools.
La sorgente normativa del nuovo payload è Plenora Contracts rc17, revisione
`5dc445ee3e6a6af277d4e2685afd1bc969fbe664`; i manifesti delle release
storiche restano immutati e non ricevono retroattivamente questo claim.

Il cambiamento riguarda il modello d'errore interno, tutti i confini pubblici
read/write dei driver, il reader Shapefile e la busta JSON d'errore della CLI.
Non modifica formati su disco, retry, pubblicazione atomica o tassonomie chiuse
già esistenti; rende però fail-closed payload Arrow che divergevano dal
contratto dichiarato.

## Problema

Il reader Shapefile rifiutava correttamente una geometria non rappresentabile,
ma interrompeva la scansione al primo record e pubblicava soltanto prosa. Il
chiamante non poteva conoscere indice sorgente, causa stabile, cardinalità per
causa o esempi bounded; una chiave DBF configurata non era disponibile nel
report.

## Decisione

`PlenoraIoError` espone il campo opzionale `row_diagnostics`, conforme a
`plenora-row-diagnostics-v1`. Il campo resta assente per gli errori non
row-scoped, preservando la forma prodotta dal protocollo v1 esistente.

Il reader Shapefile:

- usa l'ordinale zero-based del record sorgente, indipendente dai batch;
- sincronizza geometrie, marker DBF fisici e record DBF attivi separatamente:
  un record DBF deleted avanza l'indice e la geometria ma non rinumera le righe
  successive e non può disallineare attributi e geometrie;
- valida la geometria anche nelle projection attribute-only e classifica come
  cause machine-readable namespaced mismatch di tipo, conversione ed encoding;
- classifica anelli non chiusi, senza outer e degenerati senza affidarsi alla
  sola conversione WKB;
- tratta parse/decode/conversion failure degli attributi come rifiuti row-scoped;
  un'incoerenza tra i due pass che impedisce di continuare produce invece un
  report `partial` con limite di conoscenza esplicito;
- legge le chiavi DBF `Numeric`/`Float` dal lessicale ASCII del record fisico,
  senza ricostruirle dal `f64` della dipendenza;
- continua la scansione dopo il primo rifiuto soltanto per completare conteggi
  ed esempi, senza emettere altri batch;
- controlla cancellation/deadline a ogni record e la presenza del consumer ogni
  1024 record tramite heartbeat non bloccante;
- confronta la cardinalità attiva osservata nei due pass; se il dataset cambia
  o la scansione si interrompe, conserva i rifiuti già osservati ma dichiara
  `completeness=partial`, omette `total` e pubblica `knowledge_limits`;
- non inserisce mai record non validi nei batch accettati;
- limita gli esempi a un massimo configurabile di 64 mantenendo completi i
  conteggi;
- emette una chiave soltanto quando `row_diagnostics.key_field` e una policy
  esplicita `emit` o `redact` sono configurati;
- serializza i valori chiave come stringhe, evitando perdita numerica oltre
  2^53.

Non viene introdotta remediation implicita né modalità tolerant.

## Estensione trasversale ai driver

Il bordo comune di scrittura, attraversato da CSV, GeoJSON, KML, DXF,
Shapefile, GeoPackage, FileGDB, IPC, GeoParquet e XLSX:

- richiede uguaglianza dello schema runtime col `WritePlan`;
- valida nullability e ogni payload geometrico prima di invocare il backend;
- raccoglie tutti i rifiuti osservabili del batch, con indici globali
  zero-based, conteggi completi ed esempi limitati a 64;
- espone una partizione write machine-readable e invalida il writer al primo
  errore, impedendo `finish` e publish.

Il bordo comune di lettura verifica lo schema effettivo, nullability, WKB,
dimensioni, SRID/encoding e tipi geometrici prima di restituire un batch. CSV,
GeoJSON, KML, IPC e XLSX attestano l'ordinale fisico uno-a-uno; GeoParquet,
GeoPackage, FileGDB, Shapefile con record deleted e DXF con
espansioni non emettono un indice generico non dimostrabile. Shapefile mantiene
la propria diagnostica fisica dedicata. Quando l'identita' fisica non e'
attestabile, cardinalita' e cause osservate restano nei conteggi del report
`unknown`, mentre gli esempi restano vuoti: la perdita di provenance non
autorizza la perdita di osservazioni. Il drain fail-closed non espone prefissi
accepted prima di conoscere l'esito terminale e ricontrolla la cancellazione
prima di consegnare ogni batch gia' validato e buffered.

`declare_input_total` viene propagato fino ai writer specifici ed e' la
cardinalita' esatta, non un limite superiore. La CLI bufferizza un solo layer
fino a EOF sotto il `ResourceBudget`, dichiara il totale prima del primo write
di quel layer, scrive e libera il buffer prima di passare al successivo. Il
wrapper rifiuta sia righe extra sia EOF anticipato prima di `inner.finish`,
quindi non pubblica una partizione incompleta. In particolare DXF usa il totale
dichiarato per la partizione rc17 dei propri rifiuti geometrici specifici;
senza un totale dichiarato il writer fallisce come precondizione di contratto e
non emette un rifiuto row-scoped privo di payload.

Sono inoltre chiusi drop preesistenti: GeoJSON rifiuta chiavi duplicate, incluse
quelle sconosciute o fuori projection, tramite indici bounded dello schema
sorgente invece di applicare first-value-wins; DXF rifiuta geometrie nulle,
degeneri, ACIS,
tipi non supportati, blocchi mancanti e Polygon con anelli interni; KML rifiuta
Model/Track/MultiTrack non rappresentabili; FileGDB distingue geometria nulla
da errori GDAL/WKB senza trasformare ogni errore in `null`.

Se una scansione termina dopo aver osservato rifiuti, l'errore terminale
originale conserva categoria, codice, fase, effetto remoto e retry; viene
aggiunto soltanto il report partial con il limite di conoscenza appropriato.
La CLI usa exit 2 per `DataMapping` secondo gli oracoli rc17 ed exit 130 per
`Cancelled` del chiamante. Cambia soltanto lo status di processo: i codici
frozen derivati da `IoErrorCode` restano invariati (`Format`, `Wkb` e `Json`
continuano a produrre `FORMAT_ERROR`). Una deadline resta `category=timeout`,
`code=FORMAT_ERROR`, exit 1 e non viene presentata come cancellazione del
chiamante.

`RowDiagnostics::validate` applica gli invarianti di schema e aritmetici rc17
durante ogni serializzazione. La CLI non usa primitive panicking per inserire il
payload: un report interno invalido viene soppresso e sostituito da un errore
`INVALID_ROW_DIAGNOSTICS`.

La materializzazione dei reader testuali/binari non introduce buffer senza
limite: DXF mantiene al massimo 64 MiB nello spool RAM e poi passa a file,
mentre il totale logico e ogni valore sono limitati da `max_input_bytes`; KML
usa sempre uno spool su file con lo stesso limite logico e valida la lunghezza
prima di allocare ogni valore; SHP costruisce un solo batch adattivo, i campi
DBF hanno larghezza fisica bounded e geometrie/record restano inoltre sotto il
limite dell'input sorgente. Il `BudgetedReader` prenota righe, memoria e output
per il batch prima della chiamata al reader e rifiuta qualunque batch che
superi la quota prenotata. Questi sono limiti provati e fail-closed, non una
pretesa di memoria esattamente O(batch) per parser o spool preparatori.

## Compatibilità

- `row_diagnostics` è un campo additivo opzionale nella busta `error`.
- `status`, `protocol_version`, `contract` e campi errore richiesti restano
  invariati; gli exit normativi sono 2 per `DataMapping` e 130 per `Cancelled`.
- Gli errori ordinari omettono il nuovo campo; il golden test di uguaglianza
  esatta resta valido.
- Le cause sono stringhe namespaced, non estensioni di enum chiusi.
- I tipi Rust del workspace restano dichiarati `internal_unstable` e non sono
  pubblicati come crate.
- Il routing CLI legacy `.xls` viene rimosso: il driver implementa soltanto il
  contenitore `.xlsx`. Questa e' una capability drop esplicita, non una
  compatibilita' implicita con il formato binario BIFF.
- `ReadRequest::scope` distingue `Complete` da `AcceptedRows(n)`. La prima
  drena e valida fino a EOF ed e' usata da `convert`; la seconda e' riservata
  al summary `read --limit`, si ferma dopo il batch che raggiunge la soglia e
  conserva quindi l'overshoot frozen. Se il prefisso osservato contiene righe
  invalide, la diagnostica e' `Partial` (oppure `Unknown` quando l'identita'
  fisica non e' attestabile), con knowledge limit
  `read_scope_row_limit_reached`; la coda non osservata non genera indici o
  conteggi inventati.

## Perimetro DXF non supportato

La versione vendorizzata di `dxf::EntityType` espone 45 varianti. Il walker ne
gestisce geometricamente 12 (`Line`, `LwPolyline`, `Polyline`, `Circle`, `Arc`,
`ModelPoint`, `Text`, `MText`, `Solid`, `Ellipse`, `Spline`, `Insert`), ignora
soltanto quattro record strutturali/template senza geometria autonoma
(`AttributeDefinition`, `Attribute`, `Seqend`, `Vertex`), e rifiuta
esplicitamente `Region`/`Body`. Le altre 27 varianti raggiungono il catch-all
fail-closed `entita DXF non gestita`: non sono accettate, pubblicate o contate
come conversioni riuscite. Il conteggio descrive questa revisione della
dipendenza e non costituisce una promessa per future estensioni dell'enum.

## Verifica prevista

- fixture Shapefile reale da 128 record con rifiuti agli indici 17, 89 e 113;
- due patologie distinte e conteggi completi oltre il limite esempi;
- stabilità dell'indice attraverso batch multipli e un record DBF deleted;
- projection attribute-only con gli stessi rifiuti;
- attributo `N(18,0)` corrotto con causa e indice sorgente, senza bloccare i
  conteggi completi delle altre righe invalide;
- chiave assente, emessa e redatta, incluso valore Numeric decimale oltre 2^53
  conservato dal lessicale raw;
- heartbeat invisibile al reader e rilevamento del consumer rilasciato;
- propagazione del report nell'envelope CLI e assenza sugli errori ordinari;
- test dei crate interessati, formatter, Clippy workspace, suite workspace e
  gate del contratto release;
- validazione successiva contro il corpus condiviso di `plenora-contracts`.

La prova live FileGDB/GDAL con dataset e runtime reali resta pending esplicita;
nessuna suite feature-gated locale viene qualificata come sostituto.

## Follow-up rc17 del 2026-08-03

La scope typed viene ora propagata fino al parser Shapefile. Dopo la prima
rejection, `AcceptedRows(n)` osserva le righe DBF attive fino alla soglia (i
record deleted avanzano l'indice fisico ma non la soglia), quindi termina senza
leggere la coda. Il report resta `partial`, senza `total`, con
`read_scope_row_limit_reached`; `Complete` continua invece fino a EOF. Il path
valido non e' stato cambiato: il batch non viene affettato e conserva
l'overshoot storico. `AcceptedRows(0)` non chiama il reader interno.

Il merge tra diagnostica comune e diagnostica terminale del driver e'
transazionale: scope/contratto/index basis devono essere compatibili, conteggi
e totale osservato usano aritmetica checked, gli esempi restano bounded e la
completezza e' conservativa. Un payload driver invalido non viene promosso a
`complete` e viene dichiarato con `driver_row_diagnostics_invalid`.

DXF, KML e GeoJSON aggiungono diagnostica read soltanto nei call path
row-level. DXF attesta l'ordinale dell'entita top-level quando la conversione e'
iniziata; un decode che non restituisce un'entita usa `unknown`, zero esempi e
`source_row_identity_unattestable`. KML attribuisce gli errori interni al
Placemark corrente. GeoJSON mantiene `in_feature` e l'ordinale in entrambi i
pass streaming. Gli errori strutturali esterni a una riga restano errori typed
senza attribuzione inventata.

Il presunto tetto Arrow di 72 MiB era un finding reale nel bordo comune:
`BudgetedReader::drain_operation` usava `RecordBatch::get_array_memory_size`,
che per una slice zero-copy include la capacity completa dei buffer del parent.
L'accounting comune usa ora metadata Arrow posseduti piu
`ArrayData::get_slice_memory_size`; lo stesso valore guida batch sizing, read e
write budget. Il test `large_parent_small_slice_is_charged_incrementally_but_large_batch_is_rejected`
costruisce un parent da 73 MiB e una slice di una riga: la slice passa, mentre
un batch logico non affettato da 73 MiB resta rifiutato oltre la prenotazione.
