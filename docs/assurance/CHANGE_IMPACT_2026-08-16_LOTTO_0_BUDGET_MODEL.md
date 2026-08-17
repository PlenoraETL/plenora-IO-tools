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

## Registrazione S4.b.3 del 2026-08-16 — riconciliazione del confine del permit

Riconciliazione fra cio' che INV-13 dichiarava e cio' che il codice imponeva.
Nessun cambio di comportamento a runtime.

**INV-13 prometteva piu' di quanto Rust possa mantenere.** Diceva che il
permit non e' "mai separabile" dal proprio bundle. Ma `plenora-io-core` e' un
crate distinto da `plenora-io-model`, quindi l'API che gli consente di
prendere il permit deve essere `pub` — e `pub` significa raggiungibile da
chiunque aggiunga il modello fra le proprie dipendenze. Un `pub(workspace)`
non esiste. La promessa non era attuabile, e c'erano **due** vie pubbliche
per la stessa separazione: `ReadBudgetParts::take_input_permit()` e
`into_components()`.

La formulazione ora separa cio' che impone il linguaggio da cio' che impone
la convenzione. **Garantito dal tipo**: il permit non e' costruibile
dall'esterno, non e' `Clone`, ed e' legato al context che lo ha emesso — un
permit speso altrove e' un `Err`, non un'osservazione sbagliata. **Garantito
dalla convenzione e verificato**: e' separabile per move dentro il workspace,
attraverso un'unica API marcata `#[doc(hidden)]`.

Non e' una garanzia indebolita per rassegnazione. Una promessa di
impossibilita' che il compilatore non sostiene vale meno di un confine
convenzionale sorvegliato: la prima si legge come chiusa e non lo e', il
secondo dice cosa controlla e lo controlla davvero.

**Interventi**: `ReadBudgetParts::take_input_permit` rimosso; i 12 siti che lo
usavano — helper `observe` compreso, ora consumante — passati a
`into_components`. `into_components` e `WriteBudgetParts::into_budget` marcati
`#[doc(hidden)]`. `ReadOptions::take_input_permit` reso `pub(crate)`: l'unico
chiamante legittimo e' `Source::into_path_checked`, che vive in quel crate.
Nuovo gate `check_permit_boundary.py`, che verifica `publish = false` sui due
crate, la marcatura, l'assenza di un estrattore pubblico nel modello e
l'assenza di usi fuori da model/core.

Un effetto collaterale utile: `into_components` consuma le parti, quindi
l'unicita' del permit non e' piu' un `None` restituito alla seconda chiamata
ma un fatto del tipo. Il test che verificava il comportamento a runtime e'
stato sostituito da una nota, perche' non c'e' piu' nulla da verificare a
runtime.

**Il contratto d'errore di `observe_input` era sbagliato, in doc e nel
pacchetto.** Diceva "in ogni caso di errore lo stato resta `Collecting`,
quindi `NotObserved`". Falso nel caso del secondo publish: li' lo stato
precedente e' `Published`, l'errore lo lascia `Published`, e `ObservedInput`
continua a riportare il footprint registrato. La formulazione corretta e' che
**un errore non modifica lo stato precedente** — la chiamata o pubblica, o non
lascia traccia. Il test
`second_observation_is_rejected_and_keeps_the_published_footprint` gia'
verificava il comportamento reale: era la documentazione a divergere, non il
codice. Corretta anche la firma normativa nel pacchetto, rimasta
`observe_input(permit, bytes, entries)` mentre l'API reale prende il solo
permit dal 2026-08-16 (S1.2).

**"Costruibile" non e' "utilizzabile".** `from_read_parts` esiste dal commit
S4.b, ma adapter e spool prenotano ancora con `ResourceLease`. Un driver che
costruisse opzioni `Pipeline` oggi avrebbe i contatori di riga dal modello
nuovo e la memoria dei batch da quello vecchio, con la finestra non
contabilizzata aperta proprio sul percorso che `shrink_to` doveva chiudere.

L'handoff reale resta **prerequisito iniziale di S4.d** — trascinare la
riscrittura dello spool dentro una riconciliazione l'avrebbe resa
irrevisionabile — ma il vincolo e' attivo da subito e meccanico. Il gate
`check_pipeline_branch_gate.py` fallisce se un crate fuori da
`plenora-io-core` costruisce opzioni `Pipeline` mentre manca anche una sola
delle tre condizioni: `spool.rs` e `reader_adapters.rs` liberi da
`ResourceLease`, `InternalMemoryLease` effettivamente usato nel core, e il
test end-to-end `handoff_reale_della_memoria_senza_bridge_legacy`. Verificato
nei due versi: verde oggi, rosso appena si simula un driver che costruisce
`Pipeline`. **S4.c non chiude finche' il gate vincola**: i driver si
preparano, nessuno passa al ramo nuovo.

**Inventario invariato** — 261 usi, tutti i tetti confermati. S4.b.3 non tocca
il ponte legacy, quindi nessun valore doveva scendere e nessuno e' risalito.

## Registrazione S4.c del 2026-08-16 — un punto di decisione per direzione

Terzo sottopasso di S4, sui driver. **Nessun driver passa al ramo `Pipeline`**
— il gate di S4.b.3 lo vieta finche' l'handoff non e' cablato — ma dopo
questo commit nessuno di essi decide piu' quale modello governi i contatori.

**Il problema che risolve.** Prima di S4.c ogni driver scriveva da se'
`opts.legacy_budget().ok_or_else(bridge_richiede_legacy)?`: tredici copie
della stessa decisione, piu' undici copie del preflight inline. S4.d deve
capovolgere quella decisione **in un atto solo**, perche' il nuovo preflight
enumera la sorgente e i controlli vecchi devono sparire nello stesso istante,
altrimenti le stesse quote si applicano due volte. Ventiquattro punti da
cambiare insieme non e' un cambio atomico: e' una speranza.

**La forma.** Tre punti d'ingresso nel core prendono ora le **opzioni**
invece dei pezzi gia' estratti: `preflight_source(source, opts)` (nuovo),
`with_read_budget(dataset, opts, attestable)` e
`with_write_validation(writer, descriptor, plan, opts)`. Piu' tre accessori
neutri — `max_vertices()`, `ensure_active()` e `resource_budget()` — per i
percorsi su misura. I driver ricevono le opzioni e le passano; non le
interrogano sul modello.

**Risultato misurato**: `accessore_legacy` 50 → 18, `ponte_richiede_legacy`
50 → 11, e le 29 occorrenze residue vivono **tutte** dentro
`plenora-io-core`. Non e' piu' solo un conteggio: l'inventario ha una regola
strutturale nuova che rifiuta qualunque uso del ponte fuori da quel crate,
verificata in prova negativa.

