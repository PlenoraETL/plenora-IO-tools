# Change impact analysis — ratifica Lotto 0 e modello budget unificato

Data: 2026-08-16.

## Baseline e perimetro

Lo sviluppo parte da `main` `d52a8dd7a37239533b935875eca35d60df558819`.
La sorgente normativa e' `docs/DECISION-PACKAGE-Lotto-0.md`, ratificato in
pari data, e `docs/adr/ADR-IO-7-streaming-vs-operation-atomicity.md`,
promosso da Draft ad Accettato dallo stesso atto.

Questa CIA copre lo **step S0** (governance) e dichiara in anticipo
l'impatto del modello budget che gli step S1-S7 implementeranno, cosi' che
ogni PR successivo possa riferirsi a un'analisi gia' registrata invece di
riaprirla.

Il perimetro **incluso** e': i tipi budget di `plenora-io-model`, le
opzioni di lettura/scrittura di `plenora-io-core`, l'adapter comune di
lettura, i descrittori di formato e il modello d'errore interno.

Il perimetro **escluso**, esplicitamente e per tutti gli step del lotto,
e': i manifesti `release/*.json` (compreso un futuro `1.1.0.json`),
l'evidence base sotto `release/evidence/`, la facade Rust
`plenora-io-api`, i comandi CLI nuovi (`formats`, `options`, `schema`,
`validate`) e l'SDK Python. Nessuno di questi artefatti e' toccato da S0.

## Problema

La verifica statica del 2026-08-15 ha lasciato dieci finding residui nel
core. Cinque (L0.2, L0.3, L0.4, L0.9, L0.10) non sono correggibili
separatamente: descrivono lo stesso difetto strutturale visto da cinque
angoli. Il bordo ha **due** modelli di limiti (`Limits` e
`ResourceLimits`) con semantiche divergenti; un `convert` costruisce due
budget scollegati, quindi conta due volte le righe e perde
`output_expansion_ratio` sul ramo write; il budget di memoria e' consumato
in `commit` e mai restituito; il descriptor dichiara `Streaming*` mentre
l'adapter materializza l'intera sorgente; lo scan di directory non ha un
tetto sul numero di entry.

Correggerli uno alla volta avrebbe prodotto scelte locali incoerenti fra
loro: da qui un pacchetto decisionale congiunto invece di cinque PR
indipendenti.

## Decisione

Ratificato il modello a due livelli: un `PipelineContext` condiviso
(deadline, input osservato, memoria, spill, entry, cancellazione, pool
opzionale) e `OperationBudget` figli con contatori cumulativi
indipendenti. Gli invarianti normativi sono INV-1..INV-14 del pacchetto;
i piu' rilevanti per l'assurance:

- **INV-2/INV-13**: il budget si costruisce per una sola via. Il builder
  restituisce un `PipelineBundle` opaco che tiene insieme budget e
  `InputPermit`; il permit non e' costruibile ne' clonabile, e
  l'osservazione dell'input passa esclusivamente da
  `PipelineContext::observe_input(permit) ->
  Result<SourceFootprint, PlenoraIoError>`, che prende byte, entry e digest
  dallo stato accumulato invece che da parametri. Un'osservazione fabbricata o registrata
  su un context diverso non e' rappresentabile.
- **INV-3**: in `convert` reader e writer hanno contatori cumulativi
  distinti sotto lo stesso context. Il doppio conteggio di L0.10 non e'
  piu' esprimibile.
- **INV-5**: il gauge di memoria copre solo le allocazioni che la libreria
  detiene internamente; la lease e' rilasciata al transfer del batch. I
  tipi lease sono pubblici ma opachi e workspace-internal: non
  attraversano il bordo verso il consumer e non sono ri-esportati da
  alcuna facade.
- **INV-9**: `max_input_entries` diventa un limite di prima classe,
  applicato durante l'enumerazione e prima della somma dei byte.
- **INV-10**: il testo pubblico d'errore deriva da un enum tipizzato, non
  da `format!` libero; nessun hash del payload utente resta esposto.
- **INV-12**: senza `ResourcePool` i gauge memory/spill sono locali e la
  concorrenza **non esiste**; la libreria non promette di limitare
  pipeline concorrenti se il pool non c'e'.

ADR-IO 7 e' ratificato nell'**opzione A**: operation-atomicity conservata,
`VecDeque` sostituita da uno spool bounded. Le opzioni B e C restano
registrate come scartate.

## Impatto sull'hazard analysis

- **H-03 esaurimento risorse** (PLN-ASR-004, PLN-ASR-026): l'impatto e'
  positivo e sostanziale. Il picco di memoria dell'adapter comune passa da
  O(dataset) a `adaptive_memory_threshold + current_batch`, indipendente
  dalla dimensione totale dell'input; il doppio conteggio sparisce;
  `max_input_entries` chiude uno scan di directory oggi illimitato. La
  riga PLN-ASR-026 resta `Parziale` fino a S7: il modello nuovo convive
  con quello vecchio per tutta la migrazione.
- **H-01 valore inventato / perdita silenziosa**: nessuna regressione
  attesa. Il modello e' fail-closed sugli stessi assi di oggi e ne aggiunge
  uno (entry).
