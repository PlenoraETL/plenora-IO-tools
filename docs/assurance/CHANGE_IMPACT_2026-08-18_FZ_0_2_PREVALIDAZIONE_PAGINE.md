# Change impact analysis — prevalidazione degli header di pagina Parquet

Data: 2026-08-18. Sigla: **FZ-0.2**.
Baseline: `4592a0b` (INFRA-1).
Finding: `fuzz-findings/2026-08-18-parquet-uncompressed-page-size/` — **non tracciato**,
come tutta quella cartella (`.gitignore:16`). Cio' che deve sopravvivere sta qui e
in [`UPSTREAM_PARQUET_PAGE_ALLOCATION.md`](UPSTREAM_PARQUET_PAGE_ALLOCATION.md),
che porta il reproducer per intero.

## Problema

FZ-0.1 aveva chiuso il panico sul bit width degli indici di dizionario e
lasciato aperto un residuo: `SerializedPageReader` alloca con
`Vec::with_capacity(uncompressed_page_size)` leggendo il valore **dall'header
di pagina**, mentre il nostro tetto di prevalidazione guarda
`ColumnChunkMetaData::uncompressed_size()`, che è un altro numero. Fra i due non
c'è nessun legame.

Il residuo era registrato ma **non dimostrato**: era stato dedotto dal codice.
FZ-0.2 lo misura.

## Cosa è stato misurato

### Il seme

Il seme si costruisce da un GeoParquet valido modificando **un solo campo**, a
lunghezza netta invariata (script completo nel documento upstream):

```
uncompressed_page_size   10 -> 2 000 000 000   (varint 1 byte -> 5 byte)
compressed_page_size     12 -> 8               (compensa i 4 byte)
payload della pagina     -4 byte
```

Due dettagli che sembrano di forma e non lo sono:

* **L'invarianza di lunghezza.** Il primo tentativo inseriva i 4 byte. I byte in
  più sfalsavano il residuo del chunk, e `verify_page_size`
  (`serialized_reader.rs:900`) rifiutava la pagina *prima* di allocare. Un
  errore c'era, ed era pure tipizzato: preso per buono avrebbe chiuso il
  finding senza averlo dimostrato. È l'errore sbagliato al posto giusto — la
  forma di falso verde più difficile da vedere.
* **La pagina bersaglio.** Le prime versioni prendevano l'ultima pagina del
  file. Sono le quattro colonne bbox del covering, che il contratto nasconde e
  il lettore non tocca: modificarle non produceva niente. Il bersaglio è
  l'ultima pagina della **colonna letta**.

La catena delle pagine si percorre da offset 4 (`successiva = fine header +
compressed_page_size`) invece di cercare un byte che «somiglia» a un header:
cercare trovava un falso positivo in mezzo ai dati, con
`uncompressed_page_size = 3`, che avrebbe prodotto un seme senza senso. La
catena si autoverifica — finisce esattamente dove comincia la regione di
ColumnIndex/OffsetIndex — quindi un errore di lettura si vede prima di
modificare un byte, invece che dopo.

Versionato: `fuzz/seeds/geoparquet_reader/pagina-oltre-il-chunk.parquet`,
3.609 byte, SHA-256 `91f3e9c1…4347e`, provenienza in `fuzz/seeds/README.md`.

### Il comportamento, in sottoprocesso bounded

| `RLIMIT_AS` | File | Esito |
|---|---|---|
| 512 MiB | integro | exit 0 |
| 512 MiB | seme | `memory allocation of 2000000000 bytes failed`, **exit 134 (SIGABRT)** |
| 4 GiB | seme | allocazione riuscita, snappy fallisce, errore tipizzato |

**Il fatto che la registrazione iniziale non aveva: non è un panico, è un
abort.** `Vec::with_capacity` che fallisce chiama l'alloc error handler, che
termina il processo senza unwinding. Nessun `catch_unwind` lo vede — quindi la
barriera arrow non lo converte in errore tipizzato — e nessun test in-process lo
sopravvive. È la ragione per cui la dimostrazione gira in sottoprocesso, e la
ragione per cui il rischio era peggiore di come era stato scritto.