**Un cambio di comportamento deliberato, non un effetto collaterale.**
`ReadOptions::ensure_active()` sul ramo legacy ora controlla **anche** la
cancellazione. `ResourceBudget::ensure_active` guarda solo la deadline: nel
modello vecchio la cancellazione vive in un secondo posto, il token che le
opzioni portano a parte, mentre nel modello nuovo sono la stessa cosa.
Lasciare la divergenza avrebbe dato allo stesso nome due significati, e i
cicli dei driver DXF e KML avrebbero iniziato a onorare la cancellazione **il
giorno del passaggio a `Pipeline`** — una modifica scoperta nel momento
peggiore. E' un controllo in piu' sullo stesso token che quei cicli gia'
interrogano altrove, quindi stringe senza sorprendere. Lo ha trovato un test
di parita' fra i rami, non una lettura del codice.

**Config privati tipizzati al posto di `Limits`.** `Walker::new` in DXF e
`infer_layout` in XLSX prendevano un `Limits` intero per consultarne due o
tre campi. Ora prendono `DxfQuote { colonne, righe, vertici }` e `XlsxQuote {
colonne, righe, byte_ingresso }`: i soli valori usati, in una struct privata,
senza tenere in vita un tipo che il modello unificato non ha.

**Due volte l'inventario ha colto un errore mio**, entrambe salite del
conteggio invece che discese. La prima: `DxfQuote::predefinite()` costruiva
le quote del fuzz harness da `ReadOptions::default()`, legando un harness e
due test unitari al default di produzione che la migrazione sta cambiando —
sostituito da costanti esplicite, che e' anche cio' che un harness di fuzzing
dovrebbe avere. La seconda: quindici copie della costruzione legacy nei test
nuovi, concentrate in tre helper. Un tetto che sale per il motivo sbagliato
smette di misurare la migrazione, ed e' il motivo per cui la regola "puo'
solo scendere" vale anche quando salire sarebbe comodo.

**Copertura**: sette test nuovi sui punti centralizzati — il preflight rifiuta
le opzioni del modello unificato e applica le quote su quelle legacy,
`with_read_budget` idem, `resource_budget()` fallisce tipizzato, e i tre
accessori neutri danno lo stesso valore nei due rami.

## Registrazione S4.d parte 0 del 2026-08-16 — ownership delle opzioni e gate irrobustiti

Tre chiusure preliminari prima dell'handoff. Nessun cambio di comportamento a
runtime: cambia la forma delle firme e il rigore dei gate.

**`FormatDriver::open` consuma le opzioni per valore.** Il pacchetto ratificato
lo diceva gia'; l'attuazione era rimasta a `&ReadOptions`. In quella forma il
preflight **non puo'** estrarre il permit, perche' `take_input_permit(&mut
self)` non e' chiamabile attraverso un riferimento condiviso — e senza estrarlo
non si osserva l'input, cioe' S4.d sarebbe stato impossibile senza toccare di
nuovo tredici firme.

Le due vie che avrebbero conservato `&ReadOptions` sono state escluse per la
stessa ragione: un `Mutex<Option<InputPermit>>` metterebbe uno stato mutabile
dietro una firma immutabile, e un permit clonato consentirebbe due
osservazioni dello stesso input. Sono entrambe modi di riscrivere la
proprieta' one-shot come convenzione, dopo che il tipo l'aveva resa un fatto.

`preflight_source` prende ora `&mut ReadOptions`. Consumare il permit non
consuma le opzioni: l'adapter le legge dopo. Il consumo effettivo resta S4.d.

Interventi: firma del tratto, dieci `impl open`, cento call site, e i punti in
cui `opts` posseduto va ripreso a prestito da chi si aspetta `&ReadOptions`.
Copertura: `il_preflight_consuma_il_permit_una_volta_e_lascia_le_opzioni_usabili`
verifica proprio la proprieta' richiesta — permit estraibile attraverso il
prestito mutabile, una sola volta, opzioni ancora leggibili dopo.

**Nota sulla verifica per mutazione.** Provando a rompere la proprieta'
one-shot si scopre che **non e' esprimibile**: restituire un secondo permit
richiederebbe clonarlo o costruirlo, e il tipo non consente ne' l'uno ne'
l'altro. E' un risultato piu' forte di una mutazione uccisa — la garanzia sta
nel tipo, non nella copertura. La mutazione che invece e' esprimibile — non
restituire mai il permit — e' uccisa da due test nominati.

**Il perimetro dei gate testuali era troppo stretto.** Guardavano solo
`crates/*/src/**`: un test d'integrazione in `tests/`, un benchmark in
`benches/`, un `examples/` o un `build.rs` potevano attraversare il confine
del permit senza che nulla lo vedesse — e sono proprio i posti dove si scrive
codice di servizio con meno attenzione. Riconoscevano inoltre la sola forma a
metodo, mentre `ReadBudgetParts::into_components(parts)` in UFCS fa la stessa
cosa, e un riferimento a funzione senza chiamata pure. Cercavano infine
`publish = false` come testo, che una riga commentata avrebbe soddisfatto.

Ora il perimetro comprende ogni `.rs` di ogni crate piu' `fuzz/`, le forme
riconosciute includono UFCS e il riferimento senza chiamata, e il manifesto e'
letto con `tomllib` — gestendo anche il caso `publish = ["registry"]`, che non
e' `false`. Verificato in quattro prove negative: UFCS in `tests/`, metodo in
`benches/`, `InputPermit` in `build.rs`, `publish` commentato.

**Il gate dell'handoff si accontentava di una menzione.** Il nome del test
bastava trovarlo in un punto qualsiasi del crate: i commenti scritti per
spiegare cosa mancasse lo avrebbero sbloccato. Chiedeva inoltre
`InternalMemoryLease` "da qualche parte nel core", che un `use` inutilizzato
avrebbe soddisfatto.

Ora le condizioni sono ancorate ai due file che devono davvero cambiare —
`spool.rs` e `reader_adapters.rs` liberi da `ResourceLease`, entrambi con
`InternalMemoryLease` e `shrink_to` — e il test e' cercato come **definizione**
`#[test] fn`, non come stringa. Verificato nei tre stati: incompleto oggi,
ancora incompleto con il test solo citato in un commento, completo con una
vera `#[test] fn`.

**Residuo documentale chiuso**: lo snippet normativo dichiarava ancora
`pub fn take_input_permit`, ed e' ora `pub(crate) fn`; l'introduzione di
`check_permit_boundary.py` diceva che INV-13 dichiara il permit non
separabile, mentre lo dichiarava la formulazione **originaria**, corretta in
S4.b.3.