- **H-09 modifica non analizzata**: questa CIA e' il record; ogni step
  S1-S12 riferisce a essa e registra il proprio delta.

## Compatibilita'

- **Wire `cli-protocol-v1`**: invariato in S0 e per l'intero lotto, con
  una sola eccezione dichiarata: il **testo** del campo `message` della
  busta d'errore cambia in S9, mentre la **struttura** (campi, ordine,
  tipi, valori di enum) resta identica. I due invarianti sono verificati
  da suite separate.
- **Campo `read_mode`** di `catalog`: preservato byte-per-byte,
  driver-per-driver. I tre campi nuovi (`native_read_mode`,
  `effective_delivery`, `buffering`) sono additivi e compaiono solo a
  partire da S8; un consumer legacy li ignora.
- **API Rust**: i crate sono `publish = false` e la superficie e'
  dichiarata interna (PLN-ASR-024), quindi la sostituzione di
  `Limits`/`ResourceBudget` non e' una rottura pubblica. Resta una
  rottura per i consumer interni del workspace, gestita dalla migrazione
  M1-M4 con convivenza dei due modelli fino a S7.

## Verifica prevista

Ogni invariante ha una riga nella tabella "decisione → API concreta → test
di accettazione" del pacchetto, con i nomi dei test attesi. Il gate di
release verifica la presenza statica di quei nomi, non solo il verde.

Per S0 la verifica e' documentale e consiste in: ADR-IO 7 in stato
Accettato, pacchetto in stato ratificato, indice ADR di `Architetture.md`
e tabella di `docs/IMPLEMENTATION_STATUS.md` allineati, questa CIA
registrata, e i documenti di governance tracciati in git invece che
presenti solo nel working tree.

Per S1 la verifica e' il modulo `plenora-io-model::budget` nuovo, con la
sua suite dedicata, senza alcun cambio di comportamento del core.

## Hazard e residui

- Il modello vecchio e quello nuovo convivono da S1 a S7. Durante la
  finestra il rischio e' il **doppio conteggio fra i due percorsi**: e'
  mitigato in M2 dalla delega condivisa (il context ponte legge e scrive
  gli stessi contatori del `ResourceBudget` legacy per memory, spill e
  deadline) e verificato dal criterio di uscita di parita' dei limiti
  pre/post M2.
- `Rows`, `Columns`, `GeometryComponents`, `OutputBytes` e
  `ConcurrentOperations` restano applicati dal solo modello legacy fino a
  S4: nessun asse e' governato due volte, ma nemmeno uno solo dei cinque
  beneficia del modello nuovo prima di quello step.
- La revalidation della `SourceFootprint` fra `open` e `scan` e'
  **best-effort per costruzione** (size + mtime, piu' aggiunte/rimozioni
  di entry). Una mutazione concorrente che preservi size e mtime non e'
  rilevata. Nessuna variante forte (content hashing, file identity con
  handle stabile, locking) e' ratificata in questo lotto.
- Lo spool di S2 introduce una superficie I/O nuova (directory di spill,
  permessi, cleanup su lock esclusivo) gia' analizzata in ADR-IO 7 ma non
  ancora esercitata su una matrice di filesystem reali.
- La revisione indipendente resta non disponibile (PLN-ASR-012).

## Registrazione S1 (M1) del 2026-08-16

Introdotto `crates/plenora-io-model/src/budget.rs` accanto a `limits.rs` e
`resource.rs`, che restano invariati. Nessun crate consuma il modulo nuovo:
`plenora-io-core`, i driver e la CLI non cambiano di una riga, quindi il
comportamento osservabile del bordo e' identico a quello del commit base.

I tipi del modulo **non** sono ri-esportati alla radice del crate: durante la
convivenza dei due modelli un import dichiara sempre quale dei due sta
usando (`plenora_io_model::budget::PipelineLimits` contro
`plenora_io_model::Limits`).

**Default unificati**: dove i due modelli legacy divergevano — il finding
L0.2 — vince il valore piu' stretto, cosi' l'unificazione non allenta in
silenzio una quota gia' applicata: `max_rows` `10_000_000` e `max_columns`
`4_096` da `Limits` (contro `u64::MAX` e `65_536` di `ResourceLimits`),
`max_output_bytes` 1 GiB da `Limits` (contro `u64::MAX`). La scelta e'
osservabile solo da S4, quando i driver passano al modello nuovo.

**Verifica**: 41 test unitari nel modulo piu' 5 doctest, di cui 4
`compile_fail` che provano invarianti non dimostrabili a runtime —
`PipelineBundle` non destrutturabile (E0451), `InputPermit` non clonabile
(E0599), `SourceFootprint` non costruibile (E0639), trait delle parti sealed
(E0277). I quattro motivi di fallimento sono stati verificati uno per uno:
un `compile_fail` che fallisse per un errore di battitura passerebbe lo
stesso. Poiche' `cargo test --all-targets` **non** esegue i doctest, la CI
ha ora un passo dedicato: senza di esso quei gate esisterebbero nel sorgente
senza girare mai.

## Registrazione S1.1 del 2026-08-16

I due residui aperti da S1 sono chiusi con codice e test, non rinviati, e
il pacchetto e' stato corretto di conseguenza: nessuna errata contrattuale
resta pendente.

