# Change impact analysis — S9 tranche 12: `driver-shp` redatto

Data: 2026-08-21. Sigla: **S9 / tranche 12**.
Baseline: `b4e535b` (addendum alla tranche 11).
`plenora-io-error-v1` **invariato**.
Qualifica: **livello 1** — verificato, non è un checkpoint.

## Censimento a due classi

| Via | Forma | Occorrenze |
|---|---|---:|
| `err` | `impl Into<String>` | 86 chiamanti |
| `err(format!("…: {e}"))` | testo di `shapefile`, `dbase`, `io`, `arrow` | **13** |
| `format!` con nomi DBF **letti dal file** | fuga di payload | **5** |
| `format!` con conteggi dichiarati dal file | fuga di payload | **3** |
| `format!` con nomi o valori dal piano | fuga di testo | 3 |
| `Result<_, String>` / `DeError::custom` | — | 0 |

Usi legacy diretti: **22**.

## I nomi DBF: provenienza inequivocabile, nel verso sbagliato

La cautela chiesta per Shapefile era «`ContractIdentifier` soltanto quando la
provenienza attestata è inequivocabile; altrimenti `None`». Qui la provenienza
**è** inequivocabile — e va nella direzione opposta: i nomi arrivano dai
**descrittori del file DBF**, letti byte per byte da `leggi_descrittori_dbf`.

Non era quindi una scelta prudenziale ma l'unica corretta. Cinque messaggi li
interpolavano:

* `nomi campo DBF duplicati: '{name}'`
* `campo DBF '{name}' con larghezza zero`
* `nome campo DBF vuoto all'indice {index}` — il nome no, l'indice sì
* `schema DBF senza accumulatore per '{name}'`, due siti
* `campo DBF '{}' fuori record`

Escono ora gli **indici**, prodotti dalla nostra enumerazione dei descrittori.

## Conteggi dichiarati dal file e conteggi nostri

Due messaggi mettevano insieme le due cose, ed è la distinzione che il vincolo
di S9 chiede di fare:

| Messaggio | Dichiarato dal file | Calcolato da noi |
|---|---|---|
| `numero descrittori DBF incoerente: header={field_count}, decoder={}` | `field_count` — **esce di scena** | `decoded_names.len()` — **resta** |
| `record DBF dichiarato lungo {declared} byte ma i campi ne richiedono {offset}` | `declared_record_length` — **esce di scena** | `offset` — **resta** |

Stesso criterio per `numero di geometrie ({shape_count}) diverso dai record DBF
({dbf_record_count})`: **entrambi** vengono dal file, quindi resta solo la
condizione. E per `marcatore record DBF non valido: 0x{marker:02x}`, che è un
byte del payload.

## Il percorso di scrittura

* `nome campo '{}' non valido per dbf` — il nome viene dal **piano**; esce
  l'indice;
* `publish_mode Shapefile '{other}' non valido` — lo schema dichiara
  `publish_mode` come `Enumerato(&[DIRECTORY_DATASET_MODE, LOOSE_SET_MODE])`,
  quindi il ramo è difensivo: **non è una perdita**;
* `publish_mode '{}' richiede una destinazione {}` — entrambi `&'static str` del
  nostro enum. Resta `destination_suffix()`, che è l'informazione azionabile:
  chi ha scritto l'opzione sa quale modo ha chiesto, non sa come deve essere
  fatta la destinazione.

`tipo Shape nel record '{tag}' incoerente con l'header '{}'`: il tag del record
viene dal file e sparisce; l'etichetta dell'header è un nostro `&'static str` e
resta.

## Igiene, non solo migrazione

`ShapefilePublishMode::name()` esisteva **solo** per comporre un messaggio che
ora non la porta. L'ho rimossa invece di marcarla `#[allow(dead_code)]` — stessa
ragione del parametro `name` di `ogr_to_arrow` nella tranche 11: un membro
inutilizzato che resta invita a rimetterlo dentro.

`leggi_descrittori_dbf` estratta da `read_dbf_layout`, che aveva superato le
cento righe. È un ciclo che decodifica il vettore dei campi e ne verifica nomi,
duplicati e larghezze: una cosa sola, con un nome.

## Verifica (livello 1)

* `scripts/check_errori_redatti.py`: **48 → 26** in **due** crate; **dodici**
  componenti migrati e a zero;
* sonde del censimento: 7/7;
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: pulito;
* `cargo test --workspace --all-features`: verde su 31 binari;
* registro dei fallback **invariato a 115**;
* replay deterministico di **1 371 input** su `shp_wkb` senza crash, poi smoke
  senza finding.

## Prossimo passo

**INFRA-4**: rendere il censimento dei fallback lessicale invece che testuale.
Commenti e stringhe non devono contare come chiamate — vedi la tranche 11, dove
un commento faceva salire il contatore.

Poi il **checkpoint di livello 2** sul SHA risultante, con
`S9_CHECKPOINT_BASE=effc4abe3f74ade083dbed72c94c286748809d9f`.

Restano `driver-dxf` (20) e `plenora-io-cli` (6), quest'ultima **per ultima**.