## Registrazione S4.d del 2026-08-16 — handoff reale e preflight osservante

Quarto sottopasso di S4, atomico per necessita': il consumo del permit, la
rimozione dei controlli legacy e la migrazione del percorso comune sono lo
stesso cambiamento visto da tre lati, e separarli avrebbe applicato le stesse
quote due volte.

**Il preflight osserva davvero.** `Source::into_path_observed` sostituisce
`into_path_checked`: enumera la sorgente chiamando `note_entry_visited` una
volta per voce **scoperta**, e quella singola chiamata applica insieme
`max_input_entries`, i byte addebitati e il digest dell'identita'. Erano tre
controlli separati scritti a mano; separarli rendeva osservabile uno stato
intermedio e possibile un aggiornamento parziale. A enumerazione conclusa il
permit viene speso in `observe_input`, che pubblica il footprint.

I controlli vecchi non sono stati spostati: sono spariti **nello stesso atto**
in cui `note_entry_visited` ha iniziato ad applicarli. Lasciarli avrebbe
applicato due volte le stesse quote, la seconda contro contatori che la prima
aveva gia' consumato, e un input al limite sarebbe stato rifiutato per una
quota che in realta' bastava.

**Il percorso comune vive sul modello unificato.** Adapter e `StagedSpool`
prendono un `OperationBudget`: i contatori cumulativi dai suoi gauge, memoria
e spill dal `PipelineContext`. `with_read_budget` ha invertito la propria
guardia — accetta solo il modello nuovo — perche' la memoria dei batch e' una
`InternalMemoryLease`, che senza contesto non esiste.

**L'handoff.** L'adapter prenota largo (target del batch piu' tetto per
cella), misura il batch, riduce la prenotazione con `shrink_to` all'ingombro
reale piu' `PER_BATCH_OVERHEAD_BYTES`, e sposta **la stessa lease** nello
spool per `move`. Lo spool la custodisce e non ne acquisisce una seconda: la
grandezza contabilizzata e' quella della lease che arriva, non una misura
ricalcolata.

**Il test end-to-end e' costato tre correzioni**, tutte casi in cui avrebbe
dato un verde privo di significato:

1. l'osservatore girava anche durante la riconsegna, dove la memoria torna
   legittimamente al gauge: scambiava per intrusione il comportamento
   corretto. Ora sorveglia il solo drenaggio, e il reader segnala l'EOF;
2. la soglia era **fissa**. Ma l'occupazione da difendere cresce con i batch
   custoditi: dal secondo in poi una soglia fissa e' sempre superata, e il
   test tornava verde anche con il rilascia-e-riacquista. Ora la soglia e'
   `capacita - k * accounted + 1`, con `k` i batch consegnati;
3. con sei batch la finestra veniva colta due volte su cinque — non
   abbastanza per essere evidenza. Con quattrocento le occasioni sono due
   ordini di grandezza in piu'.

**Verifica per mutazione**: sostituendo `shrink_to` + `move` con
rilascia-e-riacquista, il test fallisce **5 volte su 5**; con
l'implementazione corretta passa **10 volte su 10**.

**Un difetto trovato dai test, non dalla lettura.** Il nuovo preflight
sondava i metadata della radice prima di controllare la cancellazione: una
pipeline gia' cancellata leggeva comunque il filesystem. Lo ha colto
`cancelled_source_is_rejected_before_filesystem_probe`, che esiste proprio
per quello.

**Una differenza di modello emersa in corsa.** `OperationBudget::remaining`
riporta il solo contatore cumulativo, mentre `try_lease` applica anche il
tetto derivato dall'input osservato (`output_expansion_ratio`). Nel modello
legacy l'osservazione restringeva direttamente il contatore, quindi la
differenza non si vedeva; qui il tetto e' una proiezione calcolata a ogni
lease. L'adapter prenotava sulla base del solo contatore e un round-trip CSV
di pochi byte falliva. La composizione ora e' esplicita in
`output_disponibile`.

**Lo spill dei driver non e' piu' consumo definitivo.** DXF, KML e XLSX
facevano `commit` sulla quota di spill: non tornava mai, nemmeno dopo la
rimozione del file. Ora tengono le `SpillLease` vive quanto il file
temporaneo, che e' la semantica RAII del modello nuovo.

**La concorrenza vive nel pool.** `ConcurrentOperations` non esiste in
`PipelineLimits` (INV-12): senza pool la lease e' un no-op. I due test che
verificavano il tetto sono stati riallineati con un `ResourcePool` esplicito;
non e' un indebolimento ma la semantica ratificata.

**Inventario**: `read_options_default` 74 → 1, `costruttore_legacy` 11 → 6,
`accessore_legacy` 18 → 16. **`ponte_richiede_legacy` e' salito, 11 → 15**, ed
e' la prima eccezione registrata alla regola "puo' solo scendere": quella
categoria conta la *guardia*, non il debito, e da S4.d la guardia protegge
nella direzione opposta — ogni punto che diceva "non so leggere il nuovo" ora
dice "non accetto il vecchio". Il tetto e' stato alzato con la motivazione
scritta nello script: alzarlo in silenzio sarebbe stato il modo di far passare
inosservata una crescita.

**Registro dei fallback** 99 → 102, con tre occorrenze nuove in
plenora-io-core e quattro evitabili rimosse. Nessuna richiede H-01: una e' una
conversione saturante fail-closed, due sono in moduli di test.

## Registrazione S4.d.1 del 2026-08-16 — follow-up di S4.d

Cinque correzioni su S4.d, due delle quali su difetti reali dell'handoff.

**L'ingombro strutturale non era sempre coperto (HIGH).** La prenotazione di
materializzazione valeva `target_bytes + max_wkb_cell_bytes`, ma l'ingombro
contabilizzato ceduto allo spool e' `bytes + PER_BATCH_OVERHEAD_BYTES`. Con un
tetto per cella e un target piccoli — bastava che la loro somma stesse sotto
l'overhead — il secondo superava la prima, e allo spool arrivava una lease
**piu' piccola** del batch che doveva coprire. `shrink_to` riduce e basta, e
il ramo che lo chiama scattava solo nel verso opposto: nessuno se ne
accorgeva, e la contabilita' diceva meno di quanto la libreria occupava.

Ora l'overhead entra nella prenotazione di **memoria** — e solo in quella. La
prenotazione di output resta separata, perche' `PER_BATCH_OVERHEAD_BYTES` e'
occupazione interna della libreria, non byte prodotti: sommarlo anche li'
avrebbe consumato quota di uscita che nessuno scrive. Prima della cessione
c'e' inoltre un controllo esplicito `accounted <= memory_lease.bytes()` che
fallisce chiuso: meglio fermarsi dove la causa e' visibile che custodire un
batch con una lease che non lo copre.