**`SourceDigest` implementato**. Il digest e' accumulato entry per entry
dentro il `PipelineContext` da `note_entry_visited(entry: &SourceEntry)`,
che ora porta path normalizzato, dimensione e mtime. La combinazione e' uno
XOR di FNV-1a a 64 bit applicato due volte con basi distinte:
insensibile all'ordine — l'ordine di enumerazione di una directory non e'
stabile e un digest che ne dipendesse segnalerebbe mutazioni inesistenti —
con la lunghezza del path in testa alla codifica per-entry, cosi' due
insiemi di path diversi non collassano sulla stessa sequenza di byte. Non
e' una funzione crittografica e non deve esserlo: chi puo' riscrivere i
file puo' comunque cambiarne il contenuto a dimensione e mtime invariati,
che e' il limite gia' dichiarato dalla garanzia best-effort. Evita inoltre
una dipendenza nuova, che nel workspace passa da un gate di pin.

Di conseguenza `observe_input` perde i parametri `entries`: entry e digest
vengono dal context che li ha osservati, non da argomenti che il chiamante
poteva fabbricare. Restava altrimenti una doppia sorgente di verita' sullo
stesso dato — ed era proprio il valore che la revalidation confronta.

**`max_vertices` portato nel modello unificato**. Non era una quota
astratta: `--max-vertices` e' un flag vivo della CLI e stringe il limite
di componenti per cella. `PipelineLimits` lo espone come quota e
`effective_wkb_components()` riproduce la composizione di
`Limits::effective_wkb()`. Un test confronta i due risultati direttamente,
cosi' la migrazione di S4 non puo' allentare il tetto senza rompere il
test.

**Default unificati vincolati da test**. `unified_defaults_are_never_looser_
than_either_legacy_model` confronta ogni quota con i default di `Limits` e
`ResourceLimits`: la regola "vince il piu' stretto" resta verificata contro
i modelli legacy finche' esistono, quindi una modifica dell'uno o
dell'altro non passa inosservata.

**Verifica S1.1**: 51 test unitari nel modulo (da 41) piu' i 5 doctest.

**Residuo che resta aperto e va chiuso nel lotto**:

- `ObservedInput` non distingue l'errore di osservazione dal caso "mai
  osservato": entrambi restano `NotObserved`. Un consumer non puo' sapere
  se un preflight ha fallito. La distinzione richiede uno stato tipizzato
  in piu' e va decisa insieme al modello d'errore strutturato di S9.

## Registrazione S1.2 del 2026-08-16

Tre finding aperti sulla superficie introdotta da S1, chiusi con codice e
test. Nessuno era una questione di stile: ognuno lasciava un modo di
osservare o produrre uno stato che il modello dichiara impossibile.

**L'osservazione diventa una state machine linearizzabile.** Conteggio
entry, byte addebitati e digest erano tre contatori indipendenti, due dei
quali atomici e uno protetto solo dal CAS dell'altro. Erano quindi
osservabili stati intermedi — entry gia' contata, byte non ancora sommati —
e un errore poteva lasciare un aggiornamento parziale. Ora vivono in
`Mutex<SourceObservation>` con due soli stati, `Collecting { entries,
total_bytes, digest }` e `Published(SourceFootprint)`: i controlli
precedono ogni scrittura, l'aggiornamento e' un atto unico, e la
pubblicazione e' terminale — dopo `observe_input` ogni entry nuova e'
rifiutata, perche' il footprint consegnato dichiara un insieme che non puo'
piu' cambiare.

**`observe_input` perde anche il parametro `bytes`.** Era l'ultima
grandezza del footprint che il chiamante poteva dichiarare senza averla
misurata, e per giunta quella che governa `output_expansion_ratio`: la
stessa classe di difetto di L0.10, spostata dentro il modello nuovo. Ora
`SourceEntry` distingue `metadata_size`, che entra nel digest, da
`charged_input_bytes`, che conta verso `max_input_bytes`, con costruzioni
separate `file()` e `directory()` perche' la regola "una directory addebita
zero byte" resti strutturale invece che una convenzione del chiamante.
`max_input_bytes` e' cosi' applicato dallo stesso atto che applica
`max_input_entries`.

**`OutputBytes` usa un unico loop CAS.** Proiettare il consumo con una
`load` e prelevare con una `compare_exchange` separata lascia una finestra
fra le due: due richieste concorrenti possono osservare lo stesso consumo,
superare entrambe il controllo del tetto derivato e prelevare entrambe,
sforandolo senza che nessuna lo veda. Proiezione e prelievo avvengono ora
sulla stessa osservazione atomica, e un test concorrente con otto thread
sotto un tetto derivato molto piu' stretto del limite assoluto verifica che
il totale concesso non lo superi mai.

**L'identita' di pipeline si alloca con incremento checked.** `fetch_add`
avvolge in silenzio: all'esaurimento dello spazio degli id due pipeline
riceverebbero lo stesso valore e il permit dell'una diventerebbe spendibile
sull'altra — il contrario esatto di cio' che l'identita' garantisce.
L'allocatore fallisce chiuso ed e' testato con il contatore a `u64::MAX - 1`.

