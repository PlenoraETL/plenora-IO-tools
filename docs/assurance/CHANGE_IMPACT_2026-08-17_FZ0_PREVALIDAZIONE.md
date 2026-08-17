# Change impact analysis — il panico delle librerie esterne è impedito, non catturato

Data: 2026-08-17. Sigla: **FZ-0**. Baseline: `1dd5155` (INFRA-0.1).

Stato: **parziale e dichiarato tale**. Tre target su quattro escono dalla
quarantena; `geoparquet_reader` resta dentro. FZ-0 non chiude.

## Problema

XLSX-HARDENING aveva messo una barriera `catch_unwind` attorno a `calamine`, e
i tre target arrow ne avevano una equivalente da prima. La barriera ripristina
il contratto d'errore, ma **non chiude un finding**: il panico è avvenuto, e
sotto `libfuzzer-sys` diventa `abort()` prima che l'unwinding cominci. Un target
mitigato resta quindi rosso, e la quarantena — nata per non trasformare il gate
in un rosso permanente — finisce per coprire anche le regressioni nuove di quei
driver.

FZ-0 sposta la difesa: il panico non deve avvenire. La barriera resta come
difesa in profondità, non come mitigazione.

## Vincolo

Nessuna patch, fork o vendorizzazione di dipendenze esterne; nessun
restringimento del contratto (`ALL_ARROW_TYPES` invariato); nessuna modifica ai
profili di fuzzing o ai `debug_assert!`. Solo prevalidazione bounded e
fail-closed nel nostro boundary, e aggiornamenti a release ufficiali.

## Metodo

Ogni correzione è stata preceduta e seguita da un **replay deterministico** su
corpus, semi e artefatti — `scripts/fuzz-replay.sh`, `-runs=0`, quarantena
ignorata di proposito — e poi da fuzzing attivo. Il replay è il confronto
prima/dopo che una campagna, che esplora, non può dare.

Baseline registrata prima di toccare qualsiasi cosa: **4 target falliti, 9
puliti su 11.883 input**.

## Cosa è stato chiuso

### XLSX — riferimenti di cella oltre i limiti del formato

`calamine` accumula colonna e riga di un riferimento `A1` in `u32` senza
controlli (`xlsx/mod.rs:2837-2853`); sette lettere bastano a traboccare.

Il driver ora scorre le parti XML del contenitore con `quick-xml` e verifica
ogni attributo `r` e `ref` contro i limiti che **il formato stesso** dichiara:
ultima colonna `XFD`, ultima riga 1.048.576. Non è una regola nostra e non
cambia con la libreria, quindi nessun file conforme viene rifiutato.

### Arrow IPC — validazione dichiarativa e ricorsiva

`driver-common::prevalida_arrow` deriva da ogni tipo una **classe di layout**
che dice quanti nodi e buffer il decoder consuma, in quale ordine e quali figli
attraversa, rispecchiando `create_array` della versione pinnata. I controlli
discendono dal layout, non dai singoli crash:

| Regola | Difetto che impedisce |
|---|---|
| valori di enum inesistenti, `Type::NONE`, tag di unione incoerente, endianness | `convert.rs` — una ventina di `panic!`/`unimplemented!`/`unwrap()` |
| ogni buffer dentro il corpo del messaggio | `slice_with_length`, `immutable.rs:288` |
| bitmap di validità ≥ `ceil(len/8)` con `null_count > 0` | `create_struct_array` → `BooleanBuffer::new`, `boolean.rs:128` |
| identificatori di unione ≥ `len`, offset densi ≥ `len*4` con moltiplicazione checked | ramo unione, affettamento senza controlli |
| buffer di offset con lunghezza multipla dell'elemento | `validate_offsets` → `typed_data`, `immutable.rs:323` |
| conteggio variadico presente e `+2` rappresentabile | ramo vista |

Tetti su campi visitati, profondità, byte di metadati e blocchi; aritmetica
checked ovunque; ogni condizione non verificabile ferma la lettura.

### GeoParquet — metadati Thrift