L'esposizione riguardava **il lettore quanto la nostra prevalidazione**:
`valida_bit_width_dizionario` legge le pagine con `get_next_page`, e non era al
riparo per il fatto di essere prevalidazione.

## Cosa cambia

### Il punto di controllo esiste, e non è replicare il decoder

L'API pubblica di `parquet` non lo offre: `PageMetadata` — l'unica cosa che
`peek_next_page` restituisce — porta `num_rows`, `num_levels` e `is_dict` ma non
le dimensioni, e `parquet::file::metadata::thrift`, dove vive `PageHeader`, è
`pub(crate)` in 59.1.0. Entrambi verificati sul sorgente, non dedotti.

Ma il file è nostro. `driver-geoparquet::pagine` legge gli header con un lettore
Thrift compatto minimo: tre campi `i32` di una struct, e il salto tipizzato dei
campi che non servono. Il decoder — dizionari, livelli, encoding — resta di
`parquet`.

Proprietà che rendono il rimedio non un secondo problema:

* **finestra fissa** di 64 KiB per header, che non dipende da niente che il file
  dichiari: la difesa non introduce l'allocazione guidata dall'input che sta
  impedendo;
* **nessuna allocazione** proporzionale ai valori letti — si avanza un indice
  dentro una fetta;
* **ricorsione limitata** a 16 livelli: una struct che si annida all'infinito è
  un file ostile, non un file profondo;
* **avanzamento obbligatorio**: una pagina che dichiara passo zero è rifiutata,
  altrimenti il ciclo girerebbe per sempre.

### I due controlli

* `uncompressed_page_size` non maggiore di quanto il chunk dichiari in tutto.
  **È l'invariante del formato, non una quota nostra**: la somma delle pagine
  non compresse *è* il totale del chunk, quindi una sola pagina non può
  superarlo. Non rifiuta nessun file coerente;
* `uncompressed_page_size` dentro il **budget di ingresso dichiarato**
  (`max_input_bytes`, 256 MiB per difetto).

Il secondo è il controllo che conta, e la prima stesura non ce l'aveva: aveva
una costante da 1 GiB, che è **sopra** il banco su cui il difetto era stato
misurato. Un chunk da 800 MiB con una pagina da 700 MiB è perfettamente
coerente, passa il primo controllo, sta sotto un tetto da 1 GiB, e aborta lo
stesso su un processo che di memoria ne ha mezza. Una difesa il cui limite non
ha rapporto con la macchina su cui gira non è una difesa, è un numero.

Il budget non è scelto qui: è quello che il chiamante ha **dichiarato** di
potersi permettere, preso dalle opzioni con cui il dataset è stato aperto — lo
stesso snapshot, non riletto a ogni `open_layer_reader`, così due letture sullo
stesso handle non possono usare quote diverse.

Insieme rendono il tetto sul chunk una garanzia effettiva sull'allocazione. La
doc del tetto è stata riscritta di conseguenza: prima diceva — correttamente —
di non descriverlo come protezione; ora dice che lo è **con** la prevalidazione
di pagina, e che senza tornerebbe a non esserlo.

### L'inizio del chunk, e perché viene rifiutato invece che interpretato

In parquet 59.1.0 ogni consumatore — il lettore di pagine e quello arrow —
passa da `ColumnChunkMetaData::byte_range`, che sceglie `dictionary_page_offset`
quando c'è e `data_page_offset` altrimenti. Verificato sul sorgente: non esiste
un `min()` fra i due, e la nostra regola coincide.

Ma coincide **oggi**. `inizio_del_chunk` rifiuta ora esplicitamente un chunk che
dichiari la pagina di dizionario *dopo* le pagine dati: nessun chunk legittimo
lo fa — il dizionario precede sempre i dati che indicizza — e su quei file
l'accordo fra ciò che verifichiamo e ciò che il decoder legge smette di dipendere
dal fatto che la libreria continui a scegliere come noi.

### Il parser è bounded, e adesso lo è davvero

La prima stesura aveva tre buchi, tutti nella stessa direzione — una difesa
contro l'esaurimento di risorse che si lasciava esaurire:

| Buco | Conseguenza | Chiusura |
|---|---|---|
| le lunghezze di `list`/`set`/`map` non erano confrontate con niente | un elenco che dichiara `u64::MAX` elementi manda il ciclo a girare finché il primo salto non fallisce: esito giusto, tempo arbitrario | il conteggio è confrontato con i **byte residui** prima di entrare nel ciclo — ogni elemento ne consuma almeno uno |
| i booleani non consumavano mai un byte | vero dentro una struct, **falso** dentro un elenco, dove ogni booleano è un byte a sé: un elenco di booleani veniva letto senza avanzare, e da lì ogni offset era sbagliato | `Posizione::Campo` e `Posizione::Elemento` sono due casi distinti |
| il varint accettava qualunque terminazione | un numero che non entra in `u64` veniva troncato in silenzio, dando un valore diverso da quello che il decoder leggerà | il decimo gruppo può portare un bit solo; oltre, è un errore |

E la catena delle pagine deve finire **esattamente** sulla fine del chunk, non
superarla: fermarsi a «l'abbiamo passata» lasciava passare un'ultima pagina che
sborda, e i byte oltre il chunk sono di un'altra colonna.

### L'ordine conta

`valida_dimensioni_pagine` gira **prima** di `valida_bit_width_dizionario`, che
usa `get_next_page` ed è esposta alla stessa allocazione. Cammina gli stessi
chunk — quelli che projection e pruning porteranno davvero al decoder, stesso
criterio e stesso snapshot dei metadati di FZ-0.1 — ma **tutti**, non solo
quelli a dizionario: l'allocazione della decompressione non dipende dalla
codifica.

## Verifica

### In sottoprocesso bounded, cinque casi

| | File | `RLIMIT_AS` | Quote | Esito |
|---|---|---|---|---|
| A | integro | 512 MiB | predefinite | exit 0 |
| B | pagina incoerente (2 GiB dentro trenta byte) | 512 MiB | predefinite | errore tipizzato, exit 2 |
| C | pagina **coerente** da 960 KB | 512 MiB | predefinite (tetto 256 MiB) | exit 0 — non si over-rifiuta |
| E | stessa pagina | 512 MiB | ingresso 100 KB | `byte di input oltre il limite` — è il **preflight** a fermarsi, non noi |

Prima del rimedio, il caso B sotto lo stesso limite produceva
`memory allocation of 2000000000 bytes failed`, exit 134 (SIGABRT). Le due
corse — prima e dopo, stesso seme, stesso limite, stesso comando — **sono** la
verifica per mutazione, nell'ordine giusto.

C ed E esistono per delimitare: senza C la difesa potrebbe rifiutare tutto e
sembrare corretta; senza E si potrebbe attribuire a questa prevalidazione un
rifiuto che viene da un altro controllo.

I casi che stringono la **memoria** non compaiono qui, e la ragione è una
lacuna che vale la pena nominare: **la CLI non espone `--memory-bytes`**, quindi
da riga di comando quella quota vale sempre il predefinito e non è
raggiungibile. Sono coperti dai test del driver, che le opzioni le costruiscono
direttamente — quattro casi, incluso quello in cui l'ingresso è quadruplicato e
il tetto non si muove.

### In repo

| Test | Cosa prova |
|---|---|
| `una_pagina_oltre_il_proprio_chunk_e_rifiutata_prima_dell_allocazione` | il seme versionato produce un errore tipizzato, `DataMapping`/`Read`, con messaggio statico, e non passa dal decoder |
| `una_pagina_coerente_ma_sopra_il_budget_e_rifiutata` | il caso che il solo controllo di coerenza non prende, end-to-end: il file **passa** il preflight, la pagina no |
| `un_chunk_con_il_dizionario_dopo_i_dati_e_rifiutato` | i tre casi di `inizio_del_chunk`, costruiti con il builder pubblico di `parquet` |
| `la_catena_delle_pagine_deve_chiudere_esatta` | chunk esatto, corto, lungo, e il budget che morde su una pagina coerente |
| `il_lettore_di_header_di_pagina_regge_gli_input_ostili` | header troncato, senza STOP, senza campo 3, tipo Thrift inesistente, varint infinito, varint che non entra in `u64`, struct annidate oltre la profondità, elenco che dichiara più elementi dei byte residui, booleani in lista |