**Verifica S1.2**: 58 test unitari nel modulo (da 51) piu' i 5 doctest.

## Registrazione S2 (M2) del 2026-08-16

`StagedSpool` sostituisce la `VecDeque` dell'adapter comune di lettura. La
garanzia resta la stessa — nessun prefisso accepted esposto se una violazione
emerge in un punto qualsiasi della sorgente — ma cambia dove stanno i batch
verificati: in RAM sotto una soglia adattiva, poi su file temporaneo in Arrow
IPC senza ritorno. Il picco di memoria passa da O(dataset) a
`adaptive_memory_threshold + batch corrente`.

**L0.3 chiuso.** La memoria dei batch bufferizzati non e' piu' consumata da
`commit`, che la sottraeva per sempre. Ogni batch in RAM tiene una lease
viva, restituita quando lascia la RAM: migrato su disco o consegnato al
consumer. La prenotazione di materializzazione viene rilasciata **prima**
che lo spool prenda la lease di residenza, altrimenti lo stesso batch
resterebbe contabilizzato due volte — una per la prenotazione larga
(target + cella) e una per l'occupazione reale — e una quota stretta
fallirebbe su un batch che ci sta.

**Errata: il ponte `delegating_to_legacy` non serve.** Il pacchetto lo
prevedeva perche' assumeva che lo `SpooledReader` fosse scritto contro il
modello nuovo mentre i driver stavano ancora sul vecchio. Lo spool e' invece
scritto contro il `ResourceBudget` legacy, quindi non c'e' nulla da
ponteggiare: in M2 un solo modello tocca i contatori, e il rischio di doppio
conteggio che il ponte doveva evitare non esiste. Il costo di migrazione e'
lo stesso — S4 riscrive le chiamate di budget dello spool insieme a tutto il
resto — senza un tipo transitorio di cui dimostrare la correttezza.

**Errata ADR-IO 7 sul file di spill.** Directory 0700 e sweep degli orfani su
lock sostituiti da `tempfile::tempfile_in`: file scollegato dal filesystem
appena aperto su Unix, `FILE_FLAG_DELETE_ON_CLOSE` su Windows. Nessun path da
aprire per un altro processo, nessun orfano da spazzare dopo un SIGKILL,
nessuna finestra TOCTOU, nessun symlink da seguire. `PLENORA_SPILL_DIR` resta
per scegliere il volume e fallisce chiuso se inutilizzabile.

**Difetto emerso durante l'implementazione, chiuso qui.** Un dataset di N
righe letto con quota esattamente N falliva: consegnato l'ultimo batch, il
giro successivo — fatto solo per scoprire la fine della sorgente — trovava
zero righe residue e trasformava l'EOF in `LimitExceeded`. Ora, quando una
quota e' esaurita, il reader prova comunque a leggere: se la sorgente e'
finita esce pulito, se produce ancora un batch l'errore resta quello di
prima. E' il comportamento che l'acceptance test
`convert_of_n_rows_with_max_rows_n_succeeds` richiede, e un secondo test
verifica che una riga oltre quota continui a fallire.

**Criterio di uscita "parita' dei limiti pre/post M2"**: verificato da
`limit_parity_pre_and_post_m2`, che esercita `Rows` alla soglia esatta e a
una sotto, `OutputBytes` come quota consumata e non trattenuta, e
`ConcurrentOperations` contato una volta sola e restituito al drop.

**INV-8 verificato sul ramo di replay**: un file di spool con un preambolo
valido seguito da spazzatura produce un errore tipizzato dopo aver consegnato
il primo batch integro — non un panico, e soprattutto non una fine
silenziosa che farebbe passare per completa una lettura troncata. E' il caso
in cui il consumer ha gia' ricevuto un `Ok`, quindi l'unico modo di
sbagliare in silenzio.

**Verifica S2**: 15 test dello spool piu' 5 nell'adapter
(`dataset_over_memory_bytes_succeeds_via_spool`,
`buffered_batches_do_not_permanently_consume_memory`,
`reader_of_n_rows_with_max_rows_n_succeeds`,
`reader_of_n_plus_one_rows_with_max_rows_n_still_fails`,
`limit_parity_pre_and_post_m2`).

**Veto prestazionale verificato** — vedi la registrazione del benchmark A/B
in fondo a questo documento.

## Registrazione S2.d del 2026-08-16

Cinque difetti della prima stesura dello spool, tutti chiusi con codice e
test. Nessuno era teorico: ognuno lasciava un modo di superare una quota o di
non rispondere a una cancellazione.

**La quota di spill segue i byte realmente scritti.** Addebitava la stima di
occupazione in RAM del batch, che non e' la stessa grandezza dei byte su
disco: l'IPC allinea, comprime i buffer di validita' e aggiunge intestazioni.
Un writer che conta i byte consegnati al file rende la quota una misura
dell'occupazione reale del volume. Ed era `commit`, cioe' consumo definitivo:
una pipeline lunga avrebbe esaurito lo spill accumulando quota di file gia'
rimossi. Ora e' una prenotazione RAII, rilasciata con lo spool. Le
prenotazioni avvengono a blocchi di 1 MiB perche' una lease per batch
significherebbe un milione di lease per un milione di batch; la copertura
precede sempre la scrittura, e se la stima risulta bassa la differenza viene
coperta subito invece di restare scoperta.