**Il pool non entrava nel dimensionamento (HIGH).**
`PipelineContext::remaining_memory()` riportava il solo gauge locale, mentre
`lease_memory_internal` compone locale e pool (INV-12). Con quota locale ampia
e pool stretto l'adapter chiedeva piu' di quanto entrasse, la lease falliva, e
il chiamante leggeva "memoria esaurita" dove c'era soltanto una richiesta mal
dimensionata — invece di prenotare il possibile e migrare su disco. La stessa
asimmetria colpiva `adaptive_memory_threshold`, derivata dal solo
`PipelineLimits::memory_bytes`: con un pool piu' stretto la soglia era
irraggiungibile e lo spool non migrava mai, cioe' restava inutile proprio nel
caso che deve risolvere.

Il context espone ora `effective_remaining_memory`,
`effective_remaining_spill`, `effective_memory_capacity` e
`effective_spill_capacity`, tutti come minimo fra locale e pool. Gli accessori
locali restano, documentati per quello che sono.

**Identita' del percorso.** `to_string_lossy` sostituisce ogni sequenza non
valida con U+FFFD: su Unix `b"\xff"` e `b"\xfe"` diventano la **stessa**
stringa, quindi lo stesso digest, e il footprint direbbe che due sorgenti
distinte sono la stessa. Si usano ora i byte nativi dell'`OsStr` su Unix e le
unita' UTF-16 serializzate little-endian su Windows, con un prefisso che
distingue le due codifiche: la rappresentazione non e' leggibile, e non deve
esserlo — deve essere iniettiva e stabile.

**Cancellazione per voce.** `ensure_active` e `check_cancelled` erano
all'inizio di ogni directory, non di ogni voce. Una singola directory con
molte voci comporta altrettante `symlink_metadata`, e una cancellazione non
avrebbe avuto effetto fino alla fine di quella directory. Il controllo e' ora
in testa a `scopri`, quindi copre anche la radice: i due controlli che lo
precedevano sono stati rimossi perche' ridondanti.

**Le guardie sono due, direzionali.** `bridge_richiede_legacy` copriva due
situazioni opposte con lo stesso messaggio: un percorso vecchio che rifiuta
opzioni nuove, e un percorso gia' migrato che rifiuta opzioni vecchie. Nel
secondo caso "componente non ancora migrato" era **falso**. Ora ci sono
`richiede_modello_legacy` e `richiede_modello_unificato`, con due conteggi
esatti nell'inventario: la prima misura il debito e deve scendere, la seconda
misura il progresso. Entrambe spariscono in S4.e.

**Rustdoc.** `into_path_observed` portava ancora, sopra la doc nuova, il
blocco pre-S4.d di `into_path_checked` — due descrizioni contraddittorie sulla
stessa funzione. In `spool.rs` la doc di `push` descriveva un parametro
`memory_bytes` che non esiste piu', e la stessa spiegazione dell'handoff
compariva tre volte fra doc di variante, doc di campo e commento nel corpo.

**Verifica per mutazione**, tre su tre uccise: togliere l'overhead dalla
prenotazione di memoria; dimensionare sul residuo locale invece che
sull'effettivo; derivare la soglia dal limite locale invece che dalla capacita'
effettiva. I due test nuovi coprono rispettivamente un tetto per cella sotto
l'overhead con memoria stretta, e un pool piu' stretto della pipeline che deve
spillare e completare.

## Registrazione S4.e del 2026-08-16 — il modello legacy non esiste piu'

Ultimo sottopasso di S4. Con questo commit esiste **un solo** modello di
budget, e la coesistenza che il Lotto 0 ha attraversato non e' piu'
rappresentabile.

**Cosa e' sparito.** La variante `BudgetPayload::Legacy` e l'enum stesso; i
`Default` di `ReadOptions` e `WriteOptions`; i costruttori `from_legacy` e gli
accessori `legacy_budget`, `legacy_limits`, `resource_budget`; entrambe le
guardie direzionali; `Limits`, `ResourceBudget`, `ResourceLease`,
`ResourceKind` e `ResourceLimits`, con l'intero `resource.rs`. In `limits.rs`
resta il solo `WkbLimits`, che e' sempre stato un tipo del contratto e non del
budget.

**Le opzioni non hanno `Default`, e non e' una dimenticanza.** Portano un
`OperationBudget`, che nasce da una costruzione che puo' fallire — limiti
incoerenti, deadline scaduta. Un `Default` avrebbe dovuto scegliere fra il
panico e quote inventate, ed era la seconda strada quella presa: costruiva un
ramo legacy con i valori storici che nessun chiamante aveva chiesto.

**Il ramo di scrittura e' entrato nella pipeline.** `with_write_validation` e
`with_write_limits` prendono le opzioni e ne usano il budget; il writer preleva
righe, output e componenti dai contatori dell'operazione, e la memoria dello
staging e' una `InternalMemoryLease` restituita al drop invece di un consumo
definitivo.

**`convert` usa un solo context.** Fino a S4.d la CLI costruiva due
`ResourceBudget` scollegati: risolveva il finding #3 — una riga non deve
consumare due volte la stessa quota — ma contava due volte memoria e spill, e
impediva al writer di vedere l'input osservato dal reader, che e' cio' da cui
`output_expansion_ratio` deriva il tetto (INV-6). Ora i due rami escono dallo
stesso `ConvertBudgetParts`: contatori indipendenti, context condiviso. Il test
verifica entrambe le meta'.

**I flag della CLI atterrano direttamente in `PipelineLimits`.** Il `Limits`
intermedio non serviva a nulla se non a tenere in vita il modello vecchio nel
punto piu' visibile del componente.

**`with_read_budget` non restituisce piu' `Result`.** Con un solo modello non
c'e' nulla da rifiutare, e una funzione che non puo' fallire non deve
dichiarare di poterlo fare.

**Test rimossi invece che riscritti.** Parita' fra i due rami, "il ramo
pipeline non consulta `Limits`", "il ramo legacy non tocca i gauge nuovi", le
guardie direzionali: senza un secondo modello non descrivono piu' nulla e
sarebbero passati per costruzione. Sono stati eliminati. Restano, riscritti sul
modello unico, quelli che dicono ancora qualcosa: gli scalari dai limiti della
pipeline, la vista di scrittura, il permit one-shot, il context condiviso.

I due test di parita' del modello — `effective_wkb_components` e i default
unificati — confrontavano il nuovo con le strutture legacy. Rimosse quelle, il
requisito resta: i default non devono allentarsi in silenzio, o il finding L0.2
si riaprirebbe. I valori sono ora **fissati esplicitamente**, con l'origine
scritta accanto a ciascuno.

