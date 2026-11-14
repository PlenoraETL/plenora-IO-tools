# Changelog

## 9.0.0 — candidate in lavorazione

Python minimo 3.14, deciso dal titolare: cambio di compatibilità major.
Fonte unica `.python-version`, ABI cp314 e guardia CI.
Ristrutturazione della qualifica IO B-11/B-14/B-15 in corso; questa voce
non attesta completamento, qualifica di sistema o autorizzazione al rilascio.

La proiezione GPKG delega al core: nello snapshot dei quartetti scompare il
sito locale duplicato, mentre il controllo resta in `project_layer_contract`.
Il runtime riusa il parser JSON rigoroso del model. AsyncClient e il suo stub
derivano le operazioni dal client sincrono.

## 8.0.0 — non pubblicata

### Compatibilita degli errori: snapshot dei quartetti

- FileGDB `open`: rimosso dal sito il quartetto `Crs/Validate/Crs/Never`;
  resta `Unsupported/Validate/Unsupported/Never`. Il controllo eliminato
  richiedeva ID o definizione dopo `resolve_layer_crs`, che restituisce sempre
  `Some(definition)` quando riesce: non era un rifiuto raggiungibile.
  Il rifiuto del CRS assente senza override resta in `resolve_layer_crs`
  (`Crs/Validate/Crs/Never`); override non risolvibile e WKT non esportabile
  restano `Crs/Validate/CrsUnresolved/Never` nello stesso risolutore.
  Un WKT senza ID resta una definizione canonica, mai un identificativo
  `geo:crs`. Nessun rifiuto CRS perso o spostato da questa rimozione.
- GeoParquet `multigeometria::verifica_crs`: nuovo sito
  `Schema/Validate/Schema/Never` per dimensioni risolte del campo Arrow in
  conflitto con un footer a dimensioni miste, prima del retag e per primaria
  e secondarie. `unknown` e tipi footer assenti restano non risolti e vengono
  completati dall'altra fonte; il nuovo rifiuto impedisce di nascondere una
  contraddizione. E una restrizione intenzionale della major 8.0.0.
- Snapshot `quartetto-siti.json` rigenerato con
  `python scripts/check_quartetto_sito.py --aggiorna` per questi due cambi,
  verificati sulla diff `50f9357..b9115fa5`; nessun cambio degli assi nei
  costruttori condivisi. Questa nota ne registra la ragione di compatibilita.

### Correzione 5: ingressi storici e metadati dichiarati

- **C5-GP-UNRESOLVED**: il confronto campo/footer ignora valori non risolti
  da entrambe le direzioni; il completamento conserva il valore risolto.
  La tabella delle 13 proprieta enumera assenze e token non risolti, con test.
  Restano rifiutati due valori risolti discordi, anche per primaria e secondarie.
  Il campione reale 7.0.3 GeoJSON Point con `dimensions=unknown` e conservato
  come fixture e riletto fino ai valori.
- **C5-GP-SRID**: uno SRID dichiarato dal campo si conserva anche con CRS84
  o con un ID EPSG diverso. La derivazione EPSG si usa soltanto in assenza
  di uno SRID dichiarato. Due SRID dichiarati discordi restano un conflitto.
  Anche CRS e tipi risolti del campo si conservano con footer non risolto.
  Il retag del reader conserva le proprieta native dichiarate solo dal campo,
  inclusa l'epoca, invece di cancellare tutte le chiavi geoparquet.*.
- La matrice storica riapre come ingressi tutte le uscite IPC file/stream e
  GeoParquet riuscite della 7.0.3, confrontando entrambe le versioni verso le
  tre destinazioni. Nessuna differenza non spiegata e ammessa.
- **C5-GP-EMPTY-TYPES**: la nuova fase di rilettura misura anche la differenza
  gia presente fra 7.0.3 e il completamento dei contratti 8.0.0: con tipi exact
  nel campo, footer senza tipi e righe tutte nulle o assenti, la 7.0.3 rifiuta
  la conversione per tipi non dichiarati. La 8.0.0 conserva la dichiarazione.
  L'oracolo richiede quell'esatto errore storico; per GeoParquet i byte nuovi
  devono essere identici alla sorgente, per IPC valori e schema devono
  corrispondere alla sorgente con il solo completamento nativo enumerato.