**Boundedness indipendente dai dati.** Un batch senza righe, o senza colonne,
veniva contato zero: la soglia non scattava mai e una sorgente che produce
batch vuoti in serie faceva crescere la coda senza tetto. La boundedness si
reggeva sull'ipotesi che ogni batch portasse dati, che e' esattamente cio'
che una sorgente ostile non fa. Ogni batch costa ora almeno
`PER_BATCH_OVERHEAD_BYTES`, e due test coprono il caso senza righe e quello
senza colonne.

**La sonda di EOF resta dentro quota.** La correzione di S2.b leggeva un
batch senza alcuna prenotazione per distinguere la fine della sorgente da una
violazione: se il driver avesse prodotto un batch grande, lo avrebbe
materializzato fuori budget — cioe' proprio cio' che il budget vieta. La
sonda avviene ora sotto una lease di memoria; senza memoria residua non si
sonda affatto e si fallisce chiuso, con un errore che dice quale quota manca.

**Cancellazione e deadline durante migrazione e replay.** Sono le due
sequenze lunghe dello spool. Senza controlli un Ctrl+C o una deadline scaduta
non avrebbero avuto effetto fino all'ultimo batch. Tre test coprono
cancellazione in migrazione, cancellazione in rilettura e deadline scaduta.

**Sezioni normative allineate in place.** ADR-IO 7 aveva ancora un "piano di
rollout proposto" che l'implementazione ha superato in tre punti: modulo
(`driver::spool`, non `publish`), file senza nome invece di directory 0700 con
sweep, quota RAII sui byte reali invece di `commit` sulla stima. Ora l'ADR
dichiara lo stato di attuazione e registra le divergenze come errata proprie,
invece di lasciarle solo nella CIA. Aggiornati anche il rustdoc di
`BudgetedReader` — diceva ancora "memoria O(dataset)" e rimandava a un ADR
non ancora ratificato — la riga ADR-IO 7 di `IMPLEMENTATION_STATUS.md`, la
sezione M2 del pacchetto e PLN-ASR-004 della matrice di tracciabilita'.

**Verifica S2.d**: 8 test nuovi nello spool e 1 nell'adapter, oltre
all'allineamento di 6 test le cui costanti dipendevano dalla vecchia
contabilita'.

## Registrazione S2.f del 2026-08-16

`release_storage` liberava esplicitamente le lease **prima** di assegnare
`Stage::Drained`, cioe' prima che il descrittore del file venisse chiuso.
L'ordine era invertito rispetto a quello sicuro: restituire la quota prima di
chiudere il file annuncia spazio che il volume non ha ancora liberato, e
un'altra operazione puo' prenderlo e trovarsi il disco pieno. L'errore aveva
per giunta l'aria di essere piu' accurato del codice corretto.

Ora la transizione e' una sola assegnazione: `self.stage = Stage::Drained`
distrugge il valore precedente, e i campi di `Stage` sono dichiarati
nell'ordine in cui devono sparire — prima writer o reader, che chiudono il
descrittore, poi il guardiano, che restituisce le lease. Il metodo
`SpillGuard::release` e' stato rimosso: non esiste piu' un modo di rilasciare
la quota fuori tempo.

**La garanzia e' verificata, non affermata.** L'ordine dipende dalla
dichiarazione dei campi, quindi e' fragile a un riordino distratto. Due test
lo fissano registrando gli eventi di distruzione: il file e' un newtype
`SpoolFile` e le lease sono avvolte in `TrackedLease`, cosi' il registro
segna il momento in cui la **quota torna al budget** e non quello in cui
muore il guardiano che la conteneva — le due cose coincidono solo se nessuno
svuota la lista prima del tempo, che e' esattamente l'errore da vedere.
Entrambi i test sono stati verificati per mutazione: reintroducendo il
rilascio anticipato, falliscono.

Il test sul percorso `clear` ha richiesto batch piu' grandi. Con pochi byte
il `BufWriter` non consegna nulla al file prima del drop, nessuna lease
esiste al momento di `clear`, e il test passerebbe comunque senza poter
distinguere l'ordine giusto da quello sbagliato — il modo peggiore di
fallire. Un'asserzione sui byte fisici scritti tiene ferma la precondizione.

**Residuo noto, assegnato a S4 come criterio obbligatorio**: fra il rilascio
della prenotazione di materializzazione e la lease di residenza presa dallo
spool esiste una finestra in cui il batch e' in RAM e non e' contabilizzato.
Con un budget condiviso — `convert` — un'altra operazione puo' infilarcisi.
Non e' chiudibile qui: servirebbe un trasferimento atomico che ridimensioni
la prenotazione senza restituirla al gauge, e il `ResourceLease` legacy non
sa ridimensionarsi. Il modello nuovo ha il punto giusto dove farlo, quindi il
criterio e' registrato in M3/S4 con il test che dovra' dimostrarlo.

## Registrazione S2.e del 2026-08-16

Quattro correzioni allo spool piu' la sostituzione delle sezioni normative
residue sullo sweep.