**Inventario: zero, tutte le undici categorie.** Il gate
`check_legacy_budget_inventory.py` e' stato rimosso, come previsto fin dalla
sua introduzione, insieme a `check_pipeline_branch_gate.py`, che sorvegliava
una distinzione — "costruibile" contro "utilizzabile" — che non esiste piu'.

Non e' un indebolimento della sorveglianza: **ora e' il compilatore il gate**.
`ReadOptions::default()` non compila, `Limits` non esiste, un driver non puo'
nominare un budget legacy perche' il tipo non c'e'. Una regressione non e'
distratta: e' inesprimibile. Resta `check_permit_boundary.py`, che sorveglia un
confine che il linguaggio non impone.

**Registro dei fallback** 102 → 104: due occorrenze, la stessa conversione in
due moduli di test. Il tetto di colonne passa da `u64` a `usize` perche'
`validate_write` lo prende in `usize`; prima veniva da `Limits`, dove era gia'
`usize`.

### Tracciabilita' del delta dei test in S4.e: 483 → 471

Il conteggio scende di dodici. Non e' una perdita netta di copertura ma il
saldo fra **ventitre** test rimossi e **undici** aggiunti, e ogni rimozione ha
una ragione. La tabella la registra perche' una diminuzione senza spiegazione
e' indistinguibile da una copertura persa per distrazione.

Due categorie, e vanno tenute separate.

**Transitori (14).** Descrivevano la coesistenza dei due modelli. Senza un
secondo modello non hanno un'invariante da conservare: riscriverli avrebbe
prodotto asserzioni vere per costruzione.

| Test rimosso | Dove viveva |
|---|---|
| `payload_legacy_espone_il_ramo_legacy_e_nessun_budget_pipeline` | core/driver.rs |
| `payload_pipeline_non_lascia_leggere_il_ramo_legacy` | core/driver.rs |
| `il_ponte_legacy_fallisce_tipizzato_sul_payload_pipeline` | core/driver.rs |
| `il_ramo_legacy_non_muove_i_gauge_del_modello_nuovo` | core/driver.rs |
| `il_ramo_pipeline_serve_gli_scalari_senza_costruire_limits` | core/driver.rs |
| `i_due_rami_producono_gli_stessi_scalari_a_configurazione_equivalente` | core/driver.rs |
| `i_due_rami_producono_la_stessa_vista_di_scrittura` | core/driver.rs |
| `max_vertices_coincide_fra_i_due_rami` | core/driver.rs |
| `resource_budget_fallisce_tipizzato_sul_ramo_pipeline` | core/driver.rs |
| `il_preflight_rifiuta_le_opzioni_legacy` | core/driver.rs |
| `budget_identity_is_explicit` | model/resource.rs |
| `checked_commit_returns_only_unused_quota` | model/resource.rs |
| `deadline_expires_for_every_clone` | model/resource.rs |
| `leases_cross_clones_and_return_quota_on_drop` | model/resource.rs |

Le quattro di `resource.rs` verificavano il comportamento del budget legacy —
identita', commit parziale, deadline propagata ai cloni, lease che tornano al
drop. Il modello unificato ha le proprie, gia' presenti in `budget.rs`:
`operation_budget_clone_does_not_double_the_consumption`,
`counted_lease_commit_returns_only_unused_quota`,
`deadline_expiry_is_not_conflated_with_cancellation`,
`internal_memory_lease_returns_quota_on_drop` e
`spill_lease_returns_quota_on_drop`. Non sono state aggiunte ora: esistono da
S1, e i nomi sono stati verificati contro il sorgente invece che ricostruiti a
memoria — due dei quattro che avevo citato in prima stesura non esistevano.

**Con invariante conservata (9).** Il test e' sparito, la proprieta' no.

| Test rimosso | Chi ne conserva l'invariante |
|---|---|
| `unified_defaults_are_never_looser_than_either_legacy_model` | `unified_defaults_stay_at_the_tightest_historical_values` — stessa regola (finding L0.2), ma i valori attesi sono **fissati** con l'origine accanto invece di essere letti dalle strutture legacy. Scrivendolo si e' scoperto che quattro assert precedenti (memoria, spill, durata, rapporto di decompressione) confrontavano il modello con se' stesso e non sorvegliavano nulla |
| `global_vertex_limit_tightens_wkb_components` | `effective_wkb_components_is_tightened_by_max_vertices`, esteso al verso opposto: quando e' il tetto per cella a essere piu' stretto, vince lui |
| `resource_budget_riflette_i_flag_cli` | `i_flag_atterrano_nei_limiti_della_pipeline` — i flag ora atterrano direttamente in `PipelineLimits`, quindi il test parte da `parse()` invece che dal helper di traduzione, che non esiste piu' |
| `resource_budget_rifiuta_flag_a_zero` | `la_pipeline_rifiuta_flag_a_zero` |
| `resource_budget_non_deriva_geometry_components_dal_wkb_per_cella` | `geometry_components_non_deriva_dal_wkb_per_cella` — stessa asserzione, e in piu' verifica che il **per-cella** segua il flag, cosa che il test precedente non controllava |
| `conversion_budgets_hanno_contatori_indipendenti` | `i_due_rami_di_convert_hanno_contatori_indipendenti_e_context_condiviso` — conserva l'indipendenza dei contatori e **aggiunge** la condivisione del context, che con due budget scollegati non era verificabile |
| `with_read_budget_accetta_solo_il_modello_unificato` | `with_read_budget_collega_il_budget_dell_operazione` — non c'e' piu' un modello da rifiutare, quindi il test verifica il fatto positivo: che il reader consumi la quota di concorrenza del context collegato |
| `ensure_active_interroga_il_modello_che_governa_in_entrambi_i_rami` | `ensure_active_osserva_la_cancellazione_del_context` |
| `output_limit_uses_shared_observed_input` (model/resource.rs) | `convert_writer_sees_input_observed_by_reader` e `output_limit_applies_expansion_when_bytes_positive` in `budget.rs`, presenti da S1 |

**Aggiunti (11).** Oltre agli otto sostituti nominati sopra:
`gli_scalari_arrivano_dai_limiti_della_pipeline`,
`la_vista_di_scrittura_arriva_dagli_stessi_limiti` e
`i_limiti_wkb_predefiniti_sono_quelli_storici` — quest'ultimo perche', tolto
`Limits`, i default di `WkbLimits` restavano senza alcun test.

### Il gate permanente che sostituisce l'inventario