- Nella rilettura, **C5-GP-SRID** ammette soltanto il ripristino dello SRID
  dichiarato dalla sorgente e assente nell'uscita storica: schema atteso esatto,
  pagine Parquet o messaggi dati IPC byte-identici. I negativi degli oracoli
  rifiutano SRID diverso, valori cambiati, metadati estranei ed errori diversi.
- FileGDB Linux, quarto giro: 37 test passati, 0 falliti, secondo l'evidenza
  fornita dal titolare; workspace 67 suite verdi; matrice storica 1.825 byte
  identici, 815 rifiuti concordi, 30 differenze spiegate, 0 inspiegate.
  Questa misura precede il quinto giro e non sostituisce la sua riesecuzione
  Linux. Nessuna qualifica di release o handoff esterno implicita.

### Correzione 3: riconciliazione e byte storici

- **C3-GP-CONFLICT**: footer GeoParquet e campo Arrow incorporato sono
  confrontati prima del retag, per primaria e secondarie. Encoding, tipi,
  dimensioni, precisione, semantica, SRID, CRS e proprieta native dichiarate
  da entrambe le fonti devono concordare; un disaccordo e `schema`, anche
  sul CRS. Non si uniscono gli insiemi dei tipi. `unknown`, liste footer
  vuote e proprieta assenti non dichiarano un vincolo risolto: il vincolo
  dell'altra fonte resta conservato. Nessuna promozione di primaria.
- **C3-GP-DIM**: il writer finalizza anche le dimensioni del campo Arrow
  incorporato usando i tipi realmente osservati nel footer. La precedente
  uscita GeoJSON verso GeoParquet poteva dichiarare `unknown` nel campo e
  `Point`/`Point Z` (o gli altri tipi) nel footer. Ora dichiara rispettivamente
  `xy`/`xyz`. Questo cambia i byte del footer e la sua lunghezza, non le
  pagine dati, i valori, il CRS o gli altri metadati. E una correzione della
  coerenza delle dichiarazioni nella major, non una promessa di byte identici
  per questi casi.
- La matrice versionata `scripts/compare_historical_readers.py` confronta
  7.0.3 (`50f9357`) con 8.0.0 su CSV, XLSX, GPKG, Shapefile, DXF, KML,
  GeoJSON e FileGDB, verso IPC file/stream e GeoParquet. Nella misura locale
  C3-GP-DIM riguarda GeoJSON con point, linestring, polygon, multipoint,
  multilinestring, multipolygon, geometrycollection, ciascuno XY e Z, e le
  fixture `canonico_punti` e `canonico_punti_dbf`, piu le 14 varianti
  `wkb-{tipo}-{xy,xyz}-value` degli stessi sette tipi: 30 celle. Nel terzo giro il gate ammetteva
  solo questa differenza dopo confronto indipendente di pagine dati, footer
  nativo e schema completo atteso. Nel quinto giro si aggiungono le sole
  eccezioni sopra descritte; qualunque differenza non spiegata fallisce.
  I rifiuti concordi non contano come confronti binari riusciti. FileGDB
  richiede entrambi gli eseguibili con GDAL; la misura Linux successiva del
  quarto giro e registrata sopra separatamente dalla misura locale originaria.


Release major secondo `docs/RELEASE.md`: i nuovi rifiuti PRIM-003 e il
significato di `none` restringono l'insieme degli input accettati dalla 7.x.
Wire Arrow 1 e versioni delle operazioni pubbliche restano invariati.

- IPC file e stream trasportano più colonne `geoarrow.wkb`, con CRS, encoding,
  identità, valori e null indipendenti, con o senza primaria.
