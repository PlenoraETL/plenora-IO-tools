# Prodotto — che cosa offre e che cosa promette

Questo documento descrive la superficie che un consumatore vede: driver,
opzioni, contratti pubblici e limiti. Come è costruita e come è verificata sta
in [ENGINEERING.md](ENGINEERING.md); dove siamo nel percorso di rilascio sta in
[RELEASE.md](RELEASE.md).

---

## I dieci driver

| Driver | Direzione | Lettura nativa | Projection | Pruning | Reader concorrenti | Fedeltà | CRS | Feature |
|---|---|---|---|---|---|---|---|---|
| `csv` | R/W | sequenziale | esatta | — | multipli | condizionale | nessuno | — |
| `geojson` | R/W | sequenziale | esatta | — | multipli | condizionale | WGS84 fisso | — |
| `kml` | R/W | materializzata | — | — | uno attivo | condizionale | WGS84 fisso | — |
| `shp` | R/W | sequenziale | esatta | — | multipli | condizionale | incorporato | — |
| `gpkg` | R/W | ad accesso casuale | esatta | — | multipli | condizionale | incorporato | — |
| `geoparquet` | R/W | ad accesso casuale | esatta | statistiche min/max numeriche | multipli | **lossless** | incorporato | — |
| `ipc` | R/W | ad accesso casuale | esatta | — | multipli | **lossless** | incorporato | — |
| `xls` | R/W | materializzata | — | — | uno attivo | condizionale | nessuno | — |
| `dxf` | R/W | materializzata | — | — | uno attivo | **approssimante** | incorporato | — |
| `filegdb` | R/W | sequenziale | esatta | — | uno attivo | condizionale | incorporato | `gdal-backend` |

**Consegna e buffering sono uguali per tutti**: `operation_atomic` con buffering
adattivo — memoria finché il budget lo consente, poi spool su disco. Un errore a
metà lettura non consegna righe parziali.

Una scrittura rifiutata non lascia una destinazione, con **una** eccezione
dichiarata: il set di file sciolti dello Shapefile, che si ottiene solo
accettandolo esplicitamente e il cui contratto è più sotto, in
[§ Publish](#publish-che-cosa-diventa-visibile-e-quando).

**Fin dove arriva il supporto di una specifica**: il catalogo lo dichiara nel
campo `spec_version_supported` di ciascun driver. `geoparquet` dichiara
`1.1.0`: legge per intero i metadati `geo` delle versioni 1.0.0 e 1.1.0 — i due
valori che gli schemi ufficiali fissano — e scrive 1.1.0. Una versione oltre
viene rifiutata come **funzionalità non supportata**, che è cosa diversa da
metadati non conformi: il primo errore dice che il file va bene e noi no, il
secondo che il file è sbagliato. Con lo stesso criterio sono rifiutate le
codifiche native GeoArrow e i bordi sferici, entrambi validi in GeoParquet 1.1
e non implementati qui. Gli altri driver dichiarano `null`: i loro formati non
si versionano in un modo che il driver possa dichiarare per intero.

### Determinismo

I driver garantiscono **determinismo semantico**, non byte per byte: a parità di
input, opzioni e versione dell'implementazione, stessi valori e stesso insieme
di righe, **senza** assumere un ordine o una rappresentazione fisica identici.

È la garanzia minima della scala dichiarata dal modello — `semantic`, `ordered`,
`byte_for_byte`, `unordered` — e i dieci driver dichiarano `semantic` sia in
lettura sia in scrittura.

L'unica superficie con determinismo **byte per byte** è la busta di
`plenora-io catalog`, che serializzata due volte produce gli stessi byte. È una
proprietà di quel comando, non dei driver, e vale la pena non confonderle:
riconvertire due volte lo stesso file può produrre due output diversi byte per
byte e ugualmente corretti.

`gpkg` e `filegdb` sono multi-layer.

### Publish: che cosa diventa visibile, e quando

Il publish ha tre forme. **Due sono crash-atomic e una no**, e la differenza non
è un dettaglio di implementazione: decide che cosa un altro processo può vedere
sul disco se il nostro muore a metà.

| forma | chi la usa | garanzia |
|---|---|---|
| file singolo | `csv`, `geojson`, `kml`, `xlsx`, `dxf`, `gpkg`, `geoparquet`, `arrow` | un rename: il file c'è per intero o non c'è |
| directory-dataset | `filegdb`, e `shp` con destinazione `*.shp.d` | un rename della directory: tutti i file insieme, o nessuno |
| set di file sciolti | `shp` con destinazione `*.shp` | **nessuna**: quattro rename in sequenza |

La forma raccomandata in produzione per lo Shapefile è la **directory-dataset**,
e si ottiene chiedendo una destinazione che finisce in `*.shp.d`. Dentro ci sono
`data.shp`, `data.shx`, `data.dbf` e `data.prj`, e la directory diventa visibile
con un rename solo.

#### Il set sciolto va accettato, non subìto

La forma compatibile — `dati.shp` accanto a `dati.shx`, `dati.dbf`, `dati.prj` —
**non ha quella garanzia e non può averla**: quattro file separati non si rendono
visibili in un atto solo su nessuno dei filesystem supportati. Per questo non si
ottiene più deducendola dall'estensione: una destinazione `*.shp` è **rifiutata**
finché non la si accetta con l'opzione di formato
`publish_mode=loose_shapefile_set`. Il rifiuto è `InvalidConfiguration` — la
richiesta è incompleta, non il prodotto incapace.

Che cosa quell'accettazione comporta, per intero:

| | |
|---|---|
| **ordine** | i companion prima, il `.shp` per ultimo: chi cerca il file marker non lo trova finché il resto non c'è |
| **errore durante il publish** | i companion già spostati vengono riportati nello staging, best-effort |
| **rollback riuscito** | l'errore porta `remote_effect: none`: nessuna destinazione è rimasta visibile |
| **rollback fallito** | l'errore porta `remote_effect: partial` e `retry: requires_recovery`: sul disco **possono** esserci companion senza il `.shp` |
| **processo ucciso a metà** | nessun rollback avviene: lo stesso stato parziale, e nessun errore che lo dichiari |

L'ultima riga è la ragione per cui questa forma non è raccomandata: le prime
quattro descrivono ciò che il codice fa, la quinta ciò che nessun codice in
spazio utente può fare.

#### Recovery di un set sciolto interrotto

Vale per `remote_effect: partial` e per un processo ucciso durante il publish. È
la stessa procedura, perché è lo stesso stato: un insieme incompleto di file
accanto alla destinazione.

1. **Non ripetere la scrittura senza guardare.** Il publish è no-clobber su ogni
   file del set: un companion sopravvissuto fa fallire il tentativo successivo
   con `OutputExists`, e quel fallimento è corretto — non è il segno che si può
   riprovare.
2. **Riconoscere lo stato.** Un set completo ha `.shp`, `.shx`, `.dbf` e, se il
   layer porta un CRS, `.prj`. Un set senza `.shp` è incompleto per costruzione:
   il marker è l'ultimo a essere pubblicato.
3. **Rimuovere i companion rimasti**, cioè i file con lo stesso stem della
   destinazione che il publish avrebbe scritto. Nessuno di essi è un dato
   valido: appartengono a una scrittura che non si è conclusa.
4. **Ripetere la conversione**, preferibilmente verso `*.shp.d`, dove questa
   procedura non serve.

Lo staging non va cercato: è una directory temporanea che il processo rimuove al
termine, anche in errore. Ciò che il crollo può lasciare è **solo** accanto alla
destinazione.

### Che cosa significano le classi di fedeltà

| | |
|---|---|
| **lossless** | il formato rappresenta il contratto senza perdita |
| **condizionale** | la perdita dipende dal contratto, non dal formato: un CSV senza colonne esotiche è lossless, con un tipo temporale non lo è |
| **approssimante** | il formato approssima per costruzione. DXF tassella archi ed ellissi, esplode le geometrie multipart, rappresenta il testo come punto |

Ciò che si perde è dichiarato nel report di perdita, non taciuto, e il
**contratto** di quel report è ratificato: vedi
[LossReport](#lossreport--ratificato-con-il-protocollo-2).

### Modalità di lettura nativa

La modalità nativa è ciò che il formato consente; la modalità effettiva è
sempre `streaming_sequential`, perché l'adapter comune impone l'atomicità
operativa.

* **sequenziale** — il file si legge in avanti una volta sola;
* **ad accesso casuale** — il formato indicizza e consente di saltare;
* **materializzata** — il parser ha bisogno di **tutto l'input** prima di
  consegnare la prima riga. Non vuol dire «tutto in RAM»: dove stia quell'input
  lo descrive il buffering, che è un asse separato e adattivo — RAM sotto una
  soglia, poi spool su file. La coppia dice esattamente che cosa succede: serve
  tutto l'input, non serve tutta la memoria, e confondere i due assi è il
  difetto che il modello tiene distinto per costruzione. Per questi driver i
  limiti di input sono l'unica difesa contro un file ostile, e sono applicati
  **prima** che il parser veda il contenuto.

## Opzioni di formato

Le opzioni sono dichiarate in uno **schema**: chiave, fase (`read`, `write`,
`both`), tipo del valore e default. Un valore fuori schema è rifiutato prima di
raggiungere il driver, con il token dell'opzione rifiutata e l'elenco degli
ammessi.

`plenora-io catalog` emette lo schema completo. Le opzioni con un default
diverso da «assente»:

| Driver | Chiave | Fase | Valore | Default |
|---|---|---|---|---|
| `csv` | `delimiter` | entrambe | un carattere ASCII | `,` |
| `csv` | `geometry_encoding` | scrittura | `wkt` \| `xy` | `wkt` |
| `csv` | `wkt_column` | lettura | testo | assente |
| `xls` | `geometry_encoding` | scrittura | `wkt` \| `xy` | `wkt` |
| `xls` | `wkt_column` | lettura | testo | assente |
| `xls` | `sheet` | lettura | testo | primo foglio del workbook |
| `shp` | `publish_mode` | scrittura | `shapefile_directory_dataset` \| `loose_shapefile_set` | nessuno: `*.shp.d` si deduce, `*.shp` va accettata |
| `shp` | diagnostica di riga e nomi DBF | | | vedi catalogo |
| `geoparquet` | opzioni di scrittura | | | vedi catalogo |

I driver senza opzioni dichiarate non ne accettano: passarne una è un errore di
configurazione, non un valore ignorato.

### CRS

| | |
|---|---|
| **incorporato** | il formato porta il CRS, e viene letto da lì |
| **WGS84 fisso** | il formato lo impone; un CRS diverso è rifiutato |
| **nessuno** | il formato non lo porta. Se il contratto ha una geometria, `--assume-crs` è **obbligatorio**: senza, la lettura fallisce in fase CRS invece di indovinare |

Nessun driver riproietta. Il CRS viene letto, scritto o dichiarato — mai
trasformato.

#### `--assume-crs` e il CRS che la sorgente dichiara

L'opzione **nomina** un CRS che la sorgente non dichiara. Quando la sorgente ne
dichiara uno, i due si confrontano per ogni colonna geometrica, in un punto
solo che ogni driver attraversa all'apertura (`with_read_budget` in
`plenora-io-core`, regola in `plenora_io_model::crs::verifica_crs_assunto`):

| la sorgente dichiara | `--assume-crs` |
|---|---|
| un identificatore uguale (GEO-005: autorità senza maiuscole, codice come testo) | ammesso; il contratto conserva la grafia della sorgente |
| un identificatore diverso | rifiutato con `crs` |
| un CRS senza identificatore (una definizione senza autorità) | rifiutato con `crs`: non si può confrontare |
| niente, o un CRS dichiarato ignoto (Arrow IPC senza CRS, GeoParquet con `crs: null`) | rifiutato con `crs`: l'opzione resterebbe senza effetto. Il messaggio dice che la geometria resta senza CRS «perché la sorgente lo dichiara ignoto o il formato non usa l'opzione»: fino alla rilettura 10 diceva «il formato non usa l'opzione» anche per GeoParquet, che la usa per un Parquet senza metadato `geo` (prova `un_crs_ignoto_si_riscrive_in_geoparquet_come_null`) |

Fino alla 5.0.1 GeoPackage, GeoParquet, GeoJSON, KML e IPC ignoravano
l'opzione, e DXF e FileGDB la ignoravano quando il CRS incorporato dichiarava
il proprio identificatore: chi chiedeva `EPSG:3003` riceveva il CRS del file
senza che la busta lo dicesse. Il confronto non è un resolver: `OGC:CRS84` ed
`EPSG:4326` sono identificatori diversi, e su un GeoJSON l'opzione ammessa è
`OGC:CRS84` (prove `un_assume_crs_che_contraddice_il_crs_dichiarato_e_rifiutato`
in `plenora-io-tools`, `verifica_crs_assunto_rifiuta_cio_che_non_puo_confermare`
in `plenora-io-model`).

#### `--assume-crs` dichiara, non risolve

`--assume-crs EPSG:3003` dice **quale** CRS hanno i dati. Non è un resolver
EPSG→WKT: il prodotto non porta un database di definizioni e non ne inventa
una. Per i formati che scrivono una definizione — lo Shapefile nel `.prj`, il
DXF nel `GEODATA` — l'identificatore da solo basta soltanto per WGS84, l'unico
per cui il writer sintetizza il WKT.

Fuori da quel caso i due formati si comportano in modo diverso, ed entrambi i
comportamenti sono voluti:

| | |
|---|---|
| **DXF** | **rifiuta**: senza definizione non scrive un file monco |
| **Shapefile** | **scrive** i dati senza `.prj`, e lo **dichiara** nel `LossReport` |

Ciò che lo Shapefile dichiara è `crs_id_not_preserved_absent` — e
`srid_not_preserved_absent` se la sorgente portava un SRID. Non
`..._derived`: *derivata* significa che il valore si rilegge dal file
attraverso un'altra rappresentazione, e senza `.prj` non c'è niente da cui
rileggerlo. La distinzione conta per chi automatizza: `absent` dice che il CRS
va riportato **fuori** dal file, `derived` direbbe che basta rileggerlo.

**GeoParquet è il caso più stretto**: la specifica vuole un documento PROJJSON,
e il driver non ne sintetizza uno da un identificatore. Una scrittura il cui
CRS è noto solo come `EPSG:3003` viene quindi **rifiutata**, non degradata — e
lo è anche quando la sorgente porta un WKT, che GeoParquet non ha un modo
conforme di scrivere. Un PROJJSON si scrive per intero e si rilegge:
GeoParquet **conserva** la definizione PROJJSON, e dell'SRID — che il metadato
`geo` non ha dove mettere — dichiara la sola derivazione dall'identificatore.

#### Che cosa ogni bersaglio fa della definizione che riceve

Un Arrow conforme porta la definizione in una qualunque delle tre grammatiche
(WKT, WKT 2, PROJJSON), e GEO-005 confronta solo gli identificatori. Dalla
rilettura 10 `crs_definition: preserved` vale **per grammatica**
(`CrsRepresentationCapabilities::verbatim_solo` in `plenora-io-core`), e il
verdetto per piano la legge: una definizione che il bersaglio non scrive tale e
quale è una perdita dichiarata o un rifiuto, mai una conversione `lossless`.

| bersaglio | conserva | un'altra grammatica |
|---|---|---|
| IPC | tutte | — |
| GeoParquet | PROJJSON | con `OGC:CRS84` o `EPSG:4326` il campo `crs` si omette (il file si rilegge `OGC:CRS84`, senza definizione) e la perdita è `crs_definition_not_preserved_absent`; con un altro identificatore, o senza, `unsupported`. Fino a b7a6c94 il primo caso era `lossless`, e un WKT senza identificatore usciva `crs: null`, «CRS ignoto», per un CRS che il contratto conosceva |
| Shapefile (`.prj`), DXF (`GEODATA`) | WKT, WKT 2 | un PROJJSON con `OGC:CRS84` o `EPSG:4326` si sostituisce con il WKT che il writer sintetizza, e la perdita è `crs_definition_not_preserved_derived`; con ogni altro identificatore `unsupported` («il `.prj` porta un WKT»). Fino a b7a6c94 il PROJJSON si scriveva nel `.prj` o nel `GEODATA` così com'era, la conversione riusciva, e il file non si rileggeva più — nemmeno con `--assume-crs`. Convertire PROJJSON in WKT servirebbe un resolver: un GeoParquet in EPSG:3003 non si converte in Shapefile né in DXF |
| GeoPackage, FileGDB | — | la definizione si riscrive (da `gpkg_spatial_ref_sys` o da GDAL): `crs_definition_not_preserved_derived`, come prima |
| GeoJSON, KML, CSV, XLSX | — | `crs_definition_not_preserved_absent`, come prima |

Un `missing` si scrive in GeoParquet come `crs: null` (capability
`embedded_or_unknown`, nuova nel catalogo): era la promessa della riga
«GeoParquet: `crs: null`», e fino a b7a6c94 la validazione la rifiutava
(«il formato richiede un CRS risolto»). Il dichiarato ma non risolto resta un
rifiuto: GeoParquet non ha un modo di dirlo.

**Il bersaglio non sceglie l'identificatore contro la definizione.** Un
identificatore che il bordo sa orizzontale su una radice che non lo è
(`EPSG:4326` su un `LOCAL_CS`) o di genere opposto (`EPSG:4326` su un `PROJCS`
anonimo) è la contraddizione che i lettori rifiutano (`crs_kind_for`, riga
«tutti: genere del CRS»). Un Arrow conforme la può portare, e IPC la conserva
intera; un bersaglio che conserverebbe il solo identificatore — GeoParquet con
un WKT, GeoPackage, GeoJSON, KML — la rifiuta con `unsupported` e la ragione
`crs_representations_inconsistent`, prima di ogni effetto. È un limite del
bersaglio su un ingresso conforme (GEO-000 punto 2, REJ-001 «support»), non un
difetto dell'ingresso: per questo non è `crs`. Shapefile e DXF scrivono il WKT,
e FileGDB lo passa a GDAL: lì non si sceglie niente. Fino a b7a6c94 GeoParquet
scriveva `EPSG:4326` per un `LOCAL_CS` dichiarandosi `lossless`, e GeoPackage
lo scriveva come `EPSG:4326` con la definizione di WGS 84. Prove:
`geoparquet_dichiara_il_wkt_che_non_scrive`,
`geoparquet_rifiuta_un_wkt_senza_identificatore`,
`l_identificatore_non_si_sceglie_contro_la_definizione`,
`shp_e_dxf_rifiutano_un_projjson_che_non_sanno_scrivere`,
`shp_e_dxf_sostituiscono_il_projjson_e_lo_dichiarano`,
`un_crs_ignoto_si_riscrive_in_geoparquet_come_null` (in `plenora-io-tools`),
`le_definizioni_che_non_sono_projjson_non_spariscono_in_silenzio` (in
`driver-geoparquet`), `geoparquet_proiettato_a_dxf_rifiuta_il_projjson`
(matrice delle conversioni).

#### Ordine degli assi

`plenora.geometry.axis_order` dichiara l'ordine delle prime due ordinate
**come memorizzate** nei valori WKB (Arrow Geometry Semantics 1.0, GEO-001),
non l'ordine con cui il registro elenca gli assi del CRS: `EPSG:4326` letto da
un GeoPackage è `lon_lat`, anche se EPSG registra la latitudine per prima.
Fino alla 5.x il prodotto dichiarava l'ordine del registro (`lat_lon` per ogni
`EPSG:4326`) mentre i valori portavano la longitudine prima.