`check_no_legacy_budget.py` sostituisce l'inventario a soglie decrescenti,
rimosso con S4.e. Non e' lo stesso gate con un altro nome: l'inventario
misurava una migrazione in corso e i suoi numeri scendevano a ogni sottopasso,
questo dichiara uno stato raggiunto e non ammette gradazioni.

Vieta `resource.rs` e `mod resource`, i quattro tipi `Resource*`, il tipo
esatto `Limits` — con una lookbehind che risparmia `PipelineLimits` e
`WkbLimits` — `BudgetPayload`, i costruttori `from_legacy`, gli accessori
`legacy_*` e `resource_budget`, e il `Default` di `ReadOptions`/`WriteOptions`.
Scandisce ogni `.rs` di ogni crate piu' `fuzz/`, e spoglia commenti e stringhe:
un commento che spiega perche' un tipo e' stato rimosso ne nomina il nome, e
senza lo spoglio il gate vieterebbe di documentare la propria ragione.

**Ha trovato subito un residuo reale.** Il crate `fuzz` non entra nella build
predefinita — richiede nightly e `cargo-fuzz` — e il suo harness costruiva
ancora `Limits`, `ReadOptions { .. }` e `WriteOptions::default()`. Era rotto da
S4.e e nessun gate lo vedeva, perche' `cargo metadata` verifica il grafo delle
dipendenze e non la compilazione. L'harness e' migrato, e `convert` costruisce
ora i due rami dallo stesso `ConvertBudgetParts` come il codice spedito:
misurare una forma diversa da quella in produzione avrebbe reso la campagna
poco utile. **Resta non verificato per compilazione**: e' lo stesso gap dei
gate fuzz e coverage, assenti dal `Dockerfile.dev`.

## Registrazione S5 del 2026-08-16 — i limiti configurati arrivano all'inferenza

L0.1. Fino a S4 le passate di inferenza di CSV, GeoJSON e XLSX usavano
`WkbLimits::default()` — 64 MiB per cella — con un commento che rimandava al
lotto L6. `--max-wkb-cell-bytes` non le raggiungeva: chi stringeva il flag
otteneva il rifiuto piu' tardi, o non lo otteneva affatto, e l'AST wkt veniva
allocato comunque.

**Cosa riceve ora i limiti reali.** In CSV `infer_types` e
`infer_wkt_geometry`, piu' `append_geometry` sul percorso di lettura; in
GeoJSON `infer_schema` e la deserializzazione della geometria nelle due
passate; in XLSX `encode_geometry_cell`, che serve sia l'inferenza sia la
materializzazione.

**Nessun default indipendente, nessun contatore nuovo.** Ogni driver prende le
quote dalle opzioni tramite un config privato tipizzato — `QuoteInferenza` in
CSV e GeoJSON, il gia' esistente `XlsxQuote` — con i soli valori che consulta.
Passare i `PipelineLimits` interi darebbe all'inferenza accesso a quote che non
le competono, la memoria per esempio, che governa il batch e non il parsing di
una cella.

**Il limite precede l'allocazione.** Il tetto per cella e' applicato al testo
grezzo prima di costruire l'AST; il tetto sulle righe prima di leggere la
cella. Fermarsi dopo vorrebbe dire aver gia' allocato cio' che il limite
doveva impedire.

### Una deviazione dal perimetro, dichiarata

Il perimetro chiedeva un acceptance test `inference_respects_max_input_entries`.
**Non l'ho scritto con quel nome, e non ho applicato quella quota ai record.**

`max_input_entries` governa l'enumerazione della **sorgente**, e il preflight
l'ha gia' applicata al file: riapplicarla ai record sarebbe la stessa quota
contata due volte, che e' l'errore che l'intero Lotto 0 ha evitato. Il suo
valore predefinito — 10.000, calibrato sui file di una directory — ai record
rifiuterebbe un CSV di dimensioni ordinarie: il benchmark del repository ne
genera 200.000, e sarebbe stato rotto dal commit.

Il test si chiama percio' `inference_respects_max_rows_before_materialising` e
usa `max_rows`, che e' la quota che governa le righe. Che il tetto sulle entry
sia applicato **prima** dell'inferenza resta verificato dove quell'invariante
vive: `directory_scan_over_max_input_entries_rejects_with_typed_error` nel
preflight del core. Un driver non raggiunge l'inferenza se la sorgente ha gia'
sforato.

Se il nome era vincolante piu' della semantica, va detto: il cambio e' un
rename piu' l'accettazione che il default vada alzato.

### Verifica per mutazione

| Mutazione | Esito |
|---|---|
| inferenza WKT torna al default | **uccisa** da `inference_uses_configured_wkt_cell_bytes_not_default` |
| entrambi i tetti di riga rimossi | **uccisa** da `inference_respects_max_rows_before_materialising` |
| tetto di riga rimosso da `infer_types` soltanto | sopravvive |
| tetto di riga rimosso da `infer_wkt_geometry` soltanto | sopravvive |
| lettura torna al default | sopravvive |

Le tre sopravvivenze non sono copertura mancante, e vanno lette per quello che
sono. I due tetti di riga si coprono a vicenda: `infer_types` gira prima, quindi
rimuovere il tetto da una sola passata lascia l'altra a fermare il file. La
mutazione combinata e' uccisa, che e' la forma in cui la proprieta' e'
verificabile.

La lettura e' il caso piu' interessante. Inferenza e lettura parsano le stesse
celle con la stessa quota, presa dalle stesse opzioni: dall'API pubblica non
c'e' modo di stringere la seconda senza stringere la prima, quindi una cella
oltre soglia e' sempre rifiutata dall'inferenza e la lettura non la vede mai.
E' ridondanza fra due controlli sullo stesso dato, non un buco. Il test
`il_percorso_completo_gira_sotto_un_tetto_non_predefinito` copre cio' che
resta dimostrabile: che il percorso completo funzioni con un tetto configurato,
cioe' che la lettura non usi un valore **incoerente** con l'inferenza — un file
accettato all'apertura che fallisce a meta' drenaggio sarebbe il difetto
peggiore, e quello e' escluso.

Scrivendo quel test ho tarato la soglia sul solo testo WKT e il test e'
fallito: il tetto governa **due** grandezze sullo stesso percorso — i byte del
testo in inferenza e quelli del WKB codificato nella validazione del batch — e
un punto occupa 11 byte in testo e 21 in WKB.

### Censimento dei `WkbLimits::default()` residui

Nuovo gate permanente `check_wkb_limits_defaults.py`. Non vieta il simbolo —
sarebbe sbagliato, alcune occorrenze sono corrette — ma le classifica e fissa
il conteggio per categoria, cosi' che una nuova non passi senza che qualcuno la
collochi.