**L'enforcement della quota si sposta nel writer sottostante.** Prima la
prenotazione avveniva attorno alla scrittura del batch, cioe' su una stima:
i byte trattenuti dal `BufWriter` raggiungevano il volume senza passare da
alcun controllo, e una stima bassa lasciava crescere il file oltre la quota.
Ora il controllo vive in `GuardedWriter`, che avvolge il **file** — non il
buffer — ed e' quindi l'ultimo anello prima del disco: `buffer` contiene
esattamente i byte che stanno per essere consegnati. Il prezzo e' che il
rifiuto arriva al flush invece che al `push`; il guadagno e' che rifiuta cio'
che sta per essere scritto invece di una previsione. L'errore tipizzato viene
trasportato attraverso `io::Error` con un canale dedicato, perche' `Write`
non puo' restituire altro.

**Quote piu' piccole del blocco di prenotazione sono ora utilizzabili.** La
prenotazione a blocchi da 1 MiB serve a non creare una lease per batch, ma
faceva fallire sistematicamente ogni quota di spill inferiore al blocco: il
tetto configurato veniva di fatto arrotondato per eccesso, cioe' ignorato.
Ora, se il blocco non entra, si ripiega sull'importo esatto.

**File e quota si rilasciano a fine rilettura**, non al drop dello spool. Il
consumer puo' lavorare a lungo sui batch gia' ricevuti, e tenere occupati
volume e quota per tutto quel tempo non serve a nulla. Il passaggio a
`Drained` chiude il descrittore — quindi libera lo spazio, perche' l'inode e'
gia' scollegato — e restituisce le lease.

**Sezioni normative sullo sweep sostituite in place.** Il pacchetto
descriveva ancora directory `plenora-io-spool-*` con permessi 0700/DACL,
lock file per-directory, init sweep con ownership UID/SID e rimozione
ricorsiva symlink-safe, con cinque test dedicati. Nulla di tutto cio' esiste
piu': il file non ha nome, quindi non esistono orfani da spazzare e i casi
limite che quel meccanismo avrebbe dovuto gestire — PID recycling, clock
skew, filesystem senza `flock` affidabile, race fra due processi che
spazzano insieme — non si presentano. La sezione e' riscritta come "File di
spool: politica di sicurezza (attuata in S2)" e la riga ADR-IO 7 A della
tabella di accettazione elenca i test che esistono davvero.

**Verifica S2.e**: 3 test nuovi — sottostima della stima che non aggira la
quota, quota piu' piccola del blocco che resta utilizzabile, rilascio a fine
rilettura con lo spool ancora vivo — piu' 2 test allineati alla nuova
semantica di enforcement. 26 test nello spool.

## Benchmark A/B dello spool (S2, rieseguito dopo S2.e)

L'harness principale misura `read` e `write` separatamente e con i limiti di
default: con quelli lo spool non si attiva quasi mai, e un risultato verde
non direbbe nulla sul costo che interessa. Il binario
`plenora-bench-spool-ab` misura un `convert` completo — CSV → GeoParquet,
con **budget separati per lettura e scrittura** come fa `cmd_convert` della
CLI, perche' un budget condiviso conterebbe due volte la stessa riga e
misurerebbe un percorso che la CLI non esegue — in due varianti sullo stesso
fixture:

- **no-spill**: quota di memoria 1 GiB, i batch verificati restano in RAM;
- **forced-spill**: quota 8 MiB, soglia adattiva 4 MiB, la migrazione avviene.

Ogni corsa **dichiara se lo spill e' avvenuto davvero**, campionando la quota
residua di `SpillBytes` **durante** il drain: la prenotazione e' RAII e viene
restituita a fine rilettura, quindi una misura a posteriori vedrebbe zero. Un
`forced-spill` che non spilla esce con errore, perche' misurerebbe lo stesso
percorso del `no-spill` e sarebbe verde per costruzione.

La grandezza riportata e' `spill_peak_reserved_bytes`, cioe' la quota
**prenotata** al picco: non sono i byte fisici del file. La prenotazione
avviene a blocchi, quindi e' un limite superiore all'occupazione reale del
volume. I byte fisici li conosce solo lo spool e non sono osservabili dal
benchmark; il rapporto fra le due grandezze e' verificato dai test dello
spool, che asseriscono `scritti <= prenotati`.

Baseline **prima**: `601a124` — spool presente ma non cablato, l'adapter
accumula ancora in `VecDeque`. Baseline **dopo**: S2.e. 400.000 righe, 7
batch, con i due eseguibili invocati **alternati campione per campione nella
stessa campagna** — processi distinti, non due binari nello stesso processo —
cosi' una deriva del carico colpisce entrambi allo stesso modo.

**La statistica riportata e' il minimo, non la mediana, e la ragione va
detta**: durante la campagna la macchina era sotto carico crescente e i
campioni sono degradati da ~360 ms a oltre 2300 ms sullo stesso binario. Con
rumore additivo di quella entita' la mediana misura il carico della macchina,
non il codice; il minimo e' la stima piu' stabile del costo reale.

| Variante | Prima | Dopo | Delta |
|---|---|---|---|
| no-spill | 357 ms | 335 ms | **-6%** (nel rumore: nessuna regressione) |
| forced-spill | **non completa** `LimitExceeded` | 358 ms | +6,9% rispetto al no-spill |

