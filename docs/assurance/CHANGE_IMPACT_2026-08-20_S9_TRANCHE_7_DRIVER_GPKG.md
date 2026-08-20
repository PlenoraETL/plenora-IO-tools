# Change impact analysis — S9 tranche 7: `driver-gpkg` redatto

Data: 2026-08-20. Sigla: **S9 / tranche 7**.
Baseline: `e9cc7b5` (governance append-only), dopo il checkpoint superato su
`107b7b5`.
`plenora-io-error-v1` **invariato**.
Qualifica: **livello 1** — verificato, non è un checkpoint.

## Censimento a due classi

| Via | Forma | Occorrenze |
|---|---|---:|
| `err` | `impl Into<String>` | 34 chiamanti |
| **`sql_err`** | `rusqlite::Error` → `err(format!("sqlite: {e}"))` | **27 chiamanti** |
| `err(format!(…))` con testo di dipendenza | cancellazione di struttura | 3 |
| `err(format!(…))` con valori letti dal file | fuga di payload | **7** |
| `Result<_, String>` | — | 0 |
| `DeError::custom` | — | 0 |

Usi legacy diretti: **10**.

### `sql_err`: una via di laundering dedicata

Ventisette percorsi passavano da un helper la cui unica ragione d'essere era
riportare il `Display` di `rusqlite::Error`. Per la variante `SqliteFailure`
quel testo **contiene il messaggio di SQLite**, che può portare frammenti di
query e nomi di tabella letti dal file.

Sostituirlo con un messaggio unico avrebbe appiattito ventisette cause in una.
La soluzione segue il pattern già ratificato dall'errata S9.1: `classe_sqlite`,
un `const fn` che mappa le varianti di `rusqlite::Error` su un vocabolario
**nostro, statico e chiuso**. Le cause restano distinte, il testo della
dipendenza non esce, e il messaggio non cambia se `rusqlite` riscrive i propri.

### Le sette fughe di payload

Quattro erano invisibili al grep a riga singola perché il `format!` era
multiriga — un limite del mio censimento, non della regola:

| Sito | Cosa usciva |
|---|---|
| `nome layer duplicato: {}` | nome del layer (dal piano) |
| `layer '{}' senza colonna geometria` ×2 | nome del layer |
| `gpkg_geometry_columns.srs_id={}` | **`srs_id` letto dal file** |
| `SRS per-feature {} discordante dal layer {}` | **due `srs_id` letti dal file** |
| `rowid massimo {observed_max} … cursore {previous}` | **due rowid letti dal file** |
| `la tabella "{table}" dichiara la colonna "{name}"` | **nome di tabella e di colonna letti dal file** |
| `byte order WKB non valido: 0x{other:02x}` | **byte letto dal payload** |
| `flavor WKB ISO non riconosciuto: {other}` | **codice letto dal payload** |

I nomi di layer diventano l'**indice nel piano**; gli altri valori spariscono e
resta la condizione, che è ciò che il chiamante non può dedurre da solo.

## Un costruttore redatto in più

`PlenoraIoError::crs_non_risolto_redatto(driver, &RawCrs)`, che rispecchia
`crs_unresolved`. Del `RawCrs` escono **due conteggi di byte**, non il
contenuto: la definizione e l'hint di authority vengono dal file, e dire quanto
sono lunghi è l'informazione che il chiamante non ha senza dire quella che ha
già.

Il gate l'ha trovato dopo la prima passata — le mie famiglie di sostituzione non
coprivano `crs_unresolved` — ed è la dimostrazione che il gate per-crate serve:
tre chiamate residue in un crate dichiarato migrato l'hanno acceso subito.

Servirà anche a `driver-shp` e agli altri che usano `crs_unresolved`.

## Rifattorizzazioni imposte da clippy

* `classe_arrow` di `driver-common` diventa `pub`: è il secondo driver che ne ha
  bisogno, come previsto quando è stato introdotto;
* `tipo_e_dimensioni` estratta da `wkb_shape`, che aveva superato le cento
  righe. Quel blocco decideva da solo se il payload è EWKB, quale sia il tipo
  base e quante dimensioni porta, leggendo bit di flag che non c'entrano con il
  resto della funzione: aveva già un nome implicito;
* `read_u32`/`read_f64` rinominati `leggi_intero`/`leggi_reale`. Clippy li ha
  segnalati come troppo simili **solo dopo** l'estrazione, perché il lint
  guarda le altre variabili in scope e ne erano sparite cinque.

## Superficie adiacente, non toccata

`LossExample { context: format!("field={name}: {}", coercion.detail()) }` porta
un nome di campo in un **report di perdita**, non in un errore. È una struttura
diversa, con un altro contratto di wire, e S9 non la copre. Va guardata quando
si deciderà se la stessa regola vale per il report di perdita — **non l'ho
cambiata**, perché estendere il perimetro di S9 senza ratifica sarebbe la stessa
cosa che allargare un'eccezione senza ratifica.

Analogamente, `RawCrs::new(format!("GeoPackage srs_id={srs_id}; definition={def}"))`
mette la definizione letta dal file dentro un **dato di contratto**, non dentro
un messaggio. Stessa considerazione.

## Verifica (livello 1)

* `scripts/check_errori_redatti.py`: **113 → 103** in sette crate; sette
  componenti migrati e a **zero**;
* sonde del censimento: 7/7;
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: pulito;
* `cargo test --workspace --all-features`: verde su 31 binari;
* gate specifici verdi; registro dei fallback **invariato a 115**;
* replay deterministico e smoke su `gpkg_reader`, `gpkg_geometry`,
  `ipc_to_gpkg` — il replay **prima**, come impone `s9-checkpoint.sh`.

## Prossimo passo

Tranche 8: `driver-geoparquet` (8). Poi `driver-ipc` (7), e alla chiusura della
tranche 9 il checkpoint di livello 2. La CLI resta ultima, dopo tutti i driver.