Prima di costruire il reader: offset di dizionario e pagina dati,
`compressed_size`, `num_values`, righe e dimensione del gruppo — non negativi,
rappresentabili e dentro il file. Impedisce `ColumnChunkMetaData::byte_range`
(`file/metadata/mod.rs:1063`). Lo schema `ARROW:schema` incorporato nel footer
passa dalla stessa validazione dello schema IPC.

### Ciò che *non* è stato duplicato, e perché

`ArrayData::try_new` chiama `validate_data()`: arrow valida già contenuto degli
offset, monotonia, ultimo offset e UTF-8 **in modo fallibile**. Riscrivere quelle
regole fuori avrebbe aggiunto solo il rischio di sbagliarle in modo diverso e
rifiutare file validi. La validazione mira ai punti dove il decoder aggira il
proprio costruttore fallibile; il resto resta ad arrow, che lo rifiuta bene.

## Gate anti-chiamata-nuda

Una prevalidazione vale quanto la sua copertura: nulla lega la verifica alla
costruzione del reader, e basta un percorso nuovo per rientrare dalla porta di
servizio. `scripts/check_prevalidazione_decoder.py` pretende che ogni
costruzione del decoder sia preceduta dalla prevalidazione **nella stessa
funzione**, e che nessuna crate fuori perimetro lo costruisca.

Ha trovato subito due percorsi scoperti — entrambi codice di test, ora esclusi
in modo dichiarato e **contato** («4 costruzioni escluse»), così l'esclusione non
diventa un buco silenzioso. Una delle otto sonde negative ha trovato una
debolezza del gate stesso: un commento che nominava la prevalidazione la
soddisfaceva.

## Impronta del panico rimossa

I messaggi delle barriere portavano un'impronta FNV del testo del panico. È un
valore che nasce dall'input e finisce in un errore serializzato, registrato e
passato agli altri componenti: per un bordo che promette di non far uscire nulla
di derivato dal payload è una promessa in meno. Sostituita da messaggi statici
curati; il macchinario FNV, rimasto senza usi, è stato eliminato.

## Esiti misurati

| Target | Replay | Fuzzing attivo | Stato |
|---|---|---|---|
| `xlsx_reader` | 740 input puliti | 60.511 esecuzioni | **fuori quarantena** |
| `ipc_reader` | 751 input puliti | 149.425 esecuzioni | **fuori quarantena** |
| `ipc_to_gpkg` | 896 input puliti | 139.272 esecuzioni | **fuori quarantena** |
| `geoparquet_reader` | — | crash | **in quarantena** |

## Il residuo, senza ammorbidirlo

`geoparquet_reader` fallisce su `parquet/src/arrow/decoder/dictionary_index.rs:46`:
`let bit_width = data[0]` prende come `u8` grezzo il primo byte della sezione
valori di una data page a dizionario, senza controllo di intervallo e senza
verificare che la sezione non sia vuota.

Caratterizzato in tre configurazioni, con `debug_assertions` e overflow-checks
**osservati a runtime** e non dedotti dal profilo:

| Configurazione | `debug_assertions` | Esito |
|---|---|---|
| fuzz | true | panic `bit_util.rs:697` → `abort()` |
| release distribuita | **false** | panic `bit_util.rs:719` → catturato |
| release + debug-assertions | true | panic `bit_util.rs:697` → catturato |

**Il panico esiste anche nella release spedita**, a una riga diversa, perché il
workspace tiene `overflow-checks = true`. Non è un artefatto del profilo di
fuzzing, e una registrazione precedente che lo dava per tale era sbagliata.

Comportamento del binario distribuito: `read` esce con **2**, envelope
`FORMAT_ERROR`/`data_mapping`/fase `Read`/retry `never`, **zero batch e zero
righe**, 25 ms, picco RSS 8,4 MB, deterministico su dieci esecuzioni, regge
`ulimit -v 512 MiB` e `ulimit -t 5s`. Il contratto d'errore tiene; nessun dato
parziale o errato esce.

