# Change impact analysis — S9 tranche 11: `driver-filegdb` redatto

Data: 2026-08-21. Sigla: **S9 / tranche 11**.
Baseline: `f4de214` (tranche 10, `driver-xls`).
`plenora-io-error-v1` **invariato**.
Qualifica: **livello 1** — verificato, non è un checkpoint.

## Censimento a due classi

| Via | Forma | Occorrenze |
|---|---|---:|
| `err` | `impl Into<String>` | 35 chiamanti |
| `geometry_capability` | `field: &str` + `impl Into<String>` | 9 chiamanti |
| `err(format!("…: {e}"))` | testo GDAL | **13** |
| `format!` con nomi di layer o di campo | fuga di testo | **11** |
| `PlenoraIoError::capability(…, Some(nome), …)` | nome nel campo `field` | 9 |
| `Result<_, String>` / `DeError::custom` | — | 0 |

Usi legacy diretti: **22** — il crate più grande finora.

## La decisione che vale più della migrazione

Nei siti `capability` il nome del campo **non passa più**, e ho scelto di
passare `None` invece di aggiungere `ContractIdentifier::from_arrow_field`.

La ragione è nel driver: qui un `&Field` arriva **tanto dal piano quanto dallo
schema che GDAL ha letto dal file**. `ogr_to_arrow` riceve nomi letti dal
FileGDB; `contract_ogr_type` riceve nomi del `WriteLayer`. Distinguerli sito per
sito costerebbe più di quanto vale, e sbagliarne uno significherebbe far entrare
un nome del payload dentro un tipo che promette il contrario — a quel punto il
tipo non promette più niente, e la promessa vale per **tutti** i suoi usi, non
solo per questo driver.

**È una perdita diagnostica reale**, e va detta senza attenuarla: il campo
geometrico di un FileGDB è uno solo e chi legge l'errore ha il contratto, ma per
i campi attributo l'identità ora si legge dal piano invece che dal messaggio.

`geometry_capability` perde di conseguenza il parametro `field`.

## Le altre fughe

* **13 siti con il testo di GDAL** — apertura, creazione, layer, feature,
  geometria, campi;
* **nomi di layer** interpolati in cinque messaggi (`create_layer '{}'`,
  `layer '{}' senza contratto geometrico`, …): vengono dal piano;
* **nomi di campo** interpolati in sei messaggi, alcuni dal piano e alcuni letti
  dal file;
* **`GDAL ha normalizzato il nome in '{actual_name}'`**: né il nome atteso né
  quello normalizzato escono — il primo viene dal piano, il secondo è un
  derivato che GDAL ha prodotto leggendolo;
* **`tipo campo OGR {ft}`**: il codice del tipo viene dallo schema letto dal
  file.

Restano gli **indici** — di layer, di campo Arrow, di campo OGR — perché li
produce la nostra enumerazione, e il tetto sull'output come conteggio e limite.

`tipo Arrow {other:?}` diventa `driver_common::classe_arrow`, terzo driver che
usa quel vocabolario.

## Un difetto del registro dei fallback, trovato scrivendoci dentro

`check_assurance_fallbacks.sh` conta **il testo**, non il codice.

Un commento che spiegava perché *non* stavo usando `unwrap_or(...)` faceva
salire il contatore da 5 a 6, perché conteneva quella stringa. Il commento è
stato riformulato senza nominare la forma alternativa.

**È la stessa classe di fragilità che INFRA-1 aveva chiuso** per il censimento
WKB, sostituendo `path:riga` con `percorso::funzione`: un gate che guarda il
testo si accende su ciò che il testo dice, non su ciò che il codice fa. Qui il
costo è basso — un commento riformulato — ma il registro **può essere mosso da
un commento**, e vale la pena saperlo prima di scoprirlo in un caso in cui conta.

Sul merito la conversione è ora **totale** (`unsigned_abs`) invece di avere un
ramo di riserva: non c'è nessun fallback da registrare, che è meglio che averne
uno giustificato.

## Rifattorizzazione imposta da clippy