- `plenora.primary_geometry` è una chiave di schema: accetta `none` o un ID
  decimale canonico di un campo geometrico. Sono ora rifiutati con `schema`
  nomi (`position`), indici che non sono ID dichiarati, riferimenti pendenti
  (`91` senza campo 91), ID di attributi, `01`, stringhe vuote o con segno,
  chiavi su campi e qualsiasi chiave su schemi senza geometrie.
  La risoluzione precede l'assegnazione degli ID mancanti; gli errori di
  vocabolario dei campi precedono la primaria, che precede il CRS.
- Una geometria con `none` non è primaria. Proiettare via la primaria non
  promuove una secondaria rimasta sola: viene emesso `none`.
- Emissione canonica: ID solo per più geometrie con primaria, `none` solo per
  una geometria senza primaria, chiave assente negli altri casi. Un ID
  esplicito singolo e `none` multiplo validi vengono quindi rimossi.
- GeoParquet associa `primary_column` alla primaria Arrow nei due versi e
  rifiuta con `unsupported` qualsiasi tabella geometrica senza primaria,
  inclusa una geometria con `none`. I metadati nativi sono provenienza.
- GPKG, Shapefile, CSV, XLSX, FileGDB, GeoJSON, DXF e KML conservano il rifiuto
  multigeometria prima della pubblicazione (GEO-012).
  Rifiutano anche `none`: questi sink non rappresentano l'assenza esplicita
  di primaria; IO non la promuove e non perde silenziosamente lo stato.
- Parquet senza `geo` ma con schema GeoArrow usa la primaria dello schema,
  anche senza primaria, invece del fallback per nome. La chiave incorporata
  viene validata prima di retag e completamento, anche quando esiste `geo`.
  Una chiave incorporata esplicita in conflitto con `primary_column` è
  rifiutata con `schema`; non viene riparata durante la lettura.
- Una singola geometria senza primaria passa ora tutti i controlli geometrici
  di supporto e perdita: un piano privo dei tipi richiesti dal sink può essere
  rifiutato, invece di eludere quei controlli.
- Il modello Rust conserva il limite u32 degli ID geometrici; un ID decimale
  conforme oltre quel dominio è `unsupported`, non un errore di grammatica.
- GeoParquet con un campo GeoArrow incorporato non dichiarato nel footer e
  privo del vocabolario richiesto viene rifiutato gia in apertura con `schema`,
  prima del precedente rifiuto di supporto alla lettura del layer.
- Il catalogo espone `geometry_cardinality`, derivata dalla tabella comune.
- I writer Rust diretti, le proiezioni e la conversione EWKB riconciliano
  schema e contratto interno prima di normalizzare: un disaccordo su primaria,
  identita o proprieta geometriche e' `schema`. Il vocabolario ricevuto viene
  verificato anche quando `geometry` e' `Some`. L'assenza della chiave segue
  PRIM-002 e non autorizza il piano a eleggere una primaria.
  `DataContract::from_schema` costruisce un piano coerente da uno schema;
  i campi pubblici restano compatibili, ma non aggirano la validazione.
  `DataContract::new` conserva lo schema senza completarlo, anche su campi
  privi di estensione: chi produce nuovi schemi deve dichiararne i metadati.

### Migrazione da 7.x

Validare la chiave prima di inviare Arrow: usare `plenora.field_id`, mai il
nome o la posizione fisica. Rimuovere la chiave dagli schemi non geometrici
e dai metadati dei campi. Se si desidera la primaria implicita singola,
omettere la chiave; usare `none` solo quando si vuole davvero nessuna primaria.
Per GeoParquet dichiarare esplicitamente l'ID della primaria in uno schema
multi. Per un sink singolo scegliere/proiettare esplicitamente a monte: IO
non scarta le secondarie. Aggiornare i confronti dei metadati alla forma
canonica PRIM-004. Non usare la provenienza GeoParquet per eleggere la primaria.

La qualifica 8.0.0, gli artefatti, i digest, L1/L2 e le campagne fuzz/ASan
richiedono nuove misure sul commit congelato. Nessuna pubblicazione implicita.
