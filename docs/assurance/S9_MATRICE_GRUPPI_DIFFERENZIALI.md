> **Nota del 2026-08-21** — questo documento è nuovo; la nota è qui perché la
> regola append-only vale dalla sua pubblicazione in avanti. Le correzioni
> vanno in coda, non nel corpo.

# S9 — matrice dei gruppi differenziali scoperti

Origine: diagnostica differenziale del checkpoint su `8e64965`, baseline
`effc4ab`, incrociata con la copertura da replay misurata su `5e7378d`.

**49 gruppi, 382 righe.** I totali dei due assi riconciliano entrambi a 49.

## I due assi sono indipendenti

Vanno tenuti separati perché rispondono a domande diverse, e confonderli è il
modo in cui una regressione contrattuale si nasconde dietro una lacuna di
copertura — o viceversa.

### Asse 1 — quartetto `(category, phase, code, retry)`

| | gruppi |
|---|---:|
| invariato | **48** |
| **modificato** | **1** |
| *totale* | *49* |

L'unico modificato è `plenora-io-core::account`: il sito batch-contro-contratto
usava `PlenoraIoError::new(Schema, Validate, …)`, che imposta `code = Generic`,
ed è diventato `schema_redatto`, che imposta `code = Schema`.

Gli altri tre assi identici. **Nel diff non compare una sola riga di
`ErrorCategory::`, `ErrorPhase::`, `RemoteEffect::` o `RetryDisposition::`
cambiata**: è la ragione per cui il difetto è sopravvissuto a una CIA che
dichiarava il contrario in buona fede.

Corretto in `6da790b`, ripristinando `redatto(IoErrorCode::Generic, …)`.
`Schema` sarebbe più preciso, ma renderlo tale è una decisione da ratificare,
non una conseguenza di un refactor sui messaggi.

Il gate che lo impedisce è `scripts/check_quartetto_sito.py`, con snapshot per
`percorso::funzione` in sequenza **ordinata per apparizione** — non un
multiinsieme, che sarebbe invariante per permutazione e lascerebbe passare due
siti che si scambiano il quartetto.

### Asse 2 — disposizione di copertura

| Disposizione | Gruppi | Righe | Bucket |
|---|---:|---:|---|
| `test_tabellare` | 40 | 297 | ASSURANCE-N1 |
| `seme_fuzz` | 5 | 74 | ASSURANCE-N1 |
| `strutturale` | 2 | 4 | nessuna prova dovuta |
| `difensivo` | 1 | 3 | nessuna prova dovuta |
| `chiuso` | 1 | 4 | già chiuso in `5500b74` |
| **totale** | **49** | **382** | |

**Bucket strettamente S9: vuoto.** L'unico gruppo che apparteneva a S9 è
`plenora-io-core::account`, chiuso ripristinando il quartetto; non richiede un
test di copertura perché la proprietà è ora garantita dallo snapshot.

Per i 48 con quartetto invariato vale la regola ratificata: la sola sostituzione
del messaggio non richiede un test per ramo — bastano i vincoli del tipo e i
test ostili di non-fuga per driver.

## Raggiungibilità

| Stato | Gruppi |
|---|---:|
| raggiunti, almeno in parte, dal replay deterministico | **6** |
| non raggiunti da nulla | 42 |
| fuori dai target misurati | 1 |

I sei raggiunti hanno **reachability e panic-safety** provate, non il contratto:
un fuzz target verifica che non si panichi, non che il quartetto sia quello
giusto. E il report è aggregato sul corpus, quindi **il seme non è
identificato**: sapere che *qualcuno* fra 1 419 input raggiunge il ramo non dice
quale. Per trasformarli in test la via più economica è una fixture costruita
dalla precondizione del ramo, che è scritta nel codice.

### Perché il fuzzing aiuta così poco

`shp_wkb` **non è un reader di Shapefile**: la sua intestazione dice
«conversione WKB ⇄ shape ESRI», e chiama `__fuzz_wkb_roundtrip`. **Il parsing di
`.shp` e `.dbf` non ha alcun fuzz target.** È il motivo per cui
`read_dbf_layout`, `leggi_descrittori_dbf`, `infer_shp_schema`, `spawn_parser` e
`next_physical` non sono raggiunti da nulla.

`xlsx_reader` e `gpkg_reader` *sono* reader completi, e infatti raggiungono
qualcosa — ma poco: i rami d'errore restano in gran parte intatti anche dopo
2 272 e 870 input.

Si affianca alla lacuna già dichiarata per FileGDB nell'addendum alla tranche 11.

## Il debito non è causato da S9

Verificato su `effc4ab`: le stesse righe avevano `conteggio=0` **prima** della
migrazione. S9 non ha reso non verificato nulla — ha reso **visibile** ciò che
non lo era, perché un errore da una riga è diventato quattro e la diagnostica
differenziale ha cominciato a guardare.

La distinzione decide chi paga: **S9 garantisce ciò che ha cambiato; ciò che ha
soltanto illuminato è censito in ASSURANCE-N1**, che è release-blocking e non
sparisce.

## Che cosa resta aperto, e dove

| | dove |
|---|---|
| 45 gruppi senza copertura | `docs/assurance/ASSURANCE_N1_copertura_negativa.json` |
| fuzz target per il reader `.shp`/`.dbf` | ASSURANCE-N1, disposizione `seme_fuzz` |
| spike di fattibilità fuzz per FileGDB | da aprire, bounded, con due esiti ammessi |