**Percorso comune: nessuna regressione.** Il valore "dopo" e' persino piu'
basso del "prima", il che significa soltanto che il delta e' dentro il
rumore residuo — non che lo spool acceleri qualcosa.

**Percorso forced-spill: prima non completava affatto.** Con 8 MiB di quota
il codice pre-spool falliva `batch materializzato oltre la quota prenotata`,
che e' esattamente il difetto che ADR-IO 7 esiste per chiudere. Dopo, la
stessa conversione riesce a +6,9% rispetto al percorso senza spill: il costo
di scrittura e rilettura Arrow IPC non domina il tempo utente.

**Questa misura e' evidenza provvisoria, non evidenza di release.** Il file
temporaneo vive sul filesystem del container, veloce su questa macchina; su
un volume lento il rapporto scrittura/rilettura peserebbe di piu'. E la
campagna e' girata su una macchina non isolata, con carico crescente durante
l'esecuzione: il minimo di nove campioni alternati e' la stima piu'
difendibile ricavabile in quelle condizioni, ma resta una stima.

L'evidenza di release deve essere prodotta su **runner isolato**, con
statistica **paired/interlacciata** — coppie prima/dopo misurate adiacenti e
delta calcolato per coppia, invece di confrontare due aggregati raccolti in
momenti diversi. Finche' non esiste, il veto prestazionale di S2 e'
soddisfatto in via provvisoria.

## Registrazione S4.a del 2026-08-16 — handoff atomico della memoria

Primo sottopasso di S4, sul solo modello: il meccanismo che i sottopassi
successivi useranno per spostare la memoria dal materializzatore al custode
del batch.

**La forma scelta e' la riduzione, non il trasferimento.** Il materializzatore
prenota largo — target del batch piu' tetto per cella — perche' prima di
leggere non sa quanto occupera' davvero. A batch costruito la grandezza e'
nota: `InternalMemoryLease::shrink_to` porta la prenotazione a quella,
restituendo **solo l'eccedenza**. Poi la lease si sposta per `move` a chi
custodisce il batch, e un `move` non tocca il gauge.

Non serve quindi un'API di trasferimento fra due titolari: la combinazione
"riduci, poi sposta" e' senza finestra per costruzione, perche' la quota
contabilizzata scende da `RESERVED` a `ACTUAL` senza mai passare per zero.
Rilasciare e riacquistare, che e' cio' che il codice di S2 fa oggi, lascia
invece un istante in cui il batch e' in RAM e non lo conta nessuno.

`shrink_to` **rifiuta di ingrandire**: sarebbe una seconda prenotazione, che
puo' fallire, e lascerebbe il chiamante in uno stato ambiguo a meta' handoff.

**Il test concorrente osserva la fase, non la corsa.** Fuori dall'handoff e'
legittimo che risulti custodito un solo batch; durante l'handoff devono
risultarne due — quello gia' custodito e quello in transito. Un secondo
thread prova una prenotazione che entra **solo** se ne manca uno, e ogni
successo dentro la fase e' un'intrusione.

Costruirlo correttamente ha richiesto tre correzioni al test stesso, tutte
casi in cui avrebbe dato un verde privo di significato:

1. senza attendere l'avvio dell'osservatore, il ciclo poteva concludersi
   prima che venisse schedulato: il test passava senza aver guardato nulla;
2. rilasciando l'ultimo batch prima di fermare l'osservatore, una
   prenotazione perfettamente legittima veniva contata come intrusione e il
   test falliva su codice corretto;
3. la prima soglia scelta non era discriminante: durante la finestra resta
   comunque custodito un batch, quindi quella richiesta non poteva passare in
   nessuno dei due casi.

**Verifica per mutazione**: sostituendo la riduzione con rilascio e
riacquisizione, il test fallisce 5 volte su 5; con l'implementazione corretta
passa 8 volte su 8. L'osservatore si ferma alla prima intrusione, altrimenti
sottrarrebbe quota al thread principale rendendo la corsa lentissima senza
aggiungere informazione.

**Ridurre a zero e' rifiutato.** Un batch custodito occupa sempre almeno il
proprio ingombro strutturale — l'elemento in coda, l'`Arc` dello schema, i
metadati Arrow — anche senza righe ne' colonne. Una lease da zero byte
dichiarerebbe che un oggetto vivo non occupa nulla, cioe' rimetterebbe in
circolo la stessa finestra non contabilizzata che `shrink_to` esiste per
chiudere, solo scritta in un altro modo. Chi vuole smettere di
contabilizzare il batch rilascia la lease, e allora il batch non e' piu'
custodito.

**Il test conta le proprie osservazioni.** L'osservatore incrementa un
contatore ogni volta che prova dentro la fase, e il test asserisce che sia
maggiore di zero: senza, un verde potrebbe voler dire soltanto che nessuno
ha guardato. La copertura e' inoltre deterministica — il thread principale
apre la fase e **attende** che l'osservatore abbia provato almeno una volta
prima di procedere, invece di affidarsi allo scheduler.