| Categoria | Conteggio | Natura |
|---|---|---|
| test | 45 | `decode_wkb` su un WKB prodotto dal test stesso: il tetto non governa nulla |
| attrezzaggio | 4 | `plenora-bench` e `plenora-fuzz`, non codice spedito |
| produzione, legittimi | 2 | `__fuzz_gpkg_geometry` e `__fuzz_wkb_roundtrip`: entry point per libFuzzer, input gia' bounded a 1 MiB dall'harness, e nessuna opzione da cui prendere una quota |
| produzione, **residui dichiarati** | 1 | vedi sotto |

Il residuo e' `collect_read_violations` in `reader_adapters.rs:633`: valida le
geometrie del batch con il tetto predefinito perche' non riceve le opzioni — la
firma prende contratto, batch e offset. Un `--max-wkb-cell-bytes` piu' stretto
del default non e' quindi applicato li', benche' lo sia in inferenza e nella
materializzazione. **Fuori dal perimetro di S5**, che copre l'inferenza dei tre
driver tabellari; chiuderlo richiede di portare i limiti dell'operazione dentro
la validazione del contratto di lettura. Il gate lo stampa a ogni corsa, cosi'
non diventa invisibile per abitudine.

> **Superato da S5.1.** "Fuori dal perimetro" era sbagliato nel merito: quel
> punto sta sul percorso comune che ogni driver attraversa. Il residuo e' stato
> chiuso con codice, il censimento e' a zero, e la possibilita' stessa di
> dichiarare un residuo e' stata tolta dallo script. Vedi la sezione S5.1.

Analogamente, le tre mutazioni sopravvissute qui sopra sono tutte uccise da
S5.1: le funzioni sono private, ma i test del modulo le raggiungono, e
chiamarle direttamente isola cio' che l'esercizio attraverso `open` mascherava.

### Il fuzz resta un gap di qualifica

I target di fuzzing sono stati adeguati alle nuove firme, ma **non compilano in
questo ambiente**: il `Dockerfile.dev` non ha nightly ne' `cargo-fuzz`. Non
sono percio' dichiarati verificati, e restano fra i gate non misurabili qui
insieme alla coverage.

---

## S5.1 — chiusura dei residui aperti da S5

S5 aveva lasciato tre cose in sospeso e una quarta l'ho trovata scrivendone i
test. Nessuna e' stata chiusa con documentazione.

### 1. `collect_read_violations` riceve i limiti dell'operazione

Era il residuo dichiarato dal censimento, ed era classificato "fuori dal
perimetro di S5". La classificazione era sbagliata nel merito: quella funzione
sta sul **percorso comune di lettura**, quello che ogni driver attraversa, ed
era l'unico punto dove un `--max-wkb-cell-bytes` piu' stretto del default non
arrivava. Chi stringeva il flag lo vedeva applicato in inferenza e nella
materializzazione, ma non nella validazione del batch.

La firma prende ora `wkb: &WkbLimits`, e il chiamante nel drenaggio passa
`&self.budget.context().limits().wkb_limits()`. La stessa firma attraversa
`validate_read_batch`, l'helper `#[cfg(test)]`.

Il censimento e' percio' passato da un residuo dichiarato a **zero**, e il
meccanismo che permetteva di dichiararne uno e' stato rimosso dallo script: da
S5.1 una occorrenza di produzione ha due sole uscite, `LEGITTIME` con la
ragione scritta oppure il codice cambia. Dichiarare e rinviare era il
meccanismo che teneva aperto il difetto.

### 2. `geometry_components` compone i due tetti

Costruiva `WkbLimits.max_components` dal solo residuo del contatore cumulativo
`GeometryComponents`. Con il default di quel contatore — oltre sedici milioni —
il residuo non legava praticamente mai, quindi `--max-wkb-components`, che e'
un tetto **per cella**, non aveva effetto sulla validazione del batch: una
singola geometria arbitrariamente complessa passava, purche' l'operazione nel
complesso avesse ancora quota. I due limiti hanno significato diverso e vanno
composti:

```rust
max_components: context_limits.effective_wkb_components().min(saturating_usize(
    budget.remaining(OperationCounter::GeometryComponents),
)),
```

Il test `il_tetto_per_cella_dei_componenti_lega_anche_con_quota_cumulativa_ampia`
prende una `LineString` di quattro punti, quota cumulativa un milione e tetto
per cella due: deve fallire. Con tetto per cella sedici la stessa geometria
passa, cosi' il rifiuto e' attribuibile al per-cella e non ad altro.

### 3. Encoder WKB bounded

CSV, GeoJSON e XLSX controllano la **rappresentazione d'ingresso** — il testo
WKT, il JSON grezzo — prima di costruire l'AST. E' il controllo giusto per
fermare un documento enorme senza allocarlo, ma non e' una maggiorazione della
dimensione codificata: `POINT (1 2)` occupa 11 caratteri e 21 byte in WKB,
perche' due `f64` costano 16 byte da soli. Il `Vec` cresceva quindi oltre
`max_wkb_cell_bytes` e il rifiuto arrivava dall'adapter, a memoria gia'
allocata.

**Scelta: un sink bounded, non un calcolo della dimensione.** Il perimetro
lasciava aperte le due strade. Una stima della lunghezza codificata sarebbe
stata una seconda implementazione del formato, che puo' divergere dalla prima:
sbagliata per difetto lascia passare cio' che doveva fermare, per eccesso
rifiuta geometrie valide, e in nessuno dei due casi il compilatore se ne
accorge. `BoundedSink` invece **e'** il writer — il tetto e' verificato prima
di ogni `extend_from_slice`, quindi il buffer non supera il limite nemmeno di
un byte.

`encode_wkb_into` resta e delega a `encode_wkb_into_bounded(.., usize::MAX)`:
i chiamanti che non hanno una quota non cambiano.

| Test | Grandezza osservata |
|---|---|
| `l_encoder_bounded_non_supera_mai_il_tetto` (model) | `buffer.len()` per tetti 1/9/21/len-1, e che al tetto esatto passi |
| `wkb_from_gj_value_non_fa_crescere_il_buffer_oltre_il_tetto` (GeoJSON) | `buffer.len()` sul confine del driver |
| `il_wkb_codificato_non_supera_il_tetto_anche_se_il_testo_ci_sta` (CSV, XLSX) | l'errore dal reader, con testo sotto soglia |
| `il_wkb_codificato_non_supera_il_tetto_anche_se_il_json_ci_sta` (GeoJSON) | idem, con una `LineString` di dieci punti |

Il caso GeoJSON ha richiesto una geometria costruita apposta: il JSON e'
verboso e per un punto pesa piu' del WKB. Una `LineString` spende sei byte per
punto in JSON (`[1,2],`) e sedici in WKB, quindi da tre punti in su la codifica
supera il testo; il test ne usa dieci e verifica la premessa invece di
assumerla.