**Perché non è prevalidabile qui.** Il controllo sarebbe banale — `bit_width <=
32`, sezione non vuota — ma raggiungere quel byte richiede rifare la logica di
layout della pagina: per una data page V1, quella che arrow scrive per default,
i valori cominciano dopo le sezioni dei livelli, la cui posizione dipende da
`max_rep_level`/`max_def_level` e dalla codifica dei livelli, che lo spec ammette
sia `RLE` sia `BIT_PACKED`. Servirebbe inoltre iterare tutte le pagine prima di
costruire il reader, cioè decomprimere l'intero file una seconda volta a ogni
lettura. È replicare il decoder e cambiare le prestazioni, non correggere.

### Lo spike del `PageReader` decoratore

Prima di accettare l'arresto è stato valutato un `PageReader` **decoratore** via
API pubbliche: validare inline il primo byte della sezione valori delle sole
data page a dizionario, senza secondo passaggio e senza toccare la dipendenza.

Il decoratore è scrivibile. `Page::DataPage` espone `def_level_encoding`,
`rep_level_encoding` e `num_values`; `Page::DataPageV2` espone i due byte-length
dei livelli. La sezione valori è quindi localizzabile.

Il blocco è l'**iniezione**, e non è una questione di mole: l'unica porta
pubblica che accetta un `RowGroups` custom è
`ParquetRecordBatchReader::try_new_with_row_groups`, che nel corpo chiama
`build_array_reader(levels.levels.as_ref(), &ProjectionMask::all())` e non
accetta `RowFilter`. Perde quindi **projection pushdown e pruning per
predicato**, che il driver ha e che i suoi test verificano
(`projection_pushdown_reads_only_requested`, `row_group_pruning_skips_blocks`,
`spatial_pruning_skips_blocks`).

Rifare ciò che fa `builder.build()` non è possibile dall'esterno:
`ArrayReaderBuilder::build_array_reader` è `pub`, ma prende
`Option<&ParquetField>`, e `ParquetField` è `pub(crate) use` in
`arrow/schema/mod.rs:47` — non nominabile né ottenibile fuori dal crate. Anche
`FieldLevels.levels` è `pub(crate)`.

Conservare projection e pruning **e** iniettare la validazione è quindi precluso
dall'API, non costoso.

La condizione di arresto di FZ-0 è stata quindi accettata: si attende una
release ufficiale corretta a monte. Bozza di segnalazione pronta e **non
pubblicata** in
[`UPSTREAM_PARQUET_DICTIONARY_BIT_WIDTH.md`](UPSTREAM_PARQUET_DICTIONARY_BIT_WIDTH.md);
caratterizzazione completa e artefatti in
`fuzz-findings/2026-08-17-geoparquet-bitwidth-dizionario/`.

## Perimetro e rischi residui

Toccati: `driver-common` (modulo nuovo), `driver-xls`, `driver-ipc`,
`driver-geoparquet`, `plenora-io-core` (sola rimozione dell'impronta),
`fuzz/quarantine.txt`, semi, gate, CI, documentazione. Dipendenze: nessuna
patch, nessun fork; `base64 =0.22.1`, già transitiva, promossa a diretta e
pinnata; `quick-xml`, `arrow-ipc` e `driver-common` aggiunte come dipendenze
diritte a crate che già le avevano nel grafo.

Non toccati: formati su disco, contratti pubblici, `ALL_ARROW_TYPES`, profili di
fuzzing, `debug_assert!`.

Residui dichiarati:

* `geoparquet_reader` resta rosso e in quarantena finché la correzione a monte
  non arriva. **FZ-0 non è chiuso**, e la regola zero-quarantena non è
  raggiunta;
* la validazione IPC rispecchia il comportamento della **versione pinnata** di
  `arrow-ipc`. Un aggiornamento va accompagnato da una rilettura di
  `create_array`: il pin esatto è sorvegliato da `check_dependency_pins.py`, la
  rilettura no;
* la prevalidazione XLSX copre le parti `xl/worksheets/*.xml` e `xl/workbook.xml`.
  Una parte nuova che portasse riferimenti A1 e che `calamine` attraversasse non
  sarebbe coperta;
* il gate anti-chiamata-nuda esclude il codice di test. L'esclusione è contata,
  non silenziosa, ma resta un'esclusione.
