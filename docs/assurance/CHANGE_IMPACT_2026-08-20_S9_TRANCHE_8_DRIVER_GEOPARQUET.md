# Change impact analysis — S9 tranche 8: `driver-geoparquet` redatto

Data: 2026-08-20. Sigla: **S9 / tranche 8**.
Baseline: `7019312` (tranche 7, `driver-gpkg`).
`plenora-io-error-v1` **invariato**.
Qualifica: **livello 1** — verificato, non è un checkpoint.

## Censimento a due classi

| Via | Forma | Occorrenze |
|---|---|---:|
| `fmt_err` | `impl Into<String>` | 49 chiamanti |
| `pagine::errore` | `&'static str` | 12 chiamanti, già statico |
| `fmt_err(format!("…: {e}"))` | cancellazione di struttura | **11** |
| `format!` con nome dal contratto o dal piano | fuga di testo | 2 |
| `Result<_, String>` | — | 0 |
| `DeError::custom` | — | 0 |

Usi legacy diretti: **8**.

Gli undici siti di cancellazione riportavano il `Display` di `parquet` e
`arrow`, uno per fase — apertura, lettura, batch, re-tag dello schema, bbox,
scrittura, chiusura. Ognuno ha ora un messaggio curato che dice **quale fase**
ha fallito: la fase era già implicita nel testo della dipendenza, e conservarla
non costa nulla.

## Questo crate sapeva già qualcosa

`crs_from` aveva **già** un test che verifica la non-fuga:

```rust
assert!(!error.to_string().contains("survey-grid-secret"));
```

Il nome di un CRS letto dai metadati GeoParquet non doveva uscire, e non usciva.
È l'unico driver che aveva scritto quella proprietà come test invece di
affidarla a una convenzione — e infatti quel test passa senza modifiche.

## Il nome del campo esce, ma da un altro posto

`campo esposto '{}' non presente nello schema Parquet fisico` interpolava il
nome del campo. Il nome viene dal **contratto**, non dal payload, quindi non è
la stessa classe di fuga degli `srs_id` di `driver-gpkg`.

Invece di buttarlo, viene messo nel campo `field` di `PlenoraIoError` come
[`ContractIdentifier`]. Quando il nome non è nominabile — vuoto, o oltre il
tetto — l'errore resta **senza campo** invece di portarne uno inventato, e
l'indice nel messaggio identifica comunque il punto.

### Contesto Rust tipizzato ≠ busta serializzata

Questa distinzione va tenuta ferma, perché è facile enunciare più di quanto sia
vero.

| | oggi |
|---|---|
| `PlenoraIoError.field` (tipo Rust, in-process) | **popolato**, e leggibile da chi usa la libreria direttamente |
| `plenora-io-error-v1` (busta serializzata dalla CLI) | **non emette `field`**, e nemmeno `driver` |

`err_doc` costruisce esattamente sei chiavi — `category`, `phase`,
`remote_effect`, `retry`, `code`, `message` — più `row_diagnostics` quando c'è.
S9 mantiene il wire **invariato**, quindi **nessun consumatore della busta CLI
può leggere `field` oggi**, e dichiararlo sarebbe falso.

Il campo diventerà disponibile sul wire soltanto attraverso il futuro
adattatore verso `plenora-contracts-next`, che è uno step breaking separato,
insieme a CLI v2, exit code e capabilities. La matrice di handoff lo registra
già come tale.

Il guadagno di oggi è quindi **in-process**: un consumatore Rust della libreria
legge un `ContractIdentifier` invece di estrarre un nome da una frase. Sul wire,
per ora, l'informazione è **sostituita** dall'indice nel messaggio — non
aggiunta.

Un test lo blocca: `il_wire_v1_ha_esattamente_i_campi_dichiarati_e_non_acquista_field`
costruisce un errore con `driver` e `field` popolati, verifica che ci siano nel
tipo Rust, e poi confronta l'**insieme** delle chiavi della busta con quello
dichiarato. Guarda l'insieme e non le singole chiavi di proposito: un `assert`
per campo assente si dimentica del campo che nessuno ha ancora inventato.

## Perdita dichiarata

`colonna utente '{collision}' entrerebbe in collisione con le colonne bbox
interne ({})` perde il nome utente: viene dal piano, e chi legge l'errore ha il
piano. Perde anche l'elenco delle colonne bbox, che è **nostro e costante**:
sta nella documentazione del driver, non in ogni messaggio.

`compression '{altro}' non riconosciuta` perde il valore, ma **non è una
perdita**: lo schema dichiara `compression` come `Enumerato`, quindi un valore
diverso è già stato respinto da `valida_opzioni` con il suo token bounded. Il
ramo nel driver è difensivo e irraggiungibile quando la validazione ha girato —
esattamente come `geometry_encoding` in `driver-csv`.

## Rifattorizzazioni imposte da clippy

Due funzioni hanno superato le cento righe perché i messaggi curati occupano
più righe dei `format!` che sostituiscono.

* `mappa_campi_fisici` estratta da `open`. Non è un espediente per far tacere il
  lint: quel blocco decide una cosa sola e la decide per intero, ed è cresciuto
  proprio perché ora costruisce anche il contesto strutturato dell'errore;
* `campo_fuori_range` estratta da `open_layer_reader`, che era a 101 righe.

Anche `if let Some(_) = … .find(…)` è diventato `.any(…)`: clippy l'ha
segnalato solo dopo che il binding `collision` è rimasto inutilizzato, il che
rende il predicato quello che era già.

## Verifica (livello 1)

* `scripts/check_errori_redatti.py`: **103 → 95** in sei crate; otto componenti
  migrati e a **zero**;
* sonde del censimento: 7/7;
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: pulito;
* `cargo test --workspace --all-features`: verde su 31 binari, compresi il test
  sulla non-fuga del nome CRS e il nuovo
  `il_wire_v1_ha_esattamente_i_campi_dichiarati_e_non_acquista_field`;
* gate specifici verdi; registro dei fallback **invariato a 115**;
* replay deterministico di **2 932 input** su `geoparquet_reader` e `from_wkb`
  senza crash, poi smoke sugli stessi due senza finding — il replay **prima**,
  come impone `s9-checkpoint.sh`.

## Prossimo passo

Tranche 9: `driver-ipc` (7). Alla sua chiusura è dovuto il **checkpoint di
livello 2** con `scripts/s9-checkpoint.sh`. La CLI resta ultima, dopo tutti i
driver.