Aggiunto anche `PipelineLimits::wkb_limits()`, che i driver useranno in
S4.c al posto di `Limits::effective_wkb()`.

## Registrazione S4.b del 2026-08-16 — payload transitorio e accessori centralizzati

Secondo sottopasso di S4, sul bordo driver. `ReadOptions` e `WriteOptions`
non espongono piu' i tre campi del modello legacy: al loro posto c'e' un
campo privato `payload` di tipo `BudgetPayload`.

**Perche' un enum e non tre `Option`.** Con i campi opzionali affiancati
esisterebbe la combinazione "budget nuovo presente **e** limiti vecchi
presenti", e nessun percorso saprebbe quale dei due lo governa. L'enum rende
quello stato inesprimibile: o `Legacy { limits, resource_budget,
cancellation }`, o `Pipeline { budget, permit, expected }`. `Default`
costruisce esclusivamente `Legacy`; `from_read_parts`/`from_write_parts`
costruiscono esclusivamente `Pipeline`; non esiste ripiego dal secondo al
primo.

**Gli accessori restituiscono scalari, mai un modello convertito.** Ogni
percorso consulta un solo payload, e la scelta di quale ramo risponda e'
centralizzata negli accessori invece di essere ripetuta nei driver. I valori
restituiti — `max_columns`, `max_rows`, `max_input_bytes`,
`max_input_entries`, `WkbLimits` — sono quote immutabili, non contatori:
leggerle dal ramo che governa non e' una conversione di budget. Non esiste, e
non deve esistere, un accessore che ricostruisca un `Limits` nel ramo
`Pipeline`; per questo `legacy_limits()` restituisce `Option<&Limits>` e non
un valore fabbricato. `WriteLimitsView` segue la stessa regola: tre scalari
copiati, nessun contatore, nessun budget duplicato.

**Il ponte fallisce, non ripiega.** Un driver ancora sul modello vecchio che
riceva opzioni `Pipeline` ottiene `bridge_richiede_legacy()` — un
`Unsupported` tipizzato — perche' un default silenzioso applicherebbe quote
che nessuno ha chiesto e renderebbe invisibile lo stato della migrazione.

**Permit, snapshot e budget passano per move.** `ReadBudgetParts::
into_components` e `WriteBudgetParts::into_budget` consumano le parti invece
di prestarne un riferimento da clonare. Clonare il budget sarebbe innocuo per
i contatori — condividono il `PipelineContext` — ma renderebbe
indistinguibile il passaggio dalla rigenerazione; e un permit rigenerato
porterebbe un pipeline id che il context rifiuta.

**I costruttori sostituiscono gli struct literal.** Con il payload privato la
forma `ReadOptions { .., ..Default::default() }` non e' piu' esprimibile. I
23 call site sono passati a `from_legacy` e ai builder pubblici
`with_assume_crs`, `with_format_options`, `with_format_option`,
`with_durable`. Sono API reale, non scorciatoie `cfg(test)`: nessun campo e'
stato riaperto e nessun percorso di test aggira il bordo pubblico.

**Un residuo emerso durante il lavoro.** `driver-filegdb` conteneva
`options.limits.max_output_bytes = 0` dentro `#[cfg(feature =
"gdal-backend")]`: fuori dalla build predefinita, quindi invisibile a
`cargo build --all-targets`. Lo ha trovato l'inventario, che censisce il
sorgente e non la build. E' migrato ai costruttori come tutti gli altri.

**L'inventario misurava il testo sbagliato.** Le regex della prima versione
contavano `opts.cancellation()` — l'accessore **nuovo** — nella categoria dei
campi legacy, e `pub struct ReadOptions {` fra i literal. Il numero sarebbe
salito mentre la migrazione procedeva. Le regex sono corrette; le cinque
categorie `campo_*` e `*_literal` vanno percio' a zero, non per merito del
conteggio ma perche' il payload privato rende quelle forme inesprimibili.
Da qui in avanti il ponte si misura con tre categorie aggiunte —
`costruttore_legacy` 13, `accessore_legacy` 50, `ponte_richiede_legacy` 50 —
che contano le uniche vie rimaste verso il modello vecchio e scendono a zero
in S4.e.

**Verifica per mutazione**, cinque mutazioni tutte uccise da test nominati:
far ricadere `max_columns` del ramo pipeline su `Limits::default()`; scartare
il permit in `from_read_parts`; restituire un token di cancellazione
rigenerato invece di quello del context; far leggere a `max_input_entries`
legacy il campo dei byte; azzerare il permit in `into_components`. La quinta
prima versione della terza mutazione non compilava, quindi non provava nulla:
e' stata riscritta in forma valida prima di accettarne l'esito.

**Non ancora fatto in S4.b**, e rinviato ai sottopassi che lo prevedono:
`StagedSpool::push` riceve ancora una `ResourceLease` del modello vecchio
invece della `InternalMemoryLease` gia' ridotta, quindi l'handoff di S4.a non
e' ancora cablato sul percorso reale. Il preflight ha la firma definitiva —
solo le due quote che consulta — ma la semantica e' invariata: enumerazione
via il modello unificato e rimozione atomica dei controlli legacy restano
S4.d, dove avvengono in un atto solo per non applicare le stesse quote due
volte.
