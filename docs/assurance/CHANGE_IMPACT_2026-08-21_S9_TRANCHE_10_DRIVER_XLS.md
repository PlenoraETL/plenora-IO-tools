# Change impact analysis — S9 tranche 10: `driver-xls` redatto

Data: 2026-08-21. Sigla: **S9 / tranche 10**.
Baseline: `d62db6f` (INFRA-3).
`plenora-io-error-v1` **invariato**.
Qualifica: **livello 1** — verificato, non è un checkpoint.

## La fuga che il compilatore ha confermato

Il censimento cercava, fra le altre cose, i nomi di foglio. `xls_err` era il
sospetto: dodici percorsi di scrittura che riportavano il `Display` di
`rust_xlsxwriter::XlsxError`.

Scrivendo `classe_xlsx` avevo mappato le varianti come se fossero *unit
variant*. Il compilatore ha risposto:

```
error[E0532]: expected unit struct, unit variant or constant,
              found tuple variant `E::SheetnameReused`
```

**Sette varianti portano il nome del foglio come dato.**
`SheetnameReused(String)`, `SheetnameLengthExceeded(String)`,
`SheetnameCannotBeBlank(String)`, `SheetnameContainsInvalidCharacter(String)`,
`SheetnameStartsOrEndsWithApostrophe(String)`,
`UnknownWorksheetNameOrIndex(String)`, `ParameterError(String)`.

Il `Display` le stampa. Non era un rischio teorico: era una fuga attiva su
dodici percorsi, della stessa forma di `sql_err` in `driver-gpkg`.

`classe_xlsx` la chiude come `classe_sqlite`: un vocabolario nostro, chiuso,
che tiene distinte le cause senza far uscire nulla. Il test lo verifica con un
nome che si riconosce:

```rust
let errore = xls_err(E::SheetnameReused("Foglio segreto".to_owned()));
assert!(errore.message.contains("nome del foglio gia' usato"));
assert!(!errore.message.contains("Foglio segreto"));
```

**Il test è stato scritto subito**, non rimandato al prossimo checkpoint: la
copertura differenziale l'avrebbe trovato fra tre tranche, e nel frattempo
sarebbe stato un vocabolario che nessuno attraversa.

## Censimento a due classi

| Via | Forma | Occorrenze |
|---|---|---:|
| `err` | `impl Into<String>` | 55 chiamanti |
| **`xls_err`** | `XlsxError` → `err(format!("XLSX: {e}"))` | **12 chiamanti**, 7 varianti con il nome del foglio |
| `err(format!("…: {e}"))` | cancellazione di struttura | **10** — calamine, `zip`, `quick_xml`, arrow |
| `format!` con valori letti | fuga di payload | 1 |
| `format!` con valori d'opzione | fuga di testo | 4 |
| `Result<_, String>` | — | 0 |
| `DeError::custom` | — | 0 |

Usi legacy diretti: **18**.

### Le altre fughe

* **valore di cella**: `numero XLSX non rappresentabile come f64: {n}` — `n` è
  una cella del dataset in ingresso;
* **nomi di colonna** (`wkt_column`, `x_column`, `y_column`): valori d'opzione,
  stessa situazione di `driver-csv` — lo schema li dichiara `Testo`, quindi il
  rifiuto nasce dal confronto con l'intestazione di *questo* foglio e il
  `RejectedOptionToken` non si applica. **Perdita diagnostica dichiarata**, già
  registrata nella CIA della tranche 5 come decisione dovuta;
* **`geometry_encoding`**: non è una perdita — lo schema lo dichiara
  `Enumerato`, il ramo nel driver è difensivo.

### Cosa resta

Tutti i tetti: parti del contenitore, byte per parte XML, rapporto di
decompressione, righe e colonne, byte dello spool. Sono conteggi e limiti
**nostri**, non valori letti — restano come `CuratedBetween` e `CuratedWith`.

L'asse della coordinata (`X`/`Y`) resta: è un `&'static str` del nostro codice.

## Verifica (livello 1)

* `scripts/check_errori_redatti.py`: **88 → 70** in quattro crate; **dieci**
  componenti migrati e a zero;
* sonde del censimento: 7/7;
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: pulito;
* `cargo test --workspace --all-features`: verde su 31 binari, compreso il nuovo
  `la_classe_xlsx_traduce_ogni_variante_costruibile`;
* gate specifici verdi; registro dei fallback **invariato a 115**;
* replay deterministico di **2 175 input** su `xlsx_reader` senza crash, poi
  smoke senza finding.

## Nota per il prossimo checkpoint

`S9_CHECKPOINT_BASE` va impostata a
**`effc4abe3f74ade083dbed72c94c286748809d9f`** — l'ultima revisione
**verificata**, non l'ultimo commit. Così il delta da qualificare include anche
INFRA-3 e i test che ha prodotto.

L'evidenza su `effc4ab` resta valida secondo i criteri allora applicati: i test
aggiunti dopo **non le vanno attribuiti retroattivamente**.

## Prossimo passo

Tranche 11: `driver-filegdb` (22) — percorso feature-gated e forte presenza di
testo GDAL. Poi `driver-shp` (22), e alla chiusura della tranche 12 il
checkpoint di livello 2.