Il verso «non rifiuta i file validi» **non** ha un test dedicato, e non serve:
ogni lettura di ogni test del driver passa ora dal nuovo controllo, e la suite è
verde. Un test in più direbbe meno di così.

Il test sul seme dichiara nel proprio commento che può provare **solo** il verso
positivo: l'abort non è osservabile in-process, e fingere di coprirlo sarebbe
peggio che dire che non lo si copre.

### Costo

| Caso | Righe | Byte | Row group | Prevalidazione | Lettura completa | Quota |
|---|---|---|---|---|---|---|
| piccolo | 1.000 | 33.202 | 1 | 10,0 µs | 179 µs | 5,6 % |
| medio | 50.000 | 143.357 | 1 | 24,7 µs | 2,26 ms | 1,1 % |
| grande | 400.000 | 1.115.913 | 7 | 226 µs | 18,0 ms | 1,3 % |

La quota alta del caso piccolo è costo fisso — una seek e una lettura per pagina
— su un file da 33 KB: in assoluto sono dieci microsecondi. La misura cronometra
la prevalidazione a parte dalla lettura, come in FZ-0.1: misurarla dentro il
drenaggio darebbe rumore invece di un numero.

### Il registro dei fallback resta fermo, e non è un caso

La prevalidazione avrebbe aggiunto due occorrenze al registro
`check_assurance_fallbacks`. Nessuna delle due è stata registrata.

`usize::try_from(fine - offset).unwrap_or(FINESTRA_HEADER)` era un **fallback
vero** nel senso di H-01: se il `try_from` fosse mai fallito avrebbe letto la
finestra intera oltre la fine del chunk, cioè byte di un'altra colonna, senza
dirlo. Chiuso prendendo il minimo in `u64` **prima** della conversione, che così
non può fallire.

`dictionary_page_offset().unwrap_or_else(|| data_page_offset())` non è un
fallback ma la regola del formato, ed era già registrata per la prevalidazione
Thrift. Invece di registrarla una terza volta è stata estratta in
`inizio_del_chunk`: tre siti che ripetevano una regola sola diventano uno.

Contando anche FZ-0.2.1, `driver-geoparquet` passa da 4 a **3** e il totale da
109 a 108. Ci si arriva **solo** guardando perché il numero si muoveva:
registrare le occorrenze sarebbe stato più veloce e avrebbe lasciato in piedi un
difetto e una duplicazione.

Due note di metodo, perché sono il tipo di errore che questo registro esiste per
prendere.

La prima stesura di `inizio_del_chunk` usava un `match` dove bastava
`unwrap_or_else`, e il contatore sarebbe sceso a parità di codice — il modo di
eludere H-01 che l'intestazione del registro denuncia, e non diventa lecito per
il fatto di essere involontario. Riscritta. (La forma finale usa `let … else`,
ma per un'altra ragione: la funzione ha un terzo esito, il rifiuto degli offset
invertiti, che nessun `unwrap_or_else` può esprimere.)

La seconda è più insidiosa. **A fine FZ-0.2 il conteggio era rimasto 4**, e
sembrava una conferma che nulla si fosse mosso. Non lo era: erano
`inizio_del_chunk` in calo e `i64::try_from(tetto).unwrap_or(i64::MAX)` in
aumento che si annullavano. Un contatore fermo non dice che niente si è mosso,
e la seconda occorrenza — un ripiego su un *tetto*, cioè un tetto che a volte
non c'è — è stata vista solo perché FZ-0.2.1 ha riguardato quella riga.

## FZ-0.2.1 — il tetto passa dalla quota di memoria

FZ-0.2 aveva legato il tetto per pagina a `max_input_bytes`. Funzionava e
chiudeva il caso misurato, ma confondeva due quote che il modello tiene
distinte apposta: `max_input_bytes` governa quanto è grande la **sorgente**,
mentre una pagina decompressa è **memoria temporanea**. L'effetto pratico era
che alzare il tetto sul file alzava anche quello sulla memoria — cioè rispondeva
a una domanda che chi lo alza non aveva fatto.

