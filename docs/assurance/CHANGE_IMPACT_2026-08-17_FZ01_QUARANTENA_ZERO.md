# Change impact analysis — quarantena zero

Data: 2026-08-17. Sigla: **FZ-0.1**. Baseline: `4c96f94` (FZ-0 parziale).

Stato: **FZ-0 chiuso**. Tredici target su tredici eseguiti, quarantena vuota.

## Problema

FZ-0 aveva chiuso tre target su quattro impedendo il panico invece di
catturarlo, e si era fermato su `geoparquet_reader`: `parquet` prende il bit
width degli indici di dizionario dal primo byte della sezione valori di una data
page senza controllarne l'intervallo
(`arrow/decoder/dictionary_index.rs:46`).

L'arresto era motivato da due valutazioni, entrambe **sbagliate nella
conclusione**:

* «prevalidarlo richiede un secondo passaggio di decompressione dell'intero
  file». Vero solo se la validazione guarda tutto il file. Non deve: projection
  e pruning hanno già deciso quali chunk verranno letti, e i metadati dicono
  quali usano il dizionario;
* «un `PageReader` decoratore non è iniettabile». Vero, ed è tuttora vero — ma
  la validazione non ha bisogno di essere *dentro* il reader. Un passaggio
  separato, limitato ai chunk selezionati e con lo stesso snapshot dei metadati,
  ottiene lo stesso risultato senza toccare la costruzione del reader.

La seconda valutazione era corretta come fatto e sbagliata come conclusione: da
«non posso decorare» non segue «non posso validare».

## Cosa fa la prevalidazione

`valida_bit_width_dizionario` gira in `open_layer_reader`, **dopo** che
projection e pruning hanno prodotto la loro selezione e **prima** di
`builder.build()`.

| Passo | Effetto |
|---|---|
| row group | solo quelli sopravvissuti al pruning |
| colonna | solo quelle incluse dalla `ProjectionMask`, via `leaf_included` |
| codifica | solo i chunk che dichiarano `RLE_DICTIONARY` o `PLAIN_DICTIONARY` nei **metadati**, senza toccare una pagina |
| pagina | una per volta dal `PageReader` della libreria, un byte guardato, niente trattenuto |

Lo snapshot dei metadati è quello della lettura, passato dal chiamante: se ne
rileggesse uno proprio, validazione e lettura potrebbero guardare due file
diversi.

Per una data page V1 la sezione valori comincia dopo le sezioni dei livelli, la
cui posizione dipende da `max_rep_level`/`max_def_level` e dalla codifica dei
livelli. `RLE` porta un prefisso di quattro byte; `BIT_PACKED` — deprecata ma
ammessa — no, e la sua dimensione si calcola. Una codifica diversa dalle due
**ferma la lettura** invece di far tirare a indovinare l'offset. Per V2 le due
lunghezze sono dichiarate nell'header e non c'è niente da dedurre.

## Tre difese della verifica stessa

La prevalidazione protegge la lettura, ma va protetta a sua volta: e' codice che
tocca input non fidato per mestiere, e i difetti che cerca puo' commetterli.

**Una sola apertura.** `open_layer_reader` apriva il file tre volte — schema,
prevalidazione, builder. Ora e' una `File::open` con `try_clone` per ogni
consumatore. Non e' economia di syscall: due `open` distinti possono cadere su
**due file diversi** se il percorso viene sostituito fra l'uno e l'altro, e la
verifica varrebbe per un file che non e' quello letto.

**Messaggi statici.** Il messaggio del bit width riportava il valore letto. E'
un valore che nasce dal payload e finisce in un errore serializzato e
registrato: la stessa promessa che FZ-0 aveva appena ripristinato togliendo
l'impronta FNV, violata nella correzione successiva. Tutti i messaggi della
prevalidazione Parquet sono ora costanti.

**Tetto sui metadati, e cosa non copre.** `SerializedPageReader::get_next_page`
decomprime prima di restituire, e `PageMetadata` — l'unica cosa che
`peek_next_page` offre — non porta le dimensioni: una verifica per pagina non e'
possibile con l'API pubblica. Il tetto e' quindi per chunk, sul
`uncompressed_size()` dichiarato nel footer.

E' **assoluto** e non un rapporto di decompressione: il rapporto, come quello
applicato al contenitore XLSX, rifiuterebbe anche file leciti molto
comprimibili, e sarebbe un restringimento del contratto invece di una difesa.

**Non e' pero' una protezione dagli header incoerenti**, e descriverlo cosi'
sarebbe sbagliato: l'allocazione segue `PageHeader.uncompressed_page_size`
(`file/serialized_reader.rs:447`), un `i32` per pagina indipendente dal totale
del chunk. Un file che dichiari un chunk piccolo e una pagina enorme supera
questo tetto e fa comunque chiedere fino a circa 2 GiB per pagina. E'
esaurimento di risorse, riguarda il lettore quanto la verifica — entrambi
decomprimono le stesse pagine — ed e' registrato a parte in
`fuzz-findings/2026-08-18-parquet-uncompressed-page-size/`.

