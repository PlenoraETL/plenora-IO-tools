# Change impact analysis — barriera XLSX contro i panici di `calamine`

Data: 2026-08-17. Sigla: **XLSX-HARDENING**.
Baseline: `15dc4e5` (INFRA-0).

## Problema

Lo smoke di `scripts/fuzz-smoke.sh` eseguito dopo INFRA-0 ha prodotto un
finding su `xlsx_reader`, target che **non** era in quarantena. Non è una
regressione: nessun sorgente Rust era cambiato dalla corsa precedente. Il
corpus è cresciuto di 268 unità su 37.345 esecuzioni e ha raggiunto un difetto
che era lì da sempre.

`calamine` 0.36.1 converte il riferimento testuale di una cella in coordinate
accumulando senza controlli (`src/xlsx/mod.rs`):

```rust
c @ b'A'..=b'Z' => col = col * 26 + (c - b'A') as u32 + 1,   // 2837
c @ b'a'..=b'z' => col = col * 26 + (c - b'a') as u32 + 1,   // 2838
c @ b'0'..=b'9' => row = row * 10 + (c - b'0') as u32,       // 2853
```

Il numero di lettere non è limitato. Sette bastano a superare `u32::MAX`;
l'input responsabile ne porta otto (`r="Bncasufw"`, 20.424.890.639).

### Perché contava in produzione

1. **Non era un artefatto del profilo di fuzzing.** `Cargo.toml` dichiara
   `overflow-checks = true` anche in `[profile.release]`, per scelta motivata:
   un'aritmetica che avvolge in silenzio, in un componente che legge file non
   fidati, è il verso sbagliato. Lo stesso input produceva quindi lo stesso
   panico nel binario spedito.
2. **Il percorso era quello pubblico.** `XlsDriver::open` → `infer_layout` →
   `for_each_dense_row` → `next_cell`.
3. **Non c'era barriera.** `plenora-io-core::driver::leggendo_arrow` copre i
   driver arrow — è la ragione per cui i tre target arrow in quarantena hanno
   un rischio di produzione diverso da quello descritto dal finding. XLSX non
   aveva l'equivalente: il panico attraversava il confine della libreria invece
   di diventare un errore tipizzato. Il `catch_unwind` in
   `plenora-io-cli/src/main.rs` protegge la CLI, non chi usa le crate come
   libreria.

Severità: alta sul contratto d'errore, nulla sulla memoria — nessun report
AddressSanitizer, l'overflow viene intercettato dal controllo del compilatore
prima di produrre un indice sbagliato. Con `overflow-checks` disattivati sarebbe
stato peggio e più silenzioso: coordinate plausibili e false.

## Cosa cambia

### La barriera sta nel driver, ed è stretta

`driver-xls::leggendo_calamine` avvolge le **sole** chiamate che toccano
l'input non fidato:

| Chiamata | Dove |
|---|---|
| `open_workbook` | `XlsDriver::open` |
| `sheet_names` | `XlsDriver::open`, solo se il foglio non è dichiarato |
| `worksheet_cells_reader` | `LettoreCelleSorvegliato::nuovo` |
| `dimensions` | `LettoreCelleSorvegliato::dimensioni` |
| `next_cell` + `get_position` + `get_value` | `LettoreCelleSorvegliato::prossima_cella` |

Non avvolge la logica del driver che ci sta attorno. Avvolgerla tutta avrebbe
un costo preciso: un difetto **nostro** — un `checked_sub` dimenticato in
`data_row_width`, un indice sbagliato in `for_each_dense_row` — verrebbe
riportato come «calamine in panico», cioè attribuito alla libreria e archiviato
come debito a monte. Un difetto nostro deve restare visibile come nostro.

Posizione e valore lasciano la barriera già copiati in tipi nostri: nessun tipo
`calamine` sopravvive alla chiamata, quindi non resta un accessore che possa
panicare più tardi, fuori dal `catch_unwind`.

### Lo stato attraversato dal panico viene scartato, non solo non usato

`AssertUnwindSafe` dichiara che lo stato attraversato non viene più osservato.
Qui è vero per costruzione:

* **il lettore di celle** vive dentro `LettoreCelleSorvegliato`, che al primo
  fallimento lo lascia cadere e mette `None`. Ogni chiamata successiva trova
  `None` e restituisce errore: non esiste un modo di continuare a leggere celle
  da uno stato che il panico ha attraversato — nemmeno per distrazione, in un
  ciclo che oggi non c'è e domani potrebbe esserci. Vale per qualunque
  fallimento, non solo per i panici: dopo un errore di `calamine` il flusso XML
  è comunque a metà;
* **il workbook** vive solo dentro `open`, che lo lascia cadere **prima** di
  propagare l'esito di `infer_layout` (`drop(wb)` esplicito fra la chiamata e
  il `?`). Non c'è un ramo d'errore che possa ancora toccarlo.

È la differenza fra «nessuno lo usa più» verificato dal compilatore e riletto a
ogni modifica futura.

### L'errore è tipizzato e redatto

`PlenoraIoError::format("xls", …)` — categoria `DataMapping`, **fase `Read`**,
codice `Format`, driver `xls`. Il messaggio pubblico porta solo l'impronta
redatta del panico:

    calamine in panico durante la lettura (impronta 3f2a…)