`verifica_schema_invariato` estratta da `spawn_reader`, che aveva superato le
cento righe. Il blocco confronta gli indici OGR pre-risolti con lo schema che il
worker trova riaprendo il dataset e fallisce chiuso se qualcuno l'ha cambiato
nel frattempo: è una verifica con un nome. Riceve i campi effettivi già
calcolati, perché il chiamante li riusa per la lista degli ignorati.

`ogr_to_arrow` perde il parametro `name`: non serviva al calcolo, serviva solo a
comporre il messaggio che ora non lo porta. Toglierlo dalla firma è preferibile
a marcarlo con l'underscore — un parametro inutilizzato che resta invita a
rimetterlo dentro.

## Verifica (livello 1)

* `scripts/check_errori_redatti.py`: **70 → 48** in tre crate; **undici**
  componenti migrati e a zero;
* sonde del censimento: 7/7;
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: pulito;
* `cargo test --workspace --all-features`: verde su 31 binari;
* **percorso feature-on**: `cargo test -p driver-filegdb --features gdal-backend`
  verde (22 test + 1 doc), e il **catalogo reale** prodotto dalla CLI con
  `gdal-backend` supera `check_filegdb_catalog.py`;
* registro dei fallback **invariato a 115**;
* replay deterministico di **1 844 input** su `ipc_to_gpkg` senza crash, poi
  smoke senza finding.

## Prossimo passo

Tranche 12: `driver-shp` (22) — nomi DBF e pubblicazione articolata. Alla sua
chiusura il **checkpoint di livello 2**, con
`S9_CHECKPOINT_BASE=effc4abe3f74ade083dbed72c94c286748809d9f`.

Poi `driver-dxf` (20) e, **per ultima**, `plenora-io-cli` (6).

---

## Addendum del 2026-08-21 — correzione sull'evidenza fuzz

**Il corpo di questo documento resta com'era**: la regola append-only vale anche
qui, e riscrivere un'affermazione dopo averla fatta toglie al lettore il modo di
distinguere cio' che si sapeva allora da cio' che si e' capito dopo.

L'affermazione da correggere e' nella sezione *Verifica*:

> replay deterministico di **1 844 input** su `ipc_to_gpkg` senza crash, poi
> smoke senza finding.

La misura e' esatta, ma **`ipc_to_gpkg` non esercita `driver-filegdb`**. Il suo
percorso e' Arrow IPC in ingresso, GeoPackage in uscita; l'ho scelto perche' e'
il target che esercita la validazione di capability e la scrittura, e ho
scambiato una somiglianza di forma per una copertura del driver migrato.
Presentarlo come «replay mirato della tranche» era sbagliato.

**Non esiste un fuzz target per FileGDB.** I tredici target dichiarati sono
`csv_reader`, `dxf_reader`, `from_wkb`, `geojson_reader`, `geoparquet_reader`,
`gpkg_geometry`, `gpkg_reader`, `ipc_reader`, `ipc_to_gpkg`, `kml_reader`,
`shp_wkb`, `wkt_parse`, `xlsx_reader`. Nessuno tocca `driver-filegdb`, e la
ragione e' strutturale: il driver e' feature-gated su `gdal-backend`, e la
campagna fuzz gira senza GDAL.

### L'evidenza vera della tranche 11

| | |
|---|---|
| `cargo test -p driver-filegdb --features gdal-backend` | verde, 22 test + 1 doc |
| catalogo reale della CLI con `gdal-backend`, verificato da `check_filegdb_catalog.py` | verde |
| `cargo test --workspace --all-features` | verde su 31 binari |
| fuzzing mirato al driver | **assente, e non sostituito da nulla** |

Il replay su `ipc_to_gpkg` resta una verifica di non-regressione utile — e' uno
dei tredici target e la migrazione ha toccato `driver-common`, che quel target
attraversa — ma **non e' evidenza su `driver-filegdb`**.

### Che cosa resta aperto

Un fuzz target per FileGDB richiederebbe GDAL nella campagna. Non e' una
decisione di questa tranche e non la prendo qui: e' registrata come lacuna
dichiarata, perche' una lacuna nominata e' diversa da una lacuna coperta da un
numero che parla d'altro.