> **Aggiornamento 2026-08-18 (FZ-0.2).** Il residuo e' chiuso: la
> prevalidazione degli header di pagina (`driver-geoparquet::pagine`) rifiuta
> una pagina che dichiari piu' byte non compressi del proprio chunk, prima che
> il decoder allochi. Da allora questo tetto **e'** una garanzia effettiva
> sull'allocazione — ma solo insieme a quella, e il paragrafo sopra resta vero
> per il tetto preso da solo. La misura ha anche corretto la descrizione del
> rischio: l'esito non e' un panico ma un **abort** del processo, che nessun
> `catch_unwind` intercetta. Vedi
> [`CHANGE_IMPACT_2026-08-18_FZ_0_2_PREVALIDAZIONE_PAGINE.md`](CHANGE_IMPACT_2026-08-18_FZ_0_2_PREVALIDAZIONE_PAGINE.md).

## Costo misurato

Mediana di nove esecuzioni, stesso file, stessa esecuzione, misurando
`open_layer_reader` separatamente dal drenaggio: il totale è dominato dalla
decodifica e dal rumore della macchina, e non direbbe niente.

| Caso | Byte | `open_layer` senza | con | **costo** |
|---|---|---|---|---|
| senza dizionario | 3,35 MB | 20,8 µs | 21,6 µs | **+0,8 µs** |
| con dizionario | 537 KB | 21,9 µs | 86,8 µs | **+65 µs** |
| 16 colonne a dizionario | 809 KB | 57,4 µs | 309 µs | **+252 µs** |
| dizionario + zstd | 147 KB | 20,7 µs | 467 µs | **+447 µs** |
| peggiore: 16 colonne + zstd | 155 KB | 60,3 µs | 685 µs | **+624 µs** |

Il costo è **una decompressione in più delle sole colonne a dizionario
effettivamente lette**, non del file. Senza dizionario non si paga niente,
perché il filtro viene dai metadati. Con compressione il lavoro si duplica
davvero, ed è lì che sta il peggiore misurato: 624 µs contro 7,5 ms di
drenaggio.

**Nota metodologica.** La prima misura, sul solo totale, dava +51% sul caso
*senza* dizionario — un risultato che contraddiceva il progetto, perché quel
caso non tocca una pagina. Era rumore del drenaggio. Separare le fasi ha
mostrato 0,8 µs. Riportare il primo numero avrebbe fatto respingere una
soluzione che funziona.

## Verifica

| Criterio | Esito |
|---|---|
| seme rifiutato con `Err` nel target fuzz invariato | errore tipizzato, fase `Read`, codice `Format`, **non** dalla barriera: il decoder non viene raggiunto |
| replay deterministico | **22.129 input su 13 target**, nessun crash, artefatti compresi |
| fuzzing attivo su `geoparquet_reader` | **122.801 esecuzioni**, nessun crash |
| projection e pruning conservati | 15/15 test, inclusi projection pushdown e i due pruning |
| smoke completo | **13/13 target eseguiti**, 0 in quarantena, nessun finding |

## Quarantena zero, e come resta zero

`fuzz/quarantine.txt` non ha più righe attive. Il meccanismo di skip resta —
serve, perché un gate che fallisce sempre smette di essere letto — ma
`scripts/check_quarantena_fuzz.py` rende una riga attiva un **blocco al
rilascio**, con sette sonde negative.

Non vieta di quarantinare. Se un domani un finding non fosse chiudibile la riga
si scrive e il gate diventa rosso: è il punto. Si aggira ratificando
l'eccezione, e la ratifica lascia traccia dove il file da solo non ne
lascerebbe.

## Un difetto preesistente, segnalato e non corretto

Per condividere la selezione fra validazione e lettura, i due pruning
restituiscono ora l'insieme dei row group invece di applicarlo al builder. Il
refactor ha reso visibile un difetto che c'era già:

`apply_spatial_pruning` iterava su **tutti** i row group e chiamava
`with_row_groups`, **sovrascrivendo** la selezione del pruning numerico. Con un
predicato e un hint spaziale insieme, il pruning numerico andava perso.

Non è di correttezza — l'over-return è dichiarato ammesso e le righe restano
giuste — ma è ottimizzazione persa. Nessun test lo copriva perché i due pruning
erano verificati separatamente. La composizione è stata **lasciata identica**:
correggerla è una decisione, non un effetto collaterale di FZ-0.1.

## Perimetro e rischi residui

Toccati: `driver-geoparquet` (prevalidazione, refactor dei due pruning),
`fuzz/quarantine.txt`, gate nuovo, CI, documentazione. Nessuna dipendenza
modificata, nessun contratto ristretto, nessun profilo di fuzzing toccato.

Residui dichiarati:

* il calcolo dell'inizio dei valori in una data page V1 duplica la logica di
  layout della pagina. È dieci righe e rifiuta le codifiche che non riconosce,
  ma resta allineata alla **versione pinnata** di `parquet`: un aggiornamento va
  accompagnato da una rilettura, come per la validazione IPC;
* il difetto a monte non è chiuso. La segnalazione e' **pubblicata** (apache/arrow-rs#10722) e registrata in
  [`UPSTREAM_PARQUET_DICTIONARY_BIT_WIDTH.md`](UPSTREAM_PARQUET_DICTIONARY_BIT_WIDTH.md);
  una correzione a monte non sostituirebbe la prevalidazione, come non la
  sostituisce la barriera;
* il costo su file compressi a dizionario è reale e misurato. Se un profilo
  d'uso lo rendesse inaccettabile, la leva è ratificare un'eccezione, non
  rimuovere la verifica in silenzio.