| in lettura | ordine dichiarato |
|---|---|
| GeoPackage, GeoParquet, Shapefile, FileGDB, DXF | il formato memorizza easting o longitudine prima: `lon_lat` per un geografico, `easting_northing` per un proiettato i cui assi sono est/nord, `other` se la definizione dice assi che non lo sono (una polare come EPSG:3031), `unknown` se il genere è ignoto, se la definizione non dice gli assi o se un proiettato non ha definizione (GeoPackage con `definition = 'undefined'`) |
| GeoJSON, KML | `lon_lat` (RFC 7946, KML 2.2) |
| CSV, XLSX con `x_column`/`y_column` | la richiesta nomina la `x`, e la `x` è l'est: `lon_lat` con `--assume-crs` geografico |
| CSV, XLSX con `wkt_column` | `unknown`: il WKT non fissa l'ordine e la richiesta non lo dichiara |
| Arrow IPC | quello che il campo dichiara; con la sola chiave `crs`, `unknown` |

In scrittura, un formato che fissa easting o longitudine prima — tutti tranne
Arrow IPC, compresi CSV e XLSX — **rifiuta** un campo `lat_lon` o
`northing_easting` con `unsupported`, ragione `axis_order_not_representable`,
prima di creare la destinazione: scritto com'è uscirebbe invertito, e il bordo
non ha una conversione delle ordinate qualificata. Arrow IPC porta la
dichiarazione con i valori e li accetta tutti.

Limiti dichiarati (regola 4):

* **`other` e `unknown` passano verso i formati con l'est per primo.** Il
  bordo non sa provare che contraddicano il formato, e i valori si scrivono
  come sono. Garanzia indebolita: la rilettura dichiara l'ordine del formato,
  non `unknown`. Hazard: un campo `unknown` che memorizzava la latitudine prima
  esce dalla rilettura come `lon_lat`. Rientro: rifiutare `unknown` in
  scrittura, quando una versione del contratto lo decide.
* **Proiezioni polari in WKT 1 senza `AXIS`.** Il default OGC 01-009 è est poi
  nord; il prodotto dichiara `other` solo quando il nome della proiezione dice
  polare (`Polar_Stereographic`, `..._Pole`) o la latitudine d'origine è ±90.
  Hazard: una polare che non lo dichiara esce `easting_northing`. Rientro: un
  resolver CRS al bordo.
* **CSV e XLSX rifiutano `lat_lon` anche in WKT**, dove il formato non fissa
  l'ordine: il prodotto non distingue la codifica in validazione, e la
  rilettura con `x_column`/`y_column` dichiarerebbe `lon_lat`.
* **`io.catalog` non pubblica gli ordini ammessi**: il protocollo v2 è
  ratificato, e la chiave nuova è una decisione di protocollo. Il limite si
  legge dal rifiuto.

---

## Contratti pubblici

### Limite temporaneo del consumo Rust

La verifica del consumatore esterno dall'archivio sorgente garantisce la
costruzione partendo dal `Cargo.lock` distribuito, da copiare nella radice del
progetto consumatore prima di `cargo run`. Non garantisce la risoluzione da
zero senza lockfile: calamine e rust_xlsxwriter richiedono ancora zip 8.x,
le cui versioni sono tutte yanked e riutilizzabili solo da un lockfile esistente.
È una garanzia indebolita, dichiarata nella stessa
[eccezione zip 8.6.0](../assurance/registries/dependency-exceptions.json).
Quando entrambi pubblicheranno versioni su zip 9, aggiorneremo dipendenze e
lockfile e il consumatore tornerà a essere verificato senza lockfile iniziale.

### Compatibilità

La superficie con garanzia di compatibilità è **il JSON della CLI**. L'API Rust
è interna e instabile: non porta garanzia semver e i crate non sono pubblicati.

### Le buste

| Comando | Contratto | Fonte del nome |
|---|---|---|
| errori | `plenora-error-v1` | ERRORS-1.0 |
| `catalog` | `plenora-io-catalog-v1` | catalogo comune |
| `inspect` | `plenora-io-inspect-v1` | catalogo comune |
| `layers` | `plenora-io-layers-v1` | catalogo comune |
| `read` | `plenora-io-read-result-v1` | catalogo comune |
| `write` | `plenora-io-write-result-v1` | catalogo comune |
| `convert` | `plenora-io-convert-v1` | catalogo comune |
| `capabilities` | `plenora-capabilities-v2` | CAPABILITY-DISCOVERY-2.0 |
| `--version` | `plenora-io-version-v2` | nessuna |

La colonna della fonte non è decorativa: dove un contratto fissato assegna già
un identificatore, quello **è** il nome, e non c'è niente da scegliere. Due
righe smentiscono la somiglianza — `read` rende `-read-result-v1` perché il
catalogo distingue l'ingresso dall'uscita, e l'errore è `plenora-error-v1`
**senza** `io` perché i fallimenti pubblici mappano sul contratto d'errore
comune — ed è la ragione per cui il registro sta in
`contracts/nomi-dei-contratti.json` e un gate lo confronta con il checkout
fissato invece di fidarsi della forma dei nomi.

