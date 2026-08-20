# Change impact analysis — S9 tranche 9: `driver-ipc` redatto

Data: 2026-08-20. Sigla: **S9 / tranche 9**.
Baseline: `2003d4a` (tranche 8, `driver-geoparquet`).
`plenora-io-error-v1` **invariato**.
Qualifica: **livello 1** — verificato. Il checkpoint di livello 2 è dovuto
**dopo** questo commit, su un albero pulito.

## Censimento mirato

I quattro controlli chiesti per questo driver, e cosa hanno trovato.

| Controllo | Esito |
|---|---|
| `ArrowError` trasformati in testo | **8 siti**, tutti chiusi |
| `Debug` di schema, `DataType`, metadata, nomi di campo dal file | **nessuno** nei messaggi d'errore |
| helper o `Result<_, String>` che cancellano `PlenoraIoError` | un solo helper, `err(impl Into<String>)` con 11 chiamanti; **nessun** `Result<_, String>`, **nessun** `DeError::custom` |
| conservazione di `category`, `phase`, `code`, `retry` | preservata sito per sito; un test esistente già la verifica |

Usi legacy diretti: **7**.

### Gli otto `ArrowError`

Uno per fase — apertura, lettura, batch, retag del contratto, scrittura,
`finish`, `into_inner` — e ognuno ha ora un messaggio curato che nomina la fase.
È lo stesso schema di `driver-geoparquet`: la fase era già l'unica informazione
utile di quel testo, e il resto era il modo in cui `arrow` descrive un errore.

### `Debug` dei tipi Arrow: assente

Il controllo era mirato perché è la fuga più facile da non vedere — il `Debug`
di un `DataType` annidato **contiene i nomi dei campi**, che vengono dal file
(vedi tranche 3, `driver-common`). In `driver-ipc` non ce n'erano: i messaggi
parlavano di fasi e di indici, mai di forme.

### La conservazione degli assi è già sotto test

```rust
assert_eq!(errore.phase, plenora_io_model::ErrorPhase::Read);
assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
```

Quel test — che verifica anche che il rifiuto **preceda** il panico di arrow
invece di seguirlo — passa senza modifiche. È la prova che `phase` e `code` non
sono cambiati: se `err` avesse mutato categoria o codice, sarebbe rosso.

## L'unica fuga di payload

`Arrow IPC: plenora.field_id={} non coincide con l'indice fisico {}`.

Il `plenora.field_id` dichiarato **viene dai metadati del file**: è un numero
letto dal payload, e il vincolo ratificato ammette solo indici, conteggi, tetti
e codici strutturali. Sparisce.

L'**indice fisico** invece resta: lo produce la nostra enumerazione dello
schema, non il file. È esattamente la distinzione che il vincolo chiede di fare,
e questo sito è il caso più netto incontrato finora — due numeri nella stessa
frase, uno lecito e uno no.

Il controllo di coerenza che genera l'errore resta invariato: è la guardia
contro un `.arrow` ostile che dichiara un `field_id` fuori range, il quale a
valle produrrebbe un `batch.column(index)` panic.

## Verifica (livello 1)

* `scripts/check_errori_redatti.py`: **95 → 88** in cinque crate; **nove**
  componenti migrati e a zero;
* sonde del censimento: 7/7;
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: pulito;
* `cargo test --workspace --all-features`: verde su 31 binari;
* gate specifici verdi; registro dei fallback **invariato a 115**;
* replay deterministico di **3 442 input** su `ipc_reader` e `ipc_to_gpkg`
  senza crash, poi smoke sugli stessi due senza finding — il replay prima.

## Checkpoint di livello 2

È dovuto: questa è la terza tranche di driver dopo `107b7b5` — `driver-gpkg`,
`driver-geoparquet`, `driver-ipc`.

Va eseguito con `scripts/s9-checkpoint.sh` **sull'esatto SHA di questo commit**,
ad albero pulito. L'eventuale evidenza `passed` andrà in un **commit distinto**,
che non eredita la misura: il commit che pubblica un'evidenza ha per forza un
SHA diverso da quello verificato.

## Restano

`driver-filegdb` (22), `driver-shp` (22), `driver-dxf` (20), `driver-xls` (18),
e **per ultima** `plenora-io-cli` (6), dopo tutti i driver.
