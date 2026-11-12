# Changelog

## 8.0.0 — non pubblicata

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
  `wkb-{tipo}-{xy,xyz}-value` degli stessi sette tipi: 30 celle. Il gate ammette
  solo questa differenza dopo confronto indipendente di pagine dati, footer
  nativo e schema completo atteso. Qualunque altra differenza fallisce.
  I rifiuti concordi non contano come confronti binari riusciti. FileGDB
  richiede entrambi gli eseguibili con GDAL e resta da eseguire su Linux.


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