### 4. Le mutazioni sopravvissute a S5 non erano non-isolabili

La registrazione di S5 spiegava tre sopravvivenze come ridondanza fra controlli.
La spiegazione era sbagliata: le funzioni sono private, ma **i test del modulo
le raggiungono**. Bastava chiamarle direttamente invece di esercitarle
attraverso `open`, dove `infer_types` gira per primo e maschera l'altra passata.

| Mutazione | Prima | Ora |
|---|---|---|
| tetto di riga rimosso da `infer_types` soltanto | sopravvive | **uccisa** da `infer_types_si_ferma_al_tetto_di_righe` |
| tetto di riga rimosso da `infer_wkt_geometry` soltanto | sopravvive | **uccisa** da `infer_wkt_geometry_si_ferma_al_tetto_di_righe` |
| lettura torna al default | sopravvive | **uccisa** da `append_geometry_applica_il_tetto_per_cella` |
| `collect_read_violations` torna al default | n/d | **uccisa** da `collect_read_violations_usa_i_limiti_ricevuti` |
| encoder torna a non-bounded (CSV) | n/d | **uccisa** da `il_wkb_codificato_non_supera_il_tetto_anche_se_il_testo_ci_sta` |
| encoder torna a non-bounded (GeoJSON) | n/d | **uccisa** da due test |
| encoder torna a non-bounded (XLSX) | n/d | **uccisa** da `il_wkb_codificato_non_supera_il_tetto_anche_se_il_testo_ci_sta` |

Zero sopravvivenze.

### 5. Correzioni documentali e registrazioni

- La tabella decisionale L0.1 citava ancora
  `inference_respects_max_input_entries`; riporta ora
  `inference_respects_max_rows_before_materialising`, con il rimando
  all'errata. Il test sulle entry resta nel preflight e non e' duplicato sui
  record.
- I cinque entry point `__fuzz_*` sono registrati nel contenuto di S12, da
  mettere dietro una feature `fuzzing`: `doc(hidden)` li toglie dalla
  documentazione, non dalla superficie pubblica. Il censimento ne classificava
  due perche' sono gli unici che usano `WkbLimits::default()`; gli altri tre —
  `__fuzz_read_dxf`, `__fuzz_read_geojson`, `__fuzz_read_kml` — hanno la stessa
  natura.
- Il censimento spogliava il sorgente senza rimuovere commenti e stringhe, e
  alla prima corsa dopo S5.1 ha contato come residuo la doc che spiegava
  perche' un default era stato rimosso. Usa ora lo stesso spoglio degli altri
  due gate; una sonda conferma che continua a intercettare codice vero.

### Conteggi del censimento dopo S5.1

| Categoria | Prima | Dopo |
|---|---|---|
| test | 45 | 47 |
| attrezzaggio | 4 | 4 |
| produzione, legittimi | 2 | 2 |
| produzione, **residui** | 1 | **0** |

---

## S5.1a — riconciliazione del contratto dell'encoder bounded

Quattro correzioni, tre documentali e una di comportamento.

### Il pacchetto dichiarava ancora aperto un residuo chiuso

La sezione "Errata S5" del decision package descriveva
`collect_read_violations` come residuo fuori perimetro, censito e rinviato.
E' sostituita da un'errata S5.1 che registra la chiusura e dice perche' la
classificazione originaria era sbagliata: quel punto sta sul percorso comune
di lettura, non ai margini.

### Il rustdoc di `BoundedSink` descriveva un tetto incrementale

Diceva che `max_bytes` e' contato dalla lunghezza iniziale, perche' "il
chiamante puo' passare un buffer non vuoto". Non e' cosi':
`encode_wkb_into_bounded` svuota sempre il buffer prima di costruire il sink,
quindi il tetto e' la dimensione massima della codifica, assoluta. Con un
buffer preesistente la vecchia formulazione avrebbe descritto un limite piu'
permissivo di quello reale.

### "Fallisce se raggiunge `max_bytes`" era falso

Il confronto e' `>`, non `>=`: una codifica lunga esattamente `max_bytes`
passa, ed e' il comportamento voluto — un tetto e' un massimo, non un valore
proibito. Il test `il_tetto_esatto_e_ammesso` fissa il confine, cosi' un
irrigidimento accidentale a `>=` non passa silenziosamente.

### Su errore il buffer resta vuoto

Prima lo stato del buffer dopo un `Err` non era definito: conteneva il
prefisso scritto fino al rifiuto. Un prefisso WKB parziale e' una sequenza di
byte ben formata fino a dove arriva, quindi riutilizzabile per sbaglio senza
che nulla protesti — e' esattamente il tipo di valore che sopravvive a un
`if let Err(_) = ... { /* log */ }` distratto.

`encode_wkb_into_bounded` svuota ora il buffer su qualunque errore, non solo
sul superamento del tetto: anche una geometria incoerente rifiutata a meta'
codifica lo lascia vuoto. `wkb_from_gj_value` allinea la stessa postcondizione
sul confine del driver, includendo il fallimento della **conversione**, che
avviene prima che venga scritto un byte: il buffer contiene una codifica
completa oppure niente, indipendentemente da dove l'errore sia nato.

### Lo svuotamento avrebbe mascherato il test sulla crescita

E' la conseguenza che andava gestita, non un dettaglio. I test scritti in S5.1
asserivano `buffer.len() <= tetto` dopo l'errore. Con lo svuotamento quella
misura vale zero **sempre**, quindi non distingue piu' un encoder bounded da
uno che cresce fino in fondo e poi ripulisce — cioe' proprio il difetto che
S5.1 aveva corretto.

La verifica della crescita e' percio' scesa al livello dove resta osservabile:
un test in `wkb_lossless` costruisce `BoundedSink` a mano, chiama
`write_geometry` e misura il buffer, dove nessuno ripulisce. Ai livelli
superiori i test asseriscono ora la postcondizione pubblica, che il buffer sia
vuoto.

Verificato per mutazione, ed e' il risultato che giustifica lo spostamento:

| Mutazione | Esito |
|---|---|
| rimosso lo svuotamento su `Err` | **uccisa** da `l_encoder_bounded_lascia_il_buffer_vuoto_su_errore` e da `wkb_from_gj_value_lascia_il_buffer_vuoto_su_errore` |
| il sink scrive e poi rifiuta a posteriori | **uccisa** dal solo `il_sink_non_lascia_mai_crescere_il_buffer_oltre_il_tetto`; **tutti** i test dei tre driver passano |

La seconda riga e' la misura di quanto la copertura sarebbe stata illusoria
lasciando gli assert dov'erano.