Mai il testo del panico, il percorso del file, il nome del foglio o un valore
di cella. L'impronta è FNV-1a a 64 bit sul messaggio: stabile fra esecuzioni,
quindi due occorrenze dello stesso difetto si correlano, ma non invertibile.

La funzione che la calcola è **la stessa** della barriera arrow, esposta da
`plenora-io-core::driver::impronta_di_panico`. Riscriverla accanto a ogni
barriera l'avrebbe resa inutile: un'impronta vale come identificatore solo se è
la stessa funzione ovunque.

## Verifica

### Il test osserva la barriera

`un_xlsx_che_fa_panicare_calamine_diventa_un_errore_del_driver` chiama davvero
`XlsDriver::open` sul seme versionato, con le stesse due dichiarazioni di
geometria del fuzz target, e prova quattro cose:

| Proprietà | Come |
|---|---|
| nessun panico | se la barriera non c'è, la chiamata abbatte il processo di test |
| errore tipizzato | `code == Format`, `category == DataMapping`, `phase == Read`, `driver == "xls"` |
| nessun dataset parziale | `open` non restituisce alcun handle: non c'è un `OpenDatasetHandle` a metà da osservare |
| messaggio redatto | non contiene `overflow`, `multiply`, il nome del file, `fuzz/seeds`, il valore di cella, né il percorso sorgente a monte; contiene un'impronta di 16 cifre esadecimali |

La barriera scatta su **entrambe** le dichiarazioni: il difetto è nel parser
delle celle, che le precede tutte.

### Verifica per mutazione

Sostituito il corpo di `leggendo_calamine` con la sola chiamata
all'operazione — barriera rimossa, tutto il resto invariato — il test fallisce:

    thread 'tests::un_xlsx_che_fa_panicare_calamine_diventa_un_errore_del_driver'
    panicked at calamine-0.36.1/src/xlsx/mod.rs:2838:38:
    attempt to multiply with overflow

Ripristinato il file (digest identico), il test torna verde. Il test osserva la
mitigazione, non se stesso.

### Il seme

`fuzz/seeds/xlsx_reader/riferimento-cella-oltre-u32.xlsx`, 5.428 byte,
SHA-256 `cc7be666…5295a`, provenienza e digest in `fuzz/seeds/README.md`.
`cargo fuzz tmin` non riesce a ridurlo: è un archivio ZIP, e togliere byte
rompe il contenitore prima di arrivare al parser, quindi il crash sparisce
invece di restare su un input più piccolo.

## Quarantena, e perché stavolta non sposta il rischio

`xlsx_reader` entra in `fuzz/quarantine.txt` **dopo** la mitigazione e per una
ragione di strumento: `libfuzzer-sys` installa un panic hook che chiama
`abort()` prima che l'unwinding cominci (0.4.10, `src/lib.rs:92-95`), apposta
perché un `catch_unwind` nel codice sotto test non possa nascondere difetti al
fuzzer. Il target resta quindi rosso anche a barriera funzionante.

È la stessa situazione dei tre target arrow, ed è il caso che l'intestazione di
`quarantine.txt` descrive: «un difetto a monte già mitigato da noi, che
l'harness non può però osservare». La riga cita mitigazione, test e seme, così
la copertura resta rintracciabile senza il fuzzing.

Quarantinare **prima** della mitigazione avrebbe spostato il rosso dalla CI al
prodotto. È il motivo per cui l'ordine è questo.

## A monte

Segnalazione pronta e **non pubblicata**, con reproducer costruito da zero (non
un nostro artefatto), in
[`UPSTREAM_CALAMINE_CELL_REFERENCE_OVERFLOW.md`](UPSTREAM_CALAMINE_CELL_REFERENCE_OVERFLOW.md).
Richiede autorizzazione: aprire una issue è una comunicazione pubblica a nome
del progetto.

Un aggiornamento di `calamine` **non** sostituisce la barriera: chiuderebbe
questo difetto, non la classe. La barriera si toglie per decisione separata, la
stessa regola della barriera arrow.

## Perimetro e rischi residui

Toccati: `crates/driver-xls/src/lib.rs`, `crates/plenora-io-core/src/driver.rs`
(sola esposizione di `impronta_di_panico`, comportamento invariato),
`fuzz/quarantine.txt`, `fuzz/seeds/`, documentazione.

Non toccati: formati su disco, contratti pubblici, CLI, altri driver.

Residui dichiarati:

* la barriera copre le chiamate `calamine` **elencate sopra**. Una chiamata
  nuova aggiunta domani fuori dalla barriera non è impedita da niente:
  `next_cell` è dentro un tipo sorvegliato, `open_workbook` e `sheet_names` no.
  Il perimetro è piccolo e sta tutto in `open`, ma è convenzione, non tipo;
* il difetto a monte resta aperto: la percentuale di superficie `calamine` che
  può panicare non è nota, e la barriera la copre per costruzione — cattura
  qualunque panico, non solo questo — ma non la riduce;
* `xlsx_reader` resta rosso nello smoke finché la issue a monte non è chiusa,
  come i tre target arrow.
