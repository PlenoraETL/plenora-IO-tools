# Changelog

## 8.0.0 — non pubblicata

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