`--version` è l'unica riga senza una fonte: non è un'operazione del catalogo ma
una superficie di scoperta della CLI, quindi il nome resta nostro ed è l'unico
caso in cui il suffisso `v2` significa ancora il protocollo.

### `plenora-io-error-v1`

L'oggetto `error` porta **esattamente sei chiavi**:

```json
{
  "status": "error",
  "protocol_version": 1,
  "contract": "plenora-io-error-v1",
  "error": {
    "category": "...",
    "phase": "...",
    "remote_effect": "...",
    "retry": { "kind": "..." },
    "code": "...",
    "message": "..."
  }
}
```

`row_diagnostics` è l'unico campo opzionale, presente quando si osservano
rifiuti per riga.

I campi `driver`, `field` e `capability_reason` esistono nel tipo Rust e **non
sul wire**.

### Il quartetto è la chiave di compatibilità

```
(category, phase, code, retry)
```

Un consumatore deve decidere su questi quattro. **`message` non è una chiave di
compatibilità**: il suo testo può cambiare senza preavviso, ed è cambiato.

Il quartetto di ogni sito di costruzione è fissato da uno snapshot e verificato
a ogni checkpoint.

### `PublicMessage` — il testo è scelto a compile time

Nessun costruttore pubblico di errore accetta testo libero. Il messaggio si
costruisce da varianti che accettano solo `&'static str` e numeri strutturali:

| Variante | Contenuto |
|---|---|
| `Curated` | un testo nostro |
| `CuratedPair` | due testi nostri |
| `CuratedWith` | un testo e un numero strutturale (indice, conteggio, limite) |
| `CuratedBetween` | due testi e due numeri |
| `Capability` | una ragione tipizzata |
| `OpzioneRifiutata` | l'unica variante con testo runtime: un token **bounded e scappato**, coniabile solo dal validatore delle opzioni |

Il valore **decodificato** di `message` non supera **2048 byte UTF-8**. Il tetto
non è promesso sul JSON serializzato, dove l'escaping espande: una virgoletta
diventa due byte, un carattere di controllo sei.

#### Che cosa questa garanzia non è

`&'static str` garantisce la **durata**, non la **provenienza**. Un chiamante
deliberato può promuovere testo runtime a `'static` con `Box::leak` e infilarlo
in un messaggio curato senza che il compilatore obietti.

La promessa realistica è quindi:

> impedire la propagazione **accidentale** di testo runtime nel workspace, non
> rendere crittograficamente inconiabile un messaggio dinamico da codice ostile.

I crate sono interni e `publish = false`: l'avversario di questo invariante è la
distrazione, non un aggressore. La documentazione del tipo porta un esempio
eseguibile che dimostra il limite invece di ammetterlo a parole.

### `ContractIdentifier` ed `ErrorContext`

Un nome di campo o di layer non entra nel testo dell'errore: entra in un campo
**tipizzato**. `ContractIdentifier` è costruibile solo da un contratto validato —
non esiste una conversione da `String` — e i nomi non attestabili sono
**rifiutati**, non troncati.

`ErrorContext` porta driver, campo e ragione di capability accanto all'errore,
per il chiamante Rust. Nessuno dei tre raggiunge il wire v1.

### Budget e limiti

Un'operazione riceve un `PipelineBudget` che porta i limiti e un **permit di
input**, consumato per `move`: la stessa sorgente non può essere osservata due
volte.

| Limite | Governa |
|---|---|
| `max_rows`, `max_columns` | cardinalità del contratto e dello stream |
| `max_input_bytes`, `max_input_entries` | il footprint della sorgente, prima di aprirla |
| `max_output_bytes` | la destinazione |
| `memory_bytes` | la memoria della pipeline. **Non** è la soglia dello spool: quella è la metà della capacità effettiva, e l'altra metà resta al batch in materializzazione |
| `spill_bytes` | i byte che lo spool può scrivere sul file temporaneo |
| `duration_ms` | la deadline dell'operazione, dalla costruzione del budget |
| `max_wkb_cell_bytes`, `max_wkb_components`, `max_wkb_depth` | ogni geometria, in lettura e in scrittura |
| `decompression_ratio` | il rapporto fra byte dichiarati e byte compressi negli archivi |

I limiti si applicano **prima** dell'allocazione che dovrebbero impedire. Un
tetto verificato dopo aver materializzato la cella non è un tetto.

Il superamento è sempre `ResourceLimit`, e il messaggio porta il numero
strutturale — conteggio e limite — non il valore letto dal file.

### Compatibilità

| | |
|---|---|
| **vincolante** | il contratto e la `protocol_version` di ciascuna busta; l'insieme delle chiavi di `plenora-io-error-v1`; il quartetto degli errori |
| **non vincolante** | il testo di `message`; l'ordine delle chiavi; l'API Rust |
| **cambio breaking** | rimuovere o rinominare una chiave, cambiare un valore del quartetto per un sito esistente, cambiare la semantica di un codice |

Un cambio breaking richiede una nuova versione di contratto, non una nota.

---

## Limitazioni di prodotto

