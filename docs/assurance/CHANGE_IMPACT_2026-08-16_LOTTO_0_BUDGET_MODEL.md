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
  `PipelineContext::observe_input(permit, bytes, entries) ->
  Result<SourceFootprint, PlenoraIoError>`. Un'osservazione fabbricata o
  registrata su un context diverso non e' rappresentabile.
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