Il tetto è ora **metà della capacità di memoria effettiva**:

```rust
fn tetto_pagina(context: &PipelineContext) -> u64 {
    context.effective_memory_capacity() / 2
}
```

Tre scelte, tutte deliberate:

* **memoria e non ingresso**, per la ragione sopra;
* **capacità *effettiva*** — il minimo fra limite della pipeline e limite del
  pool — e non `PipelineLimits::memory_bytes`: con un pool più stretto, una
  soglia calcolata sul solo limite locale sarebbe irraggiungibile. È la stessa
  ragione per cui il modello espone `effective_memory_capacity`;
* **metà e non tutta**: la pagina decompressa non è sola in memoria — accanto ci
  sono la pagina compressa da cui viene, i buffer del decoder e gli array Arrow
  che ne escono. Concedere l'intera capacità a una sola allocazione
  significherebbe dichiararla l'unica.

Con i valori predefiniti il tetto resta **256 MiB**, metà dei 512 MiB
dichiarati: lo stesso numero di FZ-0.2, per una strada diversa.

### I quattro casi

| Caso | Esito |
|---|---|
| memoria stretta | rifiutato, con il messaggio della memoria |
| **ingresso quadruplicato, memoria invariata** | ancora rifiutato — l'ingresso non governa la memoria |
| memoria quadruplicata | la pagina entra |
| valori predefiniti | il file si legge come qualunque altro |

Più `il_tetto_per_pagina_segue_la_memoria_dichiarata`, che fissa la derivazione
numero per numero, incluso il caso che conta di più.

### Il confine di responsabilità, scritto

Dichiarare quattro gigabyte su una macchina che ne ha mezzo produce un tetto da
due gigabyte, e **la libreria non se ne accorge**. È un errore di deployment,
non qualcosa che questa funzione prometta di rilevare: leggere la memoria reale
del processo è instabile e non portabile, e una promessa del genere sarebbe
falsa su qualche piattaforma. Il test lo fissa come comportamento atteso invece
di lasciarlo come lacuna.

## Perimetro e rischi residui

Toccati: `crates/driver-geoparquet/src/pagine.rs` (nuovo),
`crates/driver-geoparquet/src/lib.rs`, `fuzz/seeds/`,
`fuzz-findings/2026-08-18-parquet-uncompressed-page-size/`,
`scripts/check_assurance_fallbacks.sh`, documentazione.

Non toccati: formati su disco, contratti pubblici, altri driver, `parquet`
(nessuna patch, nessun fork).

Residui dichiarati:

* **Il difetto a monte resta aperto.** Qualunque altro lettore costruito su
  `parquet` ha la stessa esposizione. Segnalazione **pubblicata** il 2026-08-18
  come apache/arrow-rs#10734, con l'account PlenoraETL indicato in
  autorizzazione; testo in
  [`UPSTREAM_PARQUET_PAGE_ALLOCATION.md`](UPSTREAM_PARQUET_PAGE_ALLOCATION.md).
* **La memoria dichiarata resta una dichiarazione, non una misura** (sopra).
  Chi dichiara più di quanto la macchina abbia torna esposto all'abort, con lo
  stesso meccanismo del caso B.
* **Un file molto comprimibile con poca memoria dichiarata viene rifiutato.** È
  il significato voluto della quota — una pagina che da sola chiede metà della
  memoria disponibile non è servibile — ma è un rifiuto che prima non c'era, ed
  è visibile al chiamante come errore tipizzato, non come degrado.
* **Copre le pagine, non tutte le allocazioni di `parquet`.** Page index e
  bloom filter hanno allocazioni proprie, guidate da altri campi. Non sono nel
  percorso che leggiamo oggi — non attiviamo `with_page_index` — ma se un giorno
  lo fossero, questa prevalidazione non li coprirebbe.
* **Il lettore Thrift è nostro.** Un formato che cambiasse la codifica degli
  header lo troverebbe disallineato. È l'unico modo di stare davanti
  all'allocazione con l'API pubblica di oggi, ed è il prezzo dichiarato.