| | |
|---|---|
| DXF | approssima per costruzione: archi ed ellissi tassellati, multipart esplose, testo come punto. La perdita è dichiarata, non silenziosa |
| Shapefile, DXF: identificatore dal WKT | il writer scrive il WKT che riceve e non l'identificatore, e dichiara `crs_id_not_preserved_derived`: l'identificatore si rilegge dal WKT. Non sempre è vero: un WKT senza autorità di radice che la regola dei lettori non riconosce (un `LOCAL_CS`, un `PROJCS` anonimo) si rilegge non risolto, o con `--assume-crs` contraddetto. La perdita è dichiarata, ma come derivata invece che assente. Hazard: un consumatore che automatizza la rilettura trova un rifiuto `crs` dove si aspettava l'identificatore; nessun CRS sbagliato. Rilevato nella rilettura 10, non corretto lì. Rientra quando la derivazione da definizione verifica, per piano, che il WKT dichiari l'identificatore (`identificatore_wkt`) |
| KML, XLSX, DXF | lettura materializzata: la libreria sottostante ha bisogno di **tutto l'input** prima della prima riga. Dove stia quell'input lo decide il buffering, che è un asse separato: non è una promessa che stia tutto in RAM. I limiti di input sono l'unica difesa, e vengono applicati prima del parser |
| DXF | un `DATATABLE` oltre 2^20 righe, colonne o celle, e un'anteprima (`THUMBNAILIMAGE`) la cui decodifica supererebbe 16 MiB, rendono il documento illeggibile (`FORMAT_ERROR`). Sono tetti espliciti del prodotto contro le prenotazioni dimensionate dall'input, **non** una garanzia di leggere ogni file AutoCAD. L'ordine delle celle del `DATATABLE` (per colonne, un valore per ogni riga) segue la specifica Autodesk come la scrive il fork ed è verificato solo in round-trip interno, non contro file AutoCAD reali: il driver non legge `DATATABLE`, ma il lettore li attraversa. In scrittura il fork accetta solo `DATATABLE` completi (un nome per colonna, un valore per ogni riga di ogni colonna, un tipo per colonna) e rifiuta gli altri con un errore; il driver scrive entità e `GEODATA`, non `DATATABLE` |
| DXF | i reali si scrivono in ASCII con dodici decimali: per le coordinate è sotto la precisione geografica di 1 cm (un dodicesimo decimale di grado è circa 1e-7 m), per altri valori reali è un arrotondamento che il formato non segnala |
| GeoParquet | dalla 4.2.0 il decoder del footer (`parquet` 60 più il fork `vendor/parquet`) lega ai byte residui ogni elenco Thrift e i row group, rifiuta uno schema più profondo di 64 livelli prima di convertirlo, e addebita ogni prenotazione -- elenchi, column chunk, percorsi delle foglie, il footer stesso -- al tetto delle pagine (metà della capacità effettiva) **prima** di farla; un footer più lungo di metà del tetto si rifiuta dalla coda. Il caso della 4.1.1 (3966 byte, `malloc(2315255472)`) è un errore. Restano fuori dal tetto le espansioni vere dei dati (un dizionario ripetuto su molte righe), che governa il budget delle pagine, e il page index, che il driver non legge. Anche i decoder delle pagine non si fidano piu' dei valori del file: lunghezze, indici di dizionario, prefissi e suffissi delta, corse RLE e livelli v2 o `BIT_PACKED` oltre i byte della pagina sono un errore invece di un panico, e i quattro casi che prima si leggevano in silenzio con un valore sbagliato (un prefisso delta oltre il valore precedente, il resto di una pagina `BYTE_STREAM_SPLIT` o FLBA `PLAIN`, una corsa RLE oltre `u32`) si rifiutano. Il delta e' lo stesso del fork di plenora-data-tools2 (`vendor/parquet`, registro `vendor-parquet-fork.json`) |
| Runtime `io.read`: vettore di richiesta | Deviazione, regola 4, da RUNTIME-VECTORS-1.0 §5 (`io-read-request`), limitata al payload illustrativo: `artifact_source`, `format` e `layer` stringa non rispettano `plenora-io-read-input-v1`, che richiede `source` e `layer` intero. Il binding rifiuta con `RUNTIME_PAYLOAD_INVALID` prima del risolutore. Conseguenza per il consumatore e hazard: copiare la fixture impedisce l'invocazione, anche con metadati validi; usare il payload dello schema IO sullo stesso riferimento consente la lettura. Rientro: allineare la fixture in plenora-contracts oppure pubblicare una v2 dello schema di ingresso. Registro: `contracts/adozione-4.0.0.json`; prove: `runtime_vettori` |
| Runtime `io.read`: vettore di cancellazione | Deviazione, regola 4, da RUNTIME-VECTORS-1.0 §3 (`io-read-cancelled-error`), per la lettura con token gia' cancellato: `CANCELLED`, fase `probe`, retry `never`, contro `READ_CANCELLED`, `read`, `safe` della fixture. Categoria `cancelled` ed effetto `none` coincidono. Il binding conserva gli assi delle operazioni comuni a CLI e Rust (SURFACE-BINDINGS-1.0 §1). Conseguenza per il consumatore: nessun retry automatico. Hazard: chi instrada per codice/fase della fixture puo' non riconoscere l'errore; chi presume `safe` puo' ritentare contro l'indicazione effettiva. Rientro: allineare la fixture in plenora-contracts oppure cambiare gli assi su tutte le superfici in una release dichiarata. Registro: `contracts/adozione-4.0.0.json`; prove: `runtime_vettori` |
| GeoParquet: compatibilita' di lettura | Il fork comune con data-tools2 `6492733` rifiuta anche file validi prima leggibili: `DELTA_BINARY_PACKED`, `DELTA_LENGTH_BYTE_ARRAY`, `DELTA_BYTE_ARRAY`, `BYTE_STREAM_SPLIT` e `BIT_PACKED` per valori e livelli non sono qualificati. Sono rifiutati anche la rappresentazione Arrow `Dictionary` di `FIXED_LEN_BYTE_ARRAY` (distinta dalla codifica Parquet a dizionario) e gli `INTERVAL` validi non rappresentabili nell'unita' Arrow scelta, anche in `PLAIN`: mesi non nulli con `DayTime` (default senza schema Arrow incorporato), giorni o millisecondi non nulli con `YearMonth`, componenti conservate unsigned oltre `i32::MAX`. Il delta esclude inoltre dizionari Arrow binari su colonne testuali (`STRING`/`UTF8`, `JSON`, `ENUM`) e tipi Arrow testuali su byte senza annotazione testuale; rifiuta hint Arrow discordanti (fuso su timestamp locale, precisione/scala decimale diversa, risoluzione diversa su timestamp locale non INT96, reinterpretazione temporale di interi annotati). Questi vincoli sulla rappresentazione si applicano anche a pagine PLAIN valide. Restano inoltre i limiti di rappresentazione del fork: FLBA di larghezza zero rifiutato; DECIMAL in BYTE_ARRAY limitato a 16/32 byte per Decimal128/256, anche con estensione del segno ridondante; statistiche FLBA piu' corte della larghezza ammesse solo con flag di esattezza esplicitamente falso (l'assenza del flag e' trattata come esatta). Il controllo di fine stream ammette solo il padding RLE/bit-packed qualificato, incluse le forme DuckDB e fastparquet, e richiede varint minimi. Questi vincoli non equivalgono al supporto di ogni rappresentazione permessa dal formato Parquet. Sono rifiutati anche `INDEX_PAGE`, lettura di page index (`ColumnIndex`/`OffsetIndex`), bloom filter e salti non qualificati (selezioni/filtri di righe, offset, salto di record/pagine). IO non usa questi ultimi percorsi; usa `with_row_groups` per il pruning. Indici/bloom presenti ma non richiesti non causano da soli il rifiuto. Per Dictionary FLBA: messaggio statico `lettura Parquet fallita`; per INTERVAL non rappresentabili e codifiche escluse nei batch: errore tipizzato `DataMapping`, fase `Read`, retry `Never`, effetto `None`, messaggio statico `batch Parquet non leggibile`, senza dati; nessun panico o risultato vuoto. Rientro: qualificare ogni percorso nel fork condiviso (fuzz e regressioni), adottare la base qualificata e verificarla al confine IO. Vedi [note della prossima release](RELEASE.md#note-della-prossima-release) |
| IPC | **una sola colonna geometrica per layer.** Il contratto comune (ARROW-INTERCHANGE-1.0) non fissa un numero di colonne geometriche; il modello di questo componente porta un solo contratto geometrico per layer. Un Arrow IPC in ingresso con due o più colonne GeoArrow è rifiutato all'apertura con `unsupported` (GEO-012, REJ-001), anche quando lo schema è esterno, senza versione né chiavi `plenora.*`, e dopo i difetti di vocabolario di ogni colonna geometria (REJ-002) (prova `multiple_geoarrow_fields_are_rejected` in `driver-ipc`); in scrittura vale lo stesso rifiuto, `unsupported`, per un layer con più colonne GeoArrow e nessun contratto geometrico esplicito (prova `ambiguous_or_invalid_legacy_geometry_metadata_is_rejected` in `plenora-io-core`). Fino alla 5.x entrambi erano `invalid_plan`, che REJ-001 esclude. Hazard: nessun dato si perde in silenzio, ma una catena verso `io.write` con due geometrie fallisce invece di comporsi. In GeoParquet la colonna primaria è la geometria del layer; le altre colonne geometriche del file (le secondarie del documento `geo` e i campi che lo schema Arrow incorporato marca come geometria) sono **nominate** nella scoperta, in `native_metadata["geoparquet.altre_colonne"]`, e una lettura che le comprende — senza projection, o con una projection che ne seleziona una — è rifiutata con `unsupported` prima di riaprire il file (GEO-012: nessuna geometria degradata a binario, nessuna scelta al posto di chi legge; prove `due_geometrie_senza_selezione_sono_rifiutate_come_non_supportate` e seguenti in `driver-geoparquet`). Si legge proiettando la sola primaria, o nessuna geometria. Rientra con un contratto multi-geometria per layer, che oggi nessuna versione pubblicata prevede |
| IPC | **il CRS nella sola forma GeoArrow standard non è letto.** Il CRS si legge dalla chiave di campo `crs` e dal blocco `plenora.geometry.*` (ARROW-INTERCHANGE-1.0 §4), non da `ARROW:extension:metadata`. Un file che porta il CRS **solo** lì, come lo scrivono i produttori GeoArrow esterni, è letto con CRS `missing`, senza errore (prova `crs_only_in_geoarrow_extension_metadata_is_read_as_missing` in `driver-ipc`). Hazard: lo stato `missing` è dichiarato nel contratto e i sink che pretendono un CRS rifiutano, ma un sink che lo ammette scrive una geometria senza il CRS che la sorgente aveva. I file di plenora-data-tools e plenora-database-tools portano il blocco `plenora.geometry.*` e non sono toccati. Rientra quando `ARROW:extension:metadata` viene letto, o quando un CRS presente solo lì viene rifiutato con un errore esplicito |
| IPC | **solo i sette tipi semplici e la semantica `geometry`.** Il sink IPC (`WKB_PASSTHROUGH_GEOMETRY`) dichiara Point, LineString, Polygon, i tre Multi e GeometryCollection: una colonna che dichiara CircularString, Triangle o altri tipi estesi, o la semantica `geography`, si rifiuta con `unsupported` prima di ogni effetto (GEO-000, verdetto di supporto; GEO-010). Lo stesso valore dentro una GeometryCollection resta un valore e segue GEO-011. Il catalogo dichiara lo stesso insieme. Rientro: un sink IPC che dichiari i tipi estesi. |
| tutti: identificatore di radice di un WKT | lo SRID di radice di un WKT (capability e perdita di fedeltà) si legge con la stessa grammatica della verifica GEO-005, WKT 1 o WKT 2: `AUTHORITY` in WKT 1, `ID` in WKT 2, solo alla radice, con parentesi quadre o tonde. Fino alla 5.x uno scanner di byte più largo contava solo le parentesi quadre (un CRS annidato fra tonde esponeva il suo identificatore come di radice), trovava `ID[` dentro `XID[` e accettava le parole dell'altra grammatica: due letture della stessa radice davano due CRS. Un WKT non ben formato non ha SRID di radice. **L'identificatore che lo Shapefile (`.prj`) e il DXF (`GEODATA` in WKT) danno a una definizione WKT, con o senza `--assume-crs`, lo decide una regola sola** (`crs_definizione::identificatore_wkt` in `plenora-io-model`), sull'albero del lettore stretto: (1) il testo si legge nella grammatica WKT 1 o WKT 2 in cui è ben formato; malformato in entrambe è `crs`, con o senza opzione, e `io.inspect` e `io.read` rispondono allo stesso modo; (2) le dichiarazioni sono gli identificatori della radice con la parola della grammatica **più** quelli con la parola dell'altra (`ID` in WKT 1, `AUTHORITY` in WKT 2), che per GEO-005 non dichiarano ma chi legge il file legge come dichiarazioni; un nodo malformato dell'una o dell'altra parola (`ID[EPSG,3003]`, `ID["EPSG"]`, `ID["EPSG",3003.5]`) rende la radice indecidibile: `crs`; (3) una radice `BOUNDCRS`, riconosciuta dalla parola della radice con ogni spaziatura e maiuscola, non si risolve mai: senza opzione è non risolta, con l'opzione è `crs`; (4) due dichiarazioni diverse (autorità ASCII case-insensitive, codice come testo) sono `crs`, con o senza opzione; (5) una dichiarazione: senza opzione si risolve solo un `EPSG` con codice intero canonico, come `EPSG:<codice>`, e ogni altra autorità (`ESRI`, `OGC:CRS84` di radice) resta non risolta; con l'opzione, l'opzione deve essere uguale alla dichiarazione, e l'identificatore ha la grafia del file; (6) nessuna dichiarazione: la forma ESRI di WGS 84 **intera** — `GEOGCS[nome, DATUM[nome, SPHEROID[nome, 6378137, 298.257223563]], PRIMEM["Greenwich", 0], UNIT["degree", 0.0174532925199433]]` e nessun altro nodo (né `AXIS`, né `TOWGS84`, né un `AUTHORITY` annidato), con i nomi `WGS 84` o `GCS_WGS_1984` per la radice, `WGS_1984` o `D_WGS_1984` per il datum, `WGS 84` o `WGS_1984` per l'ellissoide, interi e ASCII case-insensitive, e i numeri confrontati come decimali esatti (`6378137.0` vale `6378137`; una forma esponenziale o `0.017453292519943295` non sono riconosciute) — è `EPSG:4326` senza opzione, e con l'opzione ammette solo `EPSG:4326` e `OGC:CRS84`; ogni altra definizione resta non risolta senza opzione, e con l'opzione l'opzione la nomina, salvo un identificatore che il bordo sa di genere contrario alla radice (riga «tutti: genere del CRS»). Le dichiarazioni (2) si confrontano ciascuna con la prima, in tempo lineare. Fino a c0e84cb la forma ESRI si riconosceva dai due soli nomi (`GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["GRS 1980",...]],PRIMEM["Paris",2.337],UNIT["grad",...]]` e `GEOGCS("WGS 84",DATUM("WGS_1984"))` erano `EPSG:4326`), la grafia di ArcGIS no, e le dichiarazioni si deduplicavano in tempo quadratico. Fino a a651844 ogni driver decideva con più letture parziali del testo, e sette riletture di fila ne trovarono una nuova che dava un'etichetta sbagliata senza errore: un `ID["EPSG",4258]` alla radice del WGS 84 ESRI usciva `EPSG:4326`; `AUTHORITY["EPSG","3003"],ID["EPSG",3004]` usciva `EPSG:3003`; un `ID` malformato lasciava passare l'opzione; nel DXF `--assume-crs EPSG:4326` passava su un `ID` in WKT 1, un'autorità ESRI, un `BOUNDCRS`, un `AUTHORITY` in WKT 2; il `DATUM["WGS_1984"` si cercava ovunque nel testo; un `.prj` malformato dava a `io.inspect` un identificatore che `io.read` poi rifiutava. Il formato di una definizione (`wkt` o `wkt2`) si decide con lo stesso lettore, WKT 2 e poi WKT 1; il prefisso maiuscolo esatto (`PROJCRS[`) resta solo per un testo che non è ben formato in nessuna delle due: prima `PROJCRS [`, `projcrs[` o `PROJECTEDCRS[` erano `wkt` e la verifica GEO-005 li rifiutava. Hazard residuo (6): il bordo non è un resolver, e un'opzione su una definizione senza dichiarazioni e diversa dalla forma ESRI intera di WGS 84 la nomina anche quando la contraddice, purché il genere non si contraddica in modo visibile: `EPSG:3003` su un UTM anonimo, `EPSG:4258` su un geografico anonimo qualunque, `EPSG:32632` su un altro proiettato. Si rifiuta soltanto un identificatore di cui il bordo conosce il genere (`EPSG:4326` e `OGC:CRS84` geografici, `EPSG:3857` proiettato) su una radice del genere opposto o che non è un CRS orizzontale (riga «tutti: genere del CRS»). Rientra con un resolver CRS. Due divergenze conservative dichiarate: un `.prj` vuoto o di soli spazi è `crs` anche con l'opzione (non è un WKT ben formato); un `ID["EPSG",4326.0]` in un WKT 1 è `crs` perché la regola (2) lo legge come dichiarazione malformata, mentre per GEO-005 e per l'oracolo in un WKT 1 `ID` non è un identificatore e si ignora. Sono rifiuti in più, mai un CRS sbagliato. Prove: `regola_wkt_senza_dichiarazioni_e_forma_esri`, `regola_wkt_con_una_dichiarazione`, `regola_wkt_con_l_altra_parola_chiave`, `regola_wkt_ambigua_o_legata`, `regola_wkt_malformata`, `definition_format_decide_la_grammatica_con_il_lettore`, `il_prj_segue_la_regola_unica_dell_identificatore`, `il_wkt_del_geodata_segue_la_regola_unica`, `inspect_e_read_concordano_su_un_prj_malformato`, `definition_epsg_rejects_ambiguous_or_nested_only_ids`, `una_definizione_wkt_multibyte_non_fa_panicare_il_bordo` |
| tutti: genere del CRS | il genere (geografico, proiettato, ignoto) che sceglie fra `lon_lat` ed `easting_northing` lo decide una funzione sola, `plenora_io_model::crs::crs_kind_for`, per Shapefile, DXF, GeoPackage, GeoParquet, CSV, XLSX e la rilettura dei metadati Arrow IPC (FileGDB lo chiede a GDAL). La definizione decide con la parola della **radice** letta dal lettore stretto (`crs_definizione::genere_radice`): `GEOGCS`, `GEOGCRS`, `GEOGRAPHICCRS` sono geografici, `PROJCS`, `PROJCRS`, `PROJECTEDCRS` proiettati, `GEODCRS` e `GEODETICCRS` geografici solo con un `CS[ellipsoidal,...]` figlio diretto; in PROJJSON decide il `type` della radice (`GeodeticCRS` solo con `coordinate_system.subtype` `ellipsoidal`). Ogni altra radice — `GEOCCS`, `GEODCRS` senza `CS[ellipsoidal,...]`, `LOCAL_CS`, `ENGCRS`, `VERT_CS`, `VERTCRS`, `COMPD_CS`, `COMPOUNDCRS`, `DERIVEDPROJCRS`, `FITTED_CS`, `TIMECRS`, `PARAMETRICCRS` e gli stessi `type` PROJJSON — ha genere **ignoto** con un identificatore che il bordo non conosce, ed è `crs` con uno di quelli che conosce (`EPSG:4326`, `OGC:CRS84`, `EPSG:3857`, in qualunque maiuscola), dichiarato o nominato da `--assume-crs`: quell'identificatore nomina un CRS orizzontale 2D, e la radice dice che non lo è. Fino a b4d8064 il genere restava ignoto e il CRS usciva **risolto**: un `.prj` `LOCAL_CS["site",LOCAL_DATUM["d",0],UNIT["metre",1],AXIS["X",EAST],AXIS["Y",NORTH]]` con `--assume-crs EPSG:4326` era `EPSG:4326`. Un `BOUNDCRS` (`BoundCRS` in PROJJSON) resta ignoto qualunque sia l'identificatore, come prima: non ha un genere proprio, e `--assume-crs` su di lui è già `crs` (regola (3) dell'identificatore). Una definizione malformata resta ignota qui e la rifiuta la verifica GEO-005. Un genere ignoto dà sempre `axis_order` `unknown`. Senza definizione, o con un PROJJSON senza `type`, decide l'identificatore, solo se il bordo lo conosce già: `OGC:CRS84` ed `EPSG:4326` geografici, `EPSG:3857` proiettato. Un identificatore noto di genere contrario alla radice — `EPSG:4326` o `OGC:CRS84` su un `PROJCS`, dichiarato o nominato da `--assume-crs`; `EPSG:3857` su un `GEOGCS`; un `id` EPSG:4326 su un `type` `ProjectedCRS` — è `crs`. Fino a c0e84cb ogni driver cercava `PROJCS[` e `GEOGCS[` nel testo, con un suo elenco: da quando `PROJECTEDCRS[`, `projcrs [` e `PROJCRS(` si leggono come `wkt2` gli assi uscivano est/nord e il genere geografico (il `BASEGEOGCRS[` annidato contiene `GEOGCRS[`), cioè `lon_lat` su dati UTM, e la conversione in IPC lo scriveva (Shapefile EPSG:32632, GeoPackage 3003, DXF); già prima `PROJCS [`, `PROJCS(` e `PROJCS\t[` uscivano geografici, `GEOGCS["PROJCS[",...]` proiettato, un composto con un `PROJCRS[` dentro proiettato e, nel DXF, un `GEODCRS` geocentrico geografico; in GeoPackage e GeoParquet l'identificatore `EPSG:4326` vinceva sulla definizione; CSV e XLSX distinguevano le maiuscole (`epsg:4326` ignoto). Hazard: nessun genere sbagliato dalla radice; un `GEODCRS` ellissoidale senza `CS` e un PROJJSON con un altro `type` restano ignoti, un'informazione in meno. Prove: `il_genere_viene_dalla_parola_della_radice`, `il_genere_segue_la_radice_e_rifiuta_l_identificatore_contrario`, `il_genere_del_prj_viene_dalla_radice`, `assume_crs_di_genere_contrario_alla_radice_e_rifiutato`, `il_genere_del_geodata_viene_dalla_radice`, `assume_crs_geografico_su_un_proiettato_anonimo_e_rifiutato`, `il_genere_della_riga_srs_viene_dalla_radice`, `il_genere_del_documento_crs_viene_dal_type`, `un_identificatore_orizzontale_su_una_radice_che_non_lo_e_si_rifiuta`, `un_identificatore_orizzontale_su_un_prj_non_orizzontale_e_crs`, `un_identificatore_geografico_su_un_local_cs_e_crs`, `geodata_local_cs_e_bom`, `il_genere_dell_opzione_segue_la_regola_comune` (CSV e XLSX: `epsg:4326` minuscolo geografico, `EPSG:3857` proiettato) |
| tutti: GEO-005 e GEO-020 in lettura | il CRS che un driver emette passa, in `with_read_budget` e per ogni driver, le verifiche che la scrittura Arrow IPC fa sul campo (`crs::verifica_crs_emesso`): una definizione malformata nel formato che dichiara, o con un identificatore di radice dell'autorità di `crs_id` diverso da `crs_id`, e un identificatore di catalogo locale dichiarato risolto (`POSTGIS:4326` di un GeoPackage) sono `crs` già in `io.inspect` e `io.read`. Fino a c0e84cb li rifiutava solo la scrittura IPC, e una conversione verso un altro formato li portava con sé. Le verifiche sono **del layer** (riga «più layer: un layer rifiutato»): in un GeoPackage a più layer il layer con la riga di `gpkg_spatial_ref_sys` malformata o di un catalogo locale non si legge, gli altri sì. Hazard: quel layer non si legge più, dove prima si leggeva e non si convertiva in IPC. Prove: `with_read_budget_rifiuta_una_definizione_che_geo_005_rifiuta`, `verifica_definizione_emessa_applica_geo_005` |
| più layer: un layer rifiutato | GeoPackage. Le verifiche del CRS di `with_read_budget` (`--assume-crs`, GEO-005, GEO-020) e la risoluzione del CRS del GeoPackage (`crs_for`: organizzazione `NONE`, riga di `gpkg_spatial_ref_sys` mancante, genere contraddetto, definizione oltre il tetto) rifiutano **il layer**, non il dataset (`OpenDatasetHandle::rifiuto_del_layer`). `io.read --layer` e `io.convert --layer` su un layer sano procedono; sul layer rifiutato, e `io.convert` senza `--layer` che lo includerebbe, falliscono con il suo rifiuto prima di preparare il sink. `io.inspect` e `io.layers`, che descrivono **tutti** i layer, falliscono con il rifiuto del primo layer rifiutato: elencarlo come leggibile, con un `geometry_crs` di cui il bordo non risponde, sarebbe una descrizione falsa, e i loro contratti non hanno un campo per dire «rifiutato». Se i layer rifiutati sono tutti, o il layer è uno solo, l'apertura fallisce con il primo rifiuto, come prima. Fino a b4d8064 il primo rifiuto chiudeva il dataset: una riga `'+proj=longlat +datum=WGS84'` del layer 0 rendeva illeggibile il layer 1. Hazard: `io.inspect` e `io.layers` non dicono quale layer è rifiutato (il messaggio è quello del rifiuto); lo dice `io.read --layer`, layer per layer. Fuori dal perimetro: il FileGDB risolve il CRS di ogni layer con GDAL all'apertura, e un layer senza CRS e senza `--assume-crs` chiude ancora il dataset (rifiuto in più, mai un CRS sbagliato; rientra portando `resolve_layer_crs` sul rifiuto del layer); il DXF e gli altri driver hanno un layer solo. Restano del dataset il nome di colonna geometrica ambiguo e i difetti strutturali di un GeoPackage (`srs_id` fuori da `i32`, alias del rowid oscurato). Prove: `un_layer_dal_crs_rifiutato_non_chiude_gli_altri`, `se_tutti_i_layer_sono_rifiutati_l_apertura_fallisce`, `un_layer_rifiutato_ferma_la_descrizione_di_tutti_i_layer` |
| tutti: tetto della definizione | **ogni** definizione CRS oltre 1 MiB (`crs_definizione::MAX_BYTE_DEFINIZIONE_CRS`), da qualunque sorgente, è `limit_exceeded`: il `.prj`, il `GEODATA`, la riga di `gpkg_spatial_ref_sys` (che `SQLite` non consegna oltre una guardia di due volte il tetto nella codifica del database, e il tetto esatto si applica poi sui byte UTF-8: fino alla rilettura 10 la guardia era il tetto stesso nella codifica del database, e in un GeoPackage UTF-16 una definizione ASCII di 600 KB si rifiutava sotto il tetto), il `crs` di GeoParquet, il metadato `plenora.geometry.crs_definition` di un campo Arrow IPC (anche in scrittura, che lo rilegge), la definizione di FileGDB. La regola è una: la applicano `crs_kind_for`, `verifica_crs_emesso`, la lettura dei metadati Arrow e la verifica dello schema IPC (`vocabolario::verifica_schema`), sempre **prima** del lettore stretto (`verifica_dimensione_definizione`), che non apre un testo più lungo e lo direbbe malformato. Fino alla rilettura 10 la verifica dello schema IPC chiamava il lettore prima del tetto, e una definizione di 1 MiB e un byte era `crs` «non ben formata» invece di `limit_exceeded`. Il lettore tiene un token per elemento, qualche decina di byte per byte d'ingresso nel caso peggiore. Fino a c0e84cb il `.prj` si leggeva intero; fino a b4d8064 le definizioni di un metadato erano limitate solo dai byte del metadato (16 MiB in Arrow IPC): una riga di `gpkg_spatial_ref_sys` di 8 MiB costava 698 MB di picco e 12 s in debug, una di 32 MiB esauriva la scadenza. Deviazione dichiarata (regola 4): ARROW-VOCABULARY-1.0 non pone un tetto alla definizione, e un campo IPC conforme con una definizione oltre 1 MiB è un rifiuto di supporto (`limit_exceeded`), mai un risultato diverso; rientra con un lettore che non materializzi i token. La stessa definizione si legge ancora più volte per apertura (formato, genere, assi, GEO-005): il costo è lineare e limitato dal tetto, e non è stato ridotto. Prove: `un_prj_oltre_il_tetto_e_un_limite`, `una_definizione_del_geodata_oltre_il_tetto_e_un_limite`, `una_definizione_oltre_il_tetto_e_un_limite`, `la_definizione_della_riga_srs_ha_il_tetto`, `il_tetto_della_definizione_e_in_utf8_anche_in_un_database_utf16`, `la_definizione_di_un_metadato_ha_il_tetto_comune`, `una_definizione_oltre_il_tetto_e_un_limite_non_un_crs_malformato` |
| tutti: BOM della definizione | un BOM UTF-8 iniziale (uno solo) di un `.prj`, di una riga di `gpkg_spatial_ref_sys` o di un `GEODATA` si toglie all'ingresso (`crs_definizione::senza_bom`): non fa parte del WKT, e la definizione emessa è quella senza. Fino a b4d8064 la definizione si leggeva malformata. Un secondo BOM resta, e la definizione resta malformata. Più `GEODATA` si confrontano **dopo** aver tolto il BOM: fino alla rilettura 10 si confrontavano prima, e due definizioni uguali, una con il BOM, erano un conflitto `crs`. Prove: `un_prj_con_il_bom_si_legge`, `un_bom_iniziale_della_definizione_si_toglie`, `geodata_local_cs_e_bom`, `due_geodata_uguali_a_meno_del_bom_concordano` |
| GeoParquet: `crs: null` | `null` è un CRS ignoto (GeoParquet 1.1), e nel vocabolario è `missing`, l'unico stato senza identificatore né definizione (ARROW-VOCABULARY-1.0 §4). Fino a b4d8064 era un `resolved` senza nessuno dei due: `io.read` lo emetteva e la conversione in IPC lo rifiutava («metadato CRS non valido»). Ora lettura e conversione concordano, e un `missing` si riscrive in GeoParquet come `null` (dalla rilettura 10: fino a b7a6c94 la validazione comune lo rifiutava, `unsupported` «il formato richiede un CRS risolto», contro ciò che questa riga diceva; prova end-to-end `un_crs_ignoto_si_riscrive_in_geoparquet_come_null`). Con `--assume-crs` è `crs` («non applicato»), come prima era `crs` («non confrontabile»). Prova: `un_crs_mancante_si_scrive_null_e_si_rilegge_sconosciuto` |
| Arrow IPC: genere riletto | il genere di un CRS `resolved` riletto dai metadati segue `crs_kind_for` (identificatore e radice della definizione); solo se la definizione manca e l'identificatore è ignoto decide l'ordine memorizzato (`lon_lat`/`lat_lon` geografico, `easting_northing`/`northing_easting` proiettato). Fino a b4d8064 decideva sempre l'ordine, e un `EPSG:3031` polare, proiettato per lo Shapefile con assi `other`, si rileggeva ignoto. Un identificatore che contraddice la radice qui lascia il genere ignoto senza rifiutare: il genere non è un campo del vocabolario, e il campo è quello che GEO-005 ha già giudicato. Prova: `il_genere_riletto_segue_la_regola_comune` |
| tutti: WKB ben formato (GEO-011) | ogni WKB o EWKB che IO decodifica, ispeziona o codifica rispetta la buona forma di Arrow Geometry Semantics 1.0 come la decide l'oracolo dei contratti (`arrow_data.py`): una linestring ha zero o almeno due punti, una circularstring zero o un numero dispari di almeno tre, un anello lineare di poligono o triangolo almeno quattro punti con l'ultimo uguale al primo **byte per byte** (Z e M comprese: `-0.0` non chiude `0.0`, un NaN con gli stessi bit chiude), un triangolo al più un anello, al più 32 livelli di annidamento, e ogni membro di un aggregato ha le dimensioni del contenitore. Il rifiuto è del codec stesso -- `decode_wkb`, `inspect_wkb` e l'encoder -- e non dei percorsi che lo consultano dopo: fino alla 5.0.1 il membro di altre dimensioni era solo annotato (`nested_dimensions_coherent`) e rifiutato a valle, dalla validazione dei batch e da quella delle scritture, e un percorso che non leggesse il flag lo avrebbe accettato o emesso (prova `un_membro_di_altre_dimensioni_e_malformato_nel_codec` in `wkb_lossless/tests.rs`). Vale per **ogni formato**, non solo per l'ingresso Arrow: un valore che un componente del contratto rifiuterebbe non esce da IO. Ambito: un file nativo con una polilinea di un vertice, un anello non chiuso o vuoto (KML, GeoJSON, DXF, Shapefile, GeoPackage scritti da altri) è rifiutato con `data_mapping` invece di essere letto; fino alla 5.x passava. Hazard: file prima leggibili ora falliscono, con un errore esplicito e nessun dato pubblicato; nessuna chiusura o riparazione implicita. Rientro: nessuno previsto, è la conformità; una riparazione esplicita sarebbe un'opzione nuova e dichiarata. Prove: `geo_011_*` in `plenora-io-model` (`wkb/tests.rs`), `un_valore_malformato_con_un_altro_srid_e_data_mapping` in `plenora-io-tools` Un membro che porta insieme uno SRID e dimensioni diverse dal contenitore è rifiutato per le dimensioni (GEO-011) dove l'oracolo cita prima lo SRID del membro (GEO-009): la categoria, `data_mapping`, è la stessa, cambia solo la causa di riga riportata. |
| tutti: `max_wkb_depth` | la profondità di GEO-011 (32) non è una quota: il parser la applica qualunque sia la configurazione, e `max_wkb_depth` (`--max-wkb-depth`, budget runtime) può solo stringerla. Un valore oltre 32 si rifiuta alla validazione dei limiti (`limit_exceeded`) invece di restare un flag senza effetto; il default passa da 64 a 32. Hazard: una configurazione che chiedeva 64 ora fallisce. Prove: `geo_011_l_annidamento_si_ferma_a_32_qualunque_sia_il_tetto`, `max_wkb_depth_oltre_il_contratto_si_rifiuta` |
| CRS: PROJJSON (GEO-005) | gli identificatori di un PROJJSON si leggono con un lettore dedicato che replica `json.loads` con i ganci dell'oracolo: numeri di qualunque grandezza (`1e400`), surrogati isolati, codici `-0` (vale `0`) o oltre `u64`, qualunque annidamento. Restano tre differenze, dove l'oracolo in CPython non dà un verdetto perché si ferma con un'eccezione dell'interprete e non con `Malformed`: `RecursionError` oltre circa mille livelli, `ValueError` per un intero di più di 4300 cifre, `decimal.InvalidOperation` per un decimale con un esponente oltre i limiti di `decimal` (`9e9999999999999999999999999999`). Lì IO dà il verdetto della grammatica, perché legge i numeri come testo senza valutarli: il documento è ben formato, un intero lungo è un codice valido, un decimale -- con qualunque esponente -- come `code` è malformato e altrove è un valore qualunque (prove `projjson_*` in `crs_definizione/tests.rs`). **Memoria**: lineare nei byte del documento con un fattore dichiarato -- un byte per contenitore aperto, 16 byte per chiave di un oggetto aperto (liberati alla sua chiusura), della radice soltanto `id` e `ids` e di ogni identificatore soltanto `authority` e `code`, in UTF-8. Fino alla 5.0.1 la verifica teneva un `HashSet` per oggetto aperto e 48 byte per contenitore, e la lettura materializzava ogni membro della radice fino al terzo livello. Misurato in release su documenti di 16 MiB, picco delle allocazioni prima e dopo: 403 MB e 8 MB (`{"a":` e 8 milioni di `[`), 591 MB e 71 MB (3,3 milioni di `{"":` annidati, il caso peggiore: 4,3 volte i byte a 16 MiB, fino a circa 6,8 volte su dimensioni dove le capacità dei vettori raddoppiano appena oltre il necessario, con un tetto assoluto di circa 71 MB fino a 16 MiB), 277 MB e meno di 1 KB (8 milioni di numeri in una lista), 170 MB e 34 MB (2,4 milioni di chiavi in un oggetto), 233 MB e 56 MB (600 mila `ids`, quasi tutto il risultato). Il documento è già limitato dai byte del metadato che lo porta. Il doppione di chiave si cerca alla chiusura dell'oggetto, su impronte SipHash a chiavi casuali e con il confronto esatto delle sole chiavi di impronta uguale: il verdetto non dipende dall'impronta (prova `le_impronte_uguali_non_cambiano_il_verdetto`). Lo **SRID** ricavato da un PROJJSON (`definition_authority_srid`, usato dalle capability e dal confronto delle rappresentazioni nel `LossReport`) usa ora lo stesso lettore: un codice numerico è un intero, un codice stringa è testo e vale solo se canonico, come nel WKT. Fino alla 5.0.1 una stringa passava da `str::parse`, e `"04326"` o `"+4326"` davano 4326 mentre lo stesso codice in un WKT non dava SRID (prova `definition_authority_srid_projjson_distingue_il_numero_dalla_stringa`). Gli **altri** usi di un PROJJSON in IO -- l'ordine degli assi dichiarato verso un formato con l'est prima, il documento `crs` di GeoParquet -- leggono ancora con `serde_json` senza `arbitrary_precision`: per un documento con un numero oltre `f64`, un surrogato isolato o più di 128 livelli l'ordine resta `unknown`, e GeoParquet tratta la definizione come una che non è PROJJSON (come un WKT): con `EPSG:4326` o `OGC:CRS84` omette il `crs`, che per il formato vale CRS84, e dalla rilettura 10 dichiara la definizione perduta (`crs_definition_not_preserved_absent`), altrimenti rifiuta con `unsupported`. Hazard: nessun risultato sbagliato, un rifiuto o un'informazione in meno. Rientro: portare quei due usi sul lettore dedicato. Prove: `projjson_*` in `crs_definizione/tests.rs` |
| CRS: spazi e maiuscole | lo spazio della grammatica di `crs_id` (GEO-016) e del WKT è `str.isspace()` di Python 3.11 (Unicode 14), scritto come elenco chiuso: il `White_Space` di Unicode più U+001C..U+001F. Le autorità si confrontano ASCII case-insensitive, come scrivono GEO-005 e GEO-020, anche nella lettura di `gpkg_spatial_ref_sys`: `EPſG` (U+017F) è un'altra autorità. L'oracolo al pin 3aec9f7 usa `str.upper()` e qui diverge; la correzione dell'oracolo è plenora-contracts #53, e nessun vettore copre il caso. Hazard: nessuno per autorità ASCII. Prove: `lo_spazio_e_quello_di_python`, `l_autorita_si_confronta_solo_in_ascii`, `reader_uppercases_the_organization_in_ascii_only` |
| GeoPackage: codice CRS non canonico | `srs_id` e `organization_coordsys_id` sono interi: un `crs_id` il cui codice non è un intero canonico (`EPSG:04326`, `EPSG:+4326`) non ha un intero che lo rappresenti senza cambiarne l'identità (GEO-005 confronta i codici come testo), e si rifiuta con `crs` prima di creare la destinazione. Fino alla 5.x diventava 4326 e si rileggeva `EPSG:4326`. Lo stesso vale per lo SRID ricavato da un identificatore (`authority_srid`, GeoParquet) e per un codice **stringa** di un `AUTHORITY` WKT: `"04326"` non diventa 4326; un codice **numerico** resta un intero, come per l'oracolo. Hazard: un identificatore così non si scrive in GeoPackage. Prove: `gpkg_rifiuta_un_codice_non_canonico_invece_di_cambiarne_l_identita`, `authority_srid_solo_per_un_codice_canonico` |
| FileGDB | richiede `gdal-backend`. Senza, ogni chiamata fallisce come capability mancante |
| FileGDB | **si apre con il solo driver GDAL `OpenFileGDB`.** Una directory `.gdb` che GDAL saprebbe aprire con un altro driver (per esempio una directory di CSV) è rifiutata invece di essere letta con le regole di un altro formato (prova `una_directory_gdb_che_non_e_un_filegdb_non_si_apre`). Un layer con più campi geometrici resta enumerato, ne dichiara il numero in `native_metadata["filegdb.geometry_field_count"]`, e la sua lettura è rifiutata con `unsupported` (GEO-012; prova `piu_campi_geometrici_rendono_il_layer_non_supportato`): prima si prendeva il primo campo e gli altri sparivano. Rientra con un contratto multi-geometria per layer |
| GeoPackage | **una sola colonna geometrica per feature table.** Una tabella con più righe in `gpkg_geometry_columns` è rifiutata all'apertura con `unsupported` (GEO-012; prova `due_colonne_geometriche_per_la_stessa_tabella_sono_rifiutate`): prima ogni riga diventava un layer con lo stesso nome, e in ciascuno l'altra geometria usciva come BLOB. «La stessa tabella» e «la colonna geometrica» si giudicano come li risolve SQLite, senza distinzione di maiuscole ASCII: `features` e `FEATURES` in `gpkg_contents` sono una tabella sola (il join con `gpkg_geometry_columns` usa `NOCASE`), e un `column_name` `GEOM` è la colonna `geom`. Fino alla 5.0.1 il confronto era esatto: la stessa tabella si apriva come due layer, e una colonna geometrica dichiarata con altre maiuscole usciva una seconda volta come attributo binario (prove `due_geometrie_sotto_nomi_di_tabella_che_differiscono_per_maiuscole_sono_rifiutate`, `la_colonna_geometrica_si_riconosce_come_la_riconosce_sqlite`). Hazard: il rifiuto chiude l'intero file, anche le altre tabelle. Rientra con un contratto multi-geometria per layer |
| GeoPackage: layer dichiarati | una riga `features` di `gpkg_contents` senza la sua riga in `gpkg_geometry_columns` (GPKG 1.3, requisito 22), e nel verso opposto una colonna di `gpkg_geometry_columns` la cui tabella non è una riga `features` di `gpkg_contents` (assente, `attributes`, o `Features` scritto con un'altra grafia), rendono il file non leggibile: errore `data_mapping` all'apertura, invece di un elenco di layer con quella tabella mancante. Fino alla 5.x quei layer sparivano in silenzio. Le tabelle `attributes` senza geometria non sono layer di IO e non si elencano: IO legge feature table. Un file con righe orfane in `gpkg_geometry_columns` (lasciate, per esempio, da un `DROP TABLE` fatto a mano) si rifiuta per intero, dove altri lettori le ignorano: è il requisito della specifica, ed è un rifiuto in più. Prove: `una_feature_table_senza_colonna_geometrica_dichiarata_e_rifiutata`, `una_geometria_dichiarata_fuori_da_una_feature_table_e_rifiutata` |
| GeoParquet senza metadato `geo` | la geometria si riconosce dal nome (`geometry`, `geom`, `wkb`); con **più** di quei nomi nel file l'apertura è rifiutata con `unsupported` invece di prendere il primo dell'elenco (prova `un_parquet_senza_geo_con_due_nomi_di_geometria_e_ambiguo`). Il CRS è `missing`, oppure quello che `--assume-crs` nomina, con ordine degli assi `unknown` (la convenzione longitudine prima è del metadato `geo`): il default `OGC:CRS84` di GeoParquet 1.1 vale per una colonna dichiarata nel metadato senza la chiave `crs`, e un Parquet senza `geo` non è GeoParquet. Fino alla 5.0.1 il driver lo etichettava WGS84 (prova `un_parquet_senza_geo_non_afferma_un_crs`). Hazard: un sink che pretende un CRS ora rifiuta questi file finché l'opzione non lo nomina |
| CSV, XLSX | `wkt_column` esclude `x_column` e `y_column`: dichiararli insieme è un errore di configurazione (`invalid_configuration`), non più una precedenza silenziosa del WKT (prove `wkt_column_insieme_a_x_y_e_una_configurazione_ambigua` in `driver-csv`, `n1_resolve_geometry_nomina_la_colonna_che_manca_e_rifiuta_due_geometrie` in `driver-xls`). Un nome di `wkt_column`, `x_column` o `y_column` che l'intestazione ripete è ambiguo ed è rifiutato con `data_mapping`: prima la geometria usciva dalla prima occorrenza, scelta in silenzio, e le altre diventavano attributi (prove `una_colonna_geometrica_dal_nome_ripetuto_e_rifiutata`, `n1_resolve_geometry_rifiuta_un_nome_che_l_intestazione_ripete`) |
| tutti | una colonna geometrica il cui nome compare **più volte** nello schema del layer è rifiutata all'apertura con `schema`, in `with_read_budget`, per ogni driver: il contratto nomina la geometria per nome, e la validazione dei batch, il conteggio dei componenti e gli esempi di perdita la cercano per nome, trovando la prima occorrenza. Un Arrow IPC con un campo `g` binario qualunque prima del campo `g` GeoArrow validava il campo sbagliato, e un GeoParquet con due colonne `geometry` le esponeva entrambe come geometria (prova `with_read_budget_rifiuta_una_colonna_geometrica_dal_nome_ripetuto`). Hazard residuo: due **attributi** con lo stesso nome restano leggibili, come la sorgente li porta, e chi li consuma per nome trova il primo; la scrittura li rifiuta già (`collisione dopo la normalizzazione`). Rientra con una regola di unicità dei nomi nel contratto |
| GeoJSON | il membro `crs` di GeoJSON 2008, su collezione, feature, geometria o figlia di una `GeometryCollection`, è ammesso solo se è un CRS con nome che nomina CRS84 o EPSG:4326, in forma breve, URN OGC (con o senza versione) o URI `opengis.net`; ogni altro nome, un CRS per link, `null`, una forma malformata o con chiavi ripetute o estranee è rifiutato con `crs`. Prima il membro si ignorava e il documento usciva `OGC:CRS84` qualunque CRS dichiarasse (prova `il_membro_crs_storico_deve_nominare_wgs84`). EPSG:4326 si accetta come CRS84 perché GeoJSON 2008 fissa l'ordine x, y, cioè longitudine prima, qualunque CRS dichiari |
| KML | nessun analogo: KML 2.2 non ha un elemento che dichiari un CRS, e il formato resta WGS84 fisso |
| GeoJSON | una chiave ripetuta nell'oggetto geometria (`type`, `coordinates`, `geometries`) o al livello del `FeatureCollection` (`type`, `features`) rifiuta il documento, come già le chiavi ripetute della feature: prima valeva l'ultima, e due `features` si concatenavano (prova `chiavi_ripetute_nella_geometria_e_nella_collezione_sono_rifiutate`) |
| KML | un `Point` con più tuple, un `coordinates` ripetuto nella stessa geometria e un `Polygon` senza un unico anello esterno (due anelli nello stesso `outerBoundaryIs`, o due `outerBoundaryIs`) sono rifiutati: prima se ne teneva una parte e il resto spariva (prova `event_parser_rejects_geometries_it_used_to_truncate`) |
| DXF | più oggetti `GEODATA` con definizioni del CRS diverse sono rifiutati con `crs`: prima valeva il primo (prova `due_geodata_discordi_sono_rifiutati`). Definizioni uguali restano ammesse |
| DXF | una definizione WKT del `GEODATA` (un testo che comincia con una parola seguita da `[` o `(`) segue la regola unica di «tutti: identificatore di radice di un WKT», la stessa del `.prj`: dichiarazioni con l'una e l'altra parola chiave, `BOUNDCRS`, forma ESRI di WGS 84 (che ora dà `EPSG:4326` anche qui), malformata `crs` (prova `il_wkt_del_geodata_segue_la_regola_unica`); un codice EPSG di radice non canonico (`"04326"`), prima letto come tale, ora resta non risolto come nello Shapefile. Le regole proprie del `GEODATA` restano del driver: più `GEODATA` discordi sono `crs`, e una definizione non WKT che dichiara codici EPSG diversi — fra gli `EPSG:<codice>` del testo e gli `<Alias>` EPSG del sistema di coordinate di un XML Autodesk — è rifiutata con `crs`: prima valeva la prima occorrenza della prima forma trovata (prova `codici_epsg_discordi_nella_definizione_sono_rifiutati`). Degli alias conta solo quello del sistema di coordinate: gli attributi del tag d'apertura si leggono tutti (`nome = "valore"` o `nome='valore'`, spazi facoltativi attorno a `=`), e conta un alias con `type` uguale a `CoordinateSystem` (ASCII case-insensitive) e `id` di sole cifre. Un alias di datum o di ellissoide non conta, e nemmeno uno senza `type`, che non dice che cosa nomina; un tag che non si legge (virgolette non chiuse, valore senza virgolette, attributo ripetuto) è `crs`; `<Aliases>` è un altro elemento. Fino a b4d8064 il tipo si cercava come sottostringa `TYPE="`: `type = "Datum"` sembrava senza tipo, e l'alias del datum 6326 usciva come il CRS `EPSG:6326`; un alias senza `type` contava (prova `il_tipo_dell_alias_si_legge_come_attributo`). Un codice testuale o di alias non canonico (`EPSG:032632`) segue la regola (5) del WKT: senza opzione resta non risolto, con un'opzione uguale come testo vale la grafia del file, diversa è `crs`; fino a b4d8064 usciva risolto così com'era (prova `un_codice_testuale_non_canonico_segue_la_regola_del_wkt`). La forma testuale vale solo per una definizione che **è** l'identificatore (`EPSG:<cifre>` o `urn:ogc:def:crs:EPSG:<versione>:<cifre>`, a meno degli spazi): fino alla 5.0.1 ogni `EPSG:` seguito da cifre **ovunque** nel testo era una dichiarazione, e un proiettato senza `AUTHORITY` di radice che nel nome citava il proprio CRS base (`PROJCS["Locale basato su EPSG:4326",...]`) usciva `EPSG:4326` senza alcun segnale. Ora il nome è testo libero: quel `GEODATA` è non risolto e si nomina con `--assume-crs` (prova `un_epsg_nel_testo_della_definizione_non_e_l_identificatore`). Hazard: un testo come `EPSG:3003 EPSG:3003`, prima letto come EPSG:3003, ora richiede l'opzione. Un `--assume-crs` diverso dal codice che il `GEODATA` dichiara è rifiutato già nel resolver, oltre che nel controllo comune (prova `assume_crs_in_conflitto_col_geodata_e_rifiutato`). Una definizione che non comincia come un WKT vale solo in quattro forme — `EPSG:<cifre>`, `urn:ogc:def:crs:EPSG:<versione>:<cifre>`, `OGC:CRS84` (tutto il testo, a meno degli spazi) e un XML (testo che comincia con `<`) con un `<Alias>` EPSG del sistema di coordinate — che danno l'identificatore e **nessuna** definizione, perché non sono WKT né PROJJSON. Ogni altro testo è `crs` con `--assume-crs` e non risolto senza: un WKT preceduto da U+200B o fra virgolette, un `1` davanti a `GEOGCS[`, un XML senza alias, `OGC:CRS84 approssimato`. Fino a c0e84cb l'opzione nominava qualunque testo, il testo usciva come definizione `wkt` malformata, e un `GEODATA` così convertito in GeoParquet diventava `OGC:CRS84` su dati UTM (prova `un_geodata_non_riconosciuto_non_si_nomina`). Gli alias si cercano solo in un testo che comincia con `<`; l'XML non si valida (nessun lettore XML nel bordo), quindi un XML malformato con un alias EPSG del sistema di coordinate ne dà il codice: hazard dichiarato, rientra con un lettore XML |
| Shapefile | l'identificatore di un `.prj` lo decide la regola unica di «tutti: identificatore di radice di un WKT», la stessa del DXF; il `.prj` che è soltanto `OGC:CRS84` lo dichiara, e l'opzione lo può solo ripetere. `--assume-crs` diverso dall'identificatore che la radice del `.prj` dichiara, di qualunque autorità (`AUTHORITY["ESRI","4265"]` compreso), con la parola chiave dell'una o dell'altra grammatica (`ID[...]` in WKT 2 o in WKT 1, `AUTHORITY` in WKT 2), è rifiutato con `crs`; concordi, l'identificatore ha la grafia del file. Con l'opzione si rifiutano anche due identificatori di radice diversi e un `BOUNDCRS` (prova `assume_crs_non_nomina_radici_che_dichiarano_altro_o_niente_di_proprio`): prima vinceva l'opzione, e il contratto univa il suo identificatore alla definizione del `.prj` (GEO-005; prova `prj_e_assume_crs_in_conflitto_sono_rifiutati`). Senza opzione, due identificatori di radice diversi sono ora `crs` (prima valeva uno dei due, in silenzio) e un `ID` in WKT 1 è una dichiarazione (`EPSG:3003` si risolve, prima restava non risolto). L'opzione resta il modo di nominare un `.prj` che non dichiara un'autorità: la forma ESRI di WGS 84 senza `AUTHORITY` (quella che il writer scrive) dà `EPSG:4326` per **deduzione** del driver, che vale soltanto senza l'opzione e solo se la radice non dichiara un identificatore (`AUTHORITY["ESRI","104000"]` sopra il nome e il datum di WGS 84 non diventa `EPSG:4326`); con l'opzione, su quella forma sono ammessi solo i due nomi di WGS 84 geografico, `EPSG:4326` e `OGC:CRS84`, e ogni altro è `crs` (prova `assume_crs_non_rietichetta_un_prj_riconosciuto_o_dichiarato`). Fino alla 5.0.1 la deduzione si confrontava con l'opzione come una dichiarazione, e `--assume-crs OGC:CRS84` su quel `.prj` era rifiutato (prove `assume_crs_nomina_un_prj_esri_che_non_dichiara_autorita` in `driver-shp`, `un_prj_senza_autorita_si_nomina_con_assume_crs` in `plenora-io-tools`). Un `.prj` che non è un WKT ben formato è `crs` già all'apertura, con o senza opzione, per `io.inspect` come per `io.read` (prova `inspect_e_read_concordano_su_un_prj_malformato`): prima l'opzione lo nominava e la definizione si rifiutava solo a valle, alla verifica GEO-005, e senza opzione la forma ESRI si riconosceva anche su un testo malformato. Un `.prj` che esiste ma non si legge come testo UTF-8 è `crs` (prova `un_prj_non_utf8_e_un_errore_non_un_assenza`): prima valeva come assente, e l'opzione nominava il CRS che il file dichiarava. Rifiuti in più, mai risultati sbagliati: un `.prj` con un NUL finale si rifiuta come definizione non ben formata, con o senza opzione; un BOM UTF-8 iniziale invece si toglie (riga «tutti: BOM della definizione»). Un `.prj` che è soltanto `OGC:CRS84` dà l'identificatore `OGC:CRS84`, geografico, `lon_lat`, **senza** definizione: non è un WKT, e la chiave `crs_definition` porta solo WKT, WKT 2 o PROJJSON (ARROW-VOCABULARY-1.0). Fino a c0e84cb usciva come definizione `wkt` con ordine `unknown`: `io.read` lo emetteva e la conversione in IPC lo rifiutava (GEO-005) (prova `un_prj_ogc_crs84_non_emette_una_definizione`). Un WKT 2 con le parole chiave lunghe (`GEOGRAPHICCRS[`), minuscole o spaziate (`PROJCRS [`) è ora `wkt2` e si legge. Hazard residuo: quello della regola (6), un'opzione su un `.prj` senza dichiarazioni e diverso dalla forma ESRI di WGS 84 lo nomina anche quando ne contraddice la definizione. Gli altri driver non deducono un identificatore: GeoJSON e KML hanno il CRS fissato dal formato, GeoPackage, GeoParquet e IPC lo leggono dichiarato, il DXF da una definizione che lo dichiara o dalla stessa forma ESRI (righe sotto); il FileGDB riporta l'autorità che GDAL attribuisce allo `SpatialRef` del file, e un'attribuzione di GDAL diversa dall'opzione è un rifiuto in più, mai un CRS sbagliato. Gli altri formati seguono la stessa regola (vedi [`--assume-crs` e il CRS che la sorgente dichiara](#--assume-crs-e-il-crs-che-la-sorgente-dichiara)) |
| tutti | `--assume-crs` si confronta col CRS dichiarato per **uguaglianza** di identificatore (GEO-005), non per equivalenza: `OGC:CRS84` contro `EPSG:4326`, o un codice scritto con zeri iniziali, è un conflitto anche quando i due nomi descrivono lo stesso CRS. Hazard: un rifiuto in più, mai un CRS sbagliato; si rimedia passando l'identificatore come la sorgente lo scrive (`io.inspect` senza opzione lo mostra). Rientra con un resolver di equivalenze, che il bordo I/O non porta |
| CSV, XLSX | non portano CRS: con una geometria, `--assume-crs` è obbligatorio |
| tutti | nessuna riproiezione |
| tutti | nessuna proiezione delle colonne: lo schema consegnato è quello della sorgente |

### Le due serializzazioni di Arrow IPC

Arrow IPC ne definisce due, e questo prodotto le produce e le legge entrambe:

| | Come si chiede | Come si riconosce |
|---|---|---|
| `application/vnd.apache.arrow.file` | il default | `ARROW1` in testa e in coda, più il footer che indicizza i blocchi |
| `application/vnd.apache.arrow.stream` | `--out-opt serialization=stream` | una sequenza di messaggi, senza magic né footer |

La scelta è un'**opzione di formato** del driver IPC, e `io.catalog` la
pubblica accanto alle altre: non è un campo nuovo di uno schema d'ingresso.
`delivered.content_type` nel risultato dice quale delle due è uscita.

In lettura non serve chiederlo: il driver guarda i **byte**. Un flusso
chiamato `.arrow` si legge, e un contenitore chiamato `.arrows` pure — il nome
del file è del chiamante, e serve solo a scegliere il driver quando il formato
non è dichiarato. `io.write` accetta il payload in entrambe le forme e riferisce
quale ha letto.

#### Che cosa uno stream **non** cambia

La consegna resta **atomica sull'operazione**. Se una violazione emerge in un
punto qualsiasi della sorgente, l'operazione viene rifiutata come blocco unico e
nessun prefisso accettato viene consegnato — o tutto, o niente. Il descrittore
lo dice con `attributes.delivery: operation_atomic`, e vale per tutti e dieci i
driver.

È una distinzione che è facile confondere, e per un periodo l'abbiamo confusa:
lo stream è una **serializzazione**, cioè come i byte sono disposti; la consegna
incrementale è un'altra cosa, cioè il consumatore che elabora i primi batch
mentre gli ultimi non esistono ancora. Produrre un flusso non richiede la
seconda, e qui non la si fa: i byte si scrivono per intero in un file di
staging, e la pubblicazione è l'ultima operazione. Un'operazione fallita non
lascia una destinazione, in nessuna delle due forme.

Il descrittore dichiara anche `attributes.materialization: bounded`, che è la
forma in cui ARROW-011 è soddisfatto: il requisito si applica a chi annuncia lo
stream, e ammette per iscritto che il descrittore dichiari la materializzazione
limitata.

### Una sorgente senza geometrie, e una di cui non si sanno i tipi

Non sono lo stesso caso, e per un po' il prodotto le ha trattate uguali.

Quando la destinazione accetta solo alcuni dei sedici tipi geometrici canonici —
il sink Arrow IPC ne dichiara sette, perché trasporta WKB senza interpretarlo —
pretende di saperli in anticipo. Se il contratto della sorgente arriva senza
tipi, la scrittura viene rifiutata: è prudente, perché altrimenti la
scoprirebbe a scrittura iniziata.

«Senza tipi» però copriva due situazioni diverse. Una sorgente percorsa fino in
fondo e priva di geometrie — un GeoJSON con zero feature — non ha niente da
dichiarare, e pretendere da lei una dichiarazione preventiva rifiutava una
conversione che il formato può fare benissimo. Una sorgente di cui i tipi non si
sono potuti determinare è un'altra cosa, e lì il rifiuto serve.

Il contratto interno ora le distingue, e il rifiuto resta solo sulla seconda.

Sul **filo** la distinzione non passa, ed è bene saperlo:
`ARROW-VOCABULARY-1.0` è un vocabolario chiuso, e
`plenora.geometry.types_declaration` ammette i soli `exact`, `mixed`,
`unresolved`. Un'assenza accertata si scrive quindi `unresolved`, come
un'ignoranza. La conseguenza si vede: un GeoJSON vuoto si consegna in Arrow, e
quel file consegnato — riletto — viene rifiutato verso lo stesso sink, perché
chi lo rilegge non ha modo di sapere che l'assenza era stata accertata.
Scioglierlo richiede un vocabolario successore, che non è una decisione di
questo componente.

---

## LossReport — ratificato con il protocollo 2

Il contratto del report di perdita è **ratificato**, ed è il
[protocollo 2](../release/cli-protocol-v2.json). Le cinque decisioni che lo
tenevano aperto — struttura, limiti, redazione, comportamento al limite,
versionamento — sono chiuse, e ciascuna è applicata dal codice e verificata da
un gate.

Che cosa esce oggi, nel v2:

```json
"read_loss": {
  "lossless": false,
  "troncato": false,
  "omesse_esatte": true,
  "omesse": { "categorie_omesse": 0, "ragioni_omesse": 0,
              "esempi_omessi": 0, "omesse_per_byte": 0 },
  "counts": [ { "categoria": "coercion tipo attributo", "conteggio": 3 } ],
  "esempi": [ { "category": "coercion tipo attributo",
                "layer_index": 0, "field_index": 4, "type_class": "decimal",
                "context": "il tipo dell'attributo richiede una coercizione" } ]
}
```

### I limiti, e chi li decide

| Grandezza | Tetto |
|---|---|
| categorie in `counts` | **64** |
| byte di un identificatore di categoria | **128**, ovunque compaia |
| ragioni in `reasons` | **64** |
| esempi in `esempi` | **64** |
| byte di un `detail` o di un `context` | **512** |
| byte di una sezione | **12 KiB**, sulla serializzazione effettiva |
| byte della diagnostica di una busta | **64 KiB** |

Nessuno di questi numeri lo decide chi fornisce il file, ed è il cambiamento
che il v2 porta. Il budget non speso da una sezione **non** passa a un'altra:
con un consumo sequenziale una sezione grande affamerebbe quelle che la
seguono, e la stessa sezione produrrebbe un output diverso a seconda di quanto
ha occupato un'altra.

### Nessun nome preso dal file

`reasons[].detail` e `esempi[].context` portano testo **curato**: stabile,
descrittivo, scritto da noi. Dove si è persa una cosa lo dicono `layer_index` e
`field_index`, che sono indici e non nomi — e nemmeno un hash dei nomi, che
resterebbe un identificatore controllato da chi fornisce il file. Il tipo di un
attributo passa da `type_class`, un vocabolario chiuso nostro, mai dalla forma
`Debug` di un tipo di dipendenza.

L'unica eccezione è dichiarata: i **codici numerici** di un registro di
autorità CRS possono comparire in un `context`, perché senza di loro
un'incoerenza fra `crs_definition`, `crs_id` e `srid` non è leggibile. Le
stringhe libere che li accompagnano restano vietate.

### Il troncamento è sempre dichiarato

Quattro cause separate, perché «sono più di sessantaquattro» e «lo spazio è
finito» portano a decisioni diverse. I conteggi pubblicati restano esatti: si
omette una voce intera, mai si riscrive un numero. Se nemmeno la dichiarazione
di troncamento entra nel budget, la CLI **fallisce** invece di pubblicare una
sezione che tace.

`omesse_esatte` qualifica i quattro contatori: vale `false` quando una perdita
di esattezza interna li rende limiti inferiori.

### Il v1 resta quello che era

`--legacy-protocol-v1-unsafe` seleziona il protocollo congelato, difetti
compresi, e lo dice nel nome. I suoi `detail` sono conservati **alla lettera** —
non ricostruiti — in un campo privato che un solo adattatore legge, e a
pretendere che il lettore resti uno solo è un gate: la visibilità di Rust non sa
dire «questo modulo e nessun altro».

### Che cosa esce su stdout e che cosa su stderr

Tre righe, e valgono insieme:

* in caso di **errore**, `stderr` contiene sempre e soltanto la busta JSON. Un
  consumatore la legge con un parser sull'intero flusso, e non trova mai nulla
  che la preceda;
* in caso di **successo con il protocollo legacy**, `stdout` conserva il
  documento v1 byte per byte e `stderr` porta l'avviso che quel protocollo è
  stato scelto;
* con il protocollo predefinito e successo, `stderr` resta **vuoto**.

L'avviso accompagna una consegna riuscita, e solo quella. Sul percorso d'errore
non c'è niente da avvertire: una busta v1 non è stata consegnata, quindi la
diagnostica illimitata di cui l'avviso parla non esiste. Su `stdout` non va
perché il v1 è congelato byte per byte, e aggiungere un avviso al documento
sarebbe cambiarlo.

Il gestore dei segnali segue la stessa regola: se non si installa, il comando
**non parte** e `stderr` porta un rifiuto tipizzato — non un avviso testuale che
finirebbe davanti alla busta.

### Adozione locale contracts 3aec9f7

Profilo io-tools v3, catalogo v3, Public Catalogs e Runtime Binding 2.0.
Wire runtime 1; catalog/read/write passano a versione operativa 2.
Gli schemi v1 restano immutati. Python SDK #32 non adottato.

- **IO-NULL-001** (cli): Regola 4; ambito risultati CLI/Rust/runtime: non e completata la qualifica del significato di ogni campo nullable. Gli schemi legacy e le prove di forma non dimostrano che null significhi sempre non applicabile e mai ignoto. Hazard: il consumatore del profilo v3 potrebbe interpretare come non applicabile un valore non qualificato. Non si afferma una violazione osservata per ogni null; la garanzia uniforme IO-NULL-001 non e rivendicata. Rientro: censire ogni campo nullable e rappresentare lo stato ignoto con un membro tipizzato o errore; registro docs/PRODUCT.md.
- **RT-024** (runtime): Regola 4; il flusso Arrow usa max_output_bytes della pipeline, senza successi troncati, ma manca un tetto uniforme per la materializzazione JSON e una prenotazione fallibile del Vec Arrow. Hazard: allocazione abortita invece del resource_limit tipizzato. Nessuna conformita completa a RT-024 e rivendicata. Rientro: budget uniforme e allocazioni fallibili prima della materializzazione, prove al limite con assi coerenti dopo pubblicazione. Limiti di trasporto a carico host; docs/PRODUCT.md.
- **RUNTIME-VECTORS-1.0 sezione 5 (versioni operative storiche)** (runtime): Regola 4; i vettori io-read-v1 fissano io.read@1 (il registro runtime-v1 include anche @2); il profilo v3 serve io.read@2. Gli originali restano immutati: @1 e rifiutato unsupported; i test applicano esplicitamente @2 e il contratto risultato v2 ai casi di esecuzione. Hazard: copiare il selettore storico impedisce la lettura. Rientro: vettori successori del catalogo v3 in contracts; prove runtime_vettori e docs/PRODUCT.md.

Il passaggio dal profilo 1 cambia i contratti di risultato: richiede una release
maggiore prima della distribuzione. Questa modifica del worktree non costituisce
una release e non modifica la versione del pacchetto. La qualifica semantica di
IO-NULL-001 resta aperta; i test degli schemi verificano la forma, non quel significato.
