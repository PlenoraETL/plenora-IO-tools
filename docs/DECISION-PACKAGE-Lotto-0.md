# Pacchetto decisionale — Lotto 0 (API hardening del core)

Stato: **ratificato** il 2026-08-16 (step S0). Il documento e' normativo
per il Lotto 0: gli invarianti INV-1..INV-14 vincolano l'implementazione,
e ogni deviazione richiede una revisione di questo pacchetto, non una
scelta locale in un PR.

La ratifica autorizza l'apertura del Lotto 0 di implementazione a partire
da S1. Restano **fuori** dal mandato, e nessuno step di questo pacchetto
li tocca: i manifesti `release/*.json` (incluso un futuro `1.1.0.json`),
l'evidence base sotto `release/evidence/`, la facade Rust
`plenora-io-api`, i comandi CLI nuovi e l'SDK Python. La CIA di S0 e'
registrata in
`docs/assurance/CHANGE_IMPACT_2026-08-16_LOTTO_0_BUDGET_MODEL.md`.

Documento di scope **congiunto**: copre in un unico modello le
decisioni interdipendenti che non possono essere prese separatamente
senza rischio di scelte incoerenti fra loro.

## Contesto

La verifica statica ha confermato **10 finding residui** nel core dopo
il lotto di hardening precedente:

| Id | Finding | File chiave |
|---|---|---|
| L0.1 | Limiti non arrivano alle passate di inferenza/parsing | `driver-csv/src/lib.rs`, `driver-geojson/src/lib.rs`, `driver-xls/src/lib.rs` |
| L0.2 | `Limits` e `ResourceLimits` con semantiche divergenti; doc pubblica prescrive budget condiviso reader/writer, CLI usa budget separati | `plenora-io-core/src/driver.rs`, `plenora-io-model/src/{limits,resource}.rs` |
| L0.3 | Budget memoria consumato definitivamente in `commit`, mai restituito quando batch e' consumato | `plenora-io-core/src/driver/reader_adapters.rs` |
| L0.4 | Descriptor `ReadMode::Streaming*` in contraddizione con adapter operation-atomic | `plenora-io-core/src/driver/reader_adapters.rs`, descrittori driver |
| L0.5 | Validazione covering GeoParquet incompleta (nomi ok, tipi/unicita' no) | `driver-geoparquet/src/lib.rs` |
| L0.6 | Redazione errori non centralizzata; payload utente puo' entrare nei messaggi pubblici | `driver-geojson/src/lib.rs`, `plenora-io-model/src/error.rs` |
| L0.7 | Format options: chiavi sconosciute ignorate, valori invalidi degradano al default | `driver-geoparquet/src/lib.rs`, tutti i driver |
| L0.8 | `wkb_shape` non ispeziona figli delle collection | `driver-gpkg/src/lib.rs` |
| L0.9 | Directory scan senza cap sul numero di entry | `plenora-io-core/src/driver.rs` |
| L0.10 | `output_expansion_ratio` perso con budget separati: solo il read budget osserva l'input; il write budget vede zero e non applica l'espansione | `plenora-io-cli/src/main.rs`, `plenora-io-model/src/resource.rs` |

Questo pacchetto tratta le decisioni **architetturali congiunte**
richieste da L0.2, L0.3, L0.4, L0.9 e L0.10. Le decisioni tecniche
locali (L0.5, L0.6, L0.7, L0.8, L0.1 di propagazione) restano
implementative del Lotto 0 ma non sono oggetto di questo documento —
tranne dove il modello proposto le vincola direttamente (L0.6
richiede una disciplina di errore che il modello espone).

## Obiettivi

O1. Definire un modello di budget coerente, che elimini il doppio
    conteggio in `convert` senza perdere le grandezze condivise
    (input osservato, memoria, spill, deadline, concorrenza, entry).

O2. Chiudere la contraddizione fra descriptor "streaming" e adapter
    operation-atomic con una semantica machine-readable esplicita.

O3. Sostituire il consumo cumulativo della memoria con un gauge
    lease-based, correttamente rilasciato al drop del batch.

O4. Ratificare ADR-IO 7 con una scelta d'implementazione (spool
    bounded), preservando l'atomicita' operativa gia' presa come
    invariante.

O5. Esporre `max_input_entries` come limite di prima classe accanto
    a `max_input_bytes`.

O6. Fissare una disciplina strutturale di redazione dell'errore che
    renda impossibile — al compilatore — inserire payload derivati
    dall'input in messaggi pubblici (bozza di API, dettaglio pieno
    in L0.6).

O7. Rendere l'API di configurazione **unica**: il consumer non deve
    piu' costruire separatamente `Limits` e `ResourceBudget`.

## Perimetro

**Incluso**:
- Modello dei budget (pipeline + operation) e ownership dei contatori.
- Lifecycle della memoria (gauge + lease + drop).
- Semantica dei descriptor (native mode vs effective delivery).
- Propagazione dell'input osservato al writer.
- Integrazione di `max_input_entries`.
- Bozza di disciplina strutturale di redazione errore (L0.6),
  dettaglio d'implementazione al lotto successivo.

**Escluso** (implementativo, non decisionale):
- Correzioni driver-specifiche (validazione covering GeoParquet L0.5,
  `wkb_shape` L0.8, propagazione limiti in inferenza L0.1).
- Schema dichiarativo `format_options` L0.7 (design proprio in
  documento separato).
- Facade `plenora-io-api`, CLI completo, SDK Python: sospesi finche'
  il Lotto 0 non chiude e non esiste una release stabile del core.

## Invarianti

Le seguenti proprieta' devono valere dopo il Lotto 0. Sono elencate
come contratto testabile; ogni finding ha almeno un'invariante di
riferimento.

**INV-1 (unicita' del modello dei limiti)**
Esiste **una sola** rappresentazione pubblica dei limiti I/O.
`Limits` viene rimosso come tipo separato; le sue quote diventano
campi di un unico `PipelineLimits` che copre tutte le grandezze di
governo.

**INV-2 (unicita' del punto di costruzione)**
Il consumer costruisce **un solo** oggetto radice
(`PipelineBudget`); da questo derivano tutti i budget necessari
alla pipeline. Non esistono percorsi pubblici che consentano di
costruire un `OperationBudget` senza passare da un
`PipelineBudget`. **Corollario**: la CLI non puo' piu' cablare
manualmente `Limits + ResourceBudget::default()`; passa dal
`PipelineBudget::builder()`.

**INV-3 (nessun doppio conteggio in `convert`)**
Reader e writer di una pipeline `convert` non consumano lo stesso
contatore `Rows`, `OutputBytes`, `GeometryComponents`. Ogni
operazione ha i propri contatori cumulativi. **Corollario**:
`--max-rows R` cappa il reader a R righe lette **e** il writer a R
righe scritte; non e' R totali.

**INV-4 (grandezze pipeline-wide condivise)**
Deadline, `observed_input_bytes`, memoria, spill, concorrenza e
numero di entry sono **condivise** fra reader e writer della
stessa pipeline. Vivono nel `PipelineContext`, non nei singoli
`OperationBudget`.

**INV-5 (budget della sola memoria posseduta internamente;
`InternalMemoryLease` rilasciata al transfer al consumer)**
Il gauge di memoria copre **esclusivamente le allocazioni che la
libreria detiene internamente** negli adapter (batch worker in
costruzione, coda del reader spool, staging temporanei del
writer). Non copre e non tenta di seguire la memoria dei
`RecordBatch` gia' consegnati al consumer.

**Semantica unica del ciclo di vita**:

1. Un adapter interno prende una `InternalMemoryLease(bytes)`
   prima di allocare il buffer.
2. La lease resta viva finche' la libreria detiene fisicamente
   quel buffer.
3. Al momento in cui la funzione pubblica `next_batch()`
   trasferisce l'ownership del `RecordBatch` al consumer, la
   libreria **rilascia** la `InternalMemoryLease` per quei
   bytes. Da quel punto il batch e' fuori dal dominio del gauge.

Il gauge riflette la RAM in uso dalla libreria in ogni istante.
La memoria dei batch consegnati al consumer non compare mai nel
budget: la libreria non tenta di modellare cio' che non possiede
e non introduce dipendenze sul comportamento del chiamante dopo
il transfer.

Alternative considerate e scartate:
- **Ownership reale** (allocator custom / arena): richiederebbe
  di sostituire l'allocatore Arrow o wrappare `Buffer`. Costo
  alto, incompatibile con `forbid(unsafe_code)`.
- **Bookkeeping-token-tenuto-dal-consumer**: fragile e
  incompatibile con la firma `next_batch() -> RecordBatch`.
  Rigettata.
- **Commit cumulativo** (pre-Lotto-0): non modella un gauge,
  produce il finding L0.3.

**INV-6 (propagazione input al writer, con distinzione
`NotObserved` vs `Bytes(0)`)**
Il writer legge lo stato dell'input dal `PipelineContext`
condiviso, come **enum tipizzato** — non come `AtomicU64` con
sentinel 0:

```
enum ObservedInput { NotObserved, Bytes(u64) }
```

Regola per `output_limit`:
- `NotObserved` (nessun preflight ha girato, per esempio writer
  standalone senza pipeline read): usa `output_bytes_absolute`
  soltanto, `output_expansion_ratio` **non** applicato.
- `Bytes(0)` (preflight ha girato, input effettivamente vuoto):
  come sopra — un input vuoto non deve trasformare
  `0 * expansion_ratio = 0` in un tetto zero che vieta ogni
  output.
- `Bytes(n)` con `n > 0`: applica
  `min(output_bytes_absolute, n * output_expansion_ratio)`.

`Source::into_path_checked` e' l'unico chiamante autorizzato a
transire da `NotObserved` a `Bytes(n)`. La transizione e'
one-shot: un secondo preflight sulla stessa pipeline e' un errore
di contratto (evita divergenze fra reader e writer che
osserverebbero valori diversi). **Corollario**: L0.10 chiuso —
entrambi i budget guardano lo stesso stato tipizzato, e il caso
"input non osservato" e' distinto da "input di 0 byte" per
costruzione.

**INV-7 (descriptor con native mode, delivery semantics, buffering
strategy separati)**
Ogni driver dichiara **tre** campi ortogonali:

- `native_read_mode: NativeReadMode` — cosa fa il parser grezzo
  del driver (`StreamingSequential`, `StreamingRandom`,
  `Materialized`).
- `effective_delivery: DeliverySemantics` — cosa il consumer
  osserva a livello di contratto pubblico
  (`OperationAtomic`, `Streaming`). Descrive **quando** il primo
  batch e' visibile e cosa succede in caso di errore dopo la
  consegna.
- `buffering: BufferingStrategy` — come e' bounded la memoria
  interna (`Passthrough`, `InMemoryBounded`,
  `AdaptiveMemoryThenDisk`). Descrive **come** l'implementazione
  bounda le risorse; non parte della semantica del contratto.

Un consumer che vuole latenza al primo batch guarda
`effective_delivery` (deve essere `Streaming`, non oggi
disponibile). Un consumer che vuole capire l'impronta risorse
guarda `buffering`. `native_read_mode` resta informativo su cosa
il driver riesce a fare se un adapter alternativo lo espone
direttamente in futuro.

**Corollario**: L0.4 chiuso. Ex-INV-8 (spool preserva atomicita')
diventa una scelta di combinazione (`OperationAtomic` +
`AdaptiveMemoryThenDisk`) ratificata da ADR-IO 7 opzione A; altre
combinazioni sono declared-only e non selezionabili nel Lotto 0.

**INV-8 (`OperationAtomic` = validation atomicity + publish
atomicity; errori durante IPC replay sono possibili e tipizzati)**

`DeliverySemantics::OperationAtomic` garantisce **due proprieta'
distinte** ed esclusivamente queste:

- **Validation atomicity**: la validazione dell'intera sorgente
  completa **prima** che il writer entri in `write_batch`. Se la
  validazione fallisce, il writer non viene mai istanziato.
- **Publish atomicity**: se il writer entra e poi fallisce, la
  destinazione non viene pubblicata (garanzia data dal pattern
  `StagedFile` gia' presente, invariato dal Lotto 0).

`OperationAtomic` **non** implica:
- Assenza di errori durante il replay dei batch dallo spool.
  Fra fine-validazione e fine-scrittura, la fase "replay dallo
  spool file al writer" puo' fallire su:
  - I/O read del file temporaneo (disco pieno, cancellazione
    esterna del file, permessi cambiati);
  - decodifica Arrow IPC del batch salvato (corruzione,
    incompatibilita' version, header rotto);
  - deadline scaduta durante il replay.
  Questi errori sono **tipizzati** con `ErrorKind::Io(IoErrorKind::
  SpoolReplay)` o `ErrorKind::Contract(SpoolCorruption)` e
  producono `RemoteEffect::None` (il writer non ha ancora
  pubblicato — il pattern `StagedFile` garantisce la destinazione
  intatta). Non producono `Partial`, perche' il writer non ha
  toccato la destinazione.
- Recupero dei batch spool sopravvissuti a un crash. Il file di
  spool non lascia orfani: il file non ha nome e il kernel ne
  libera l'inode alla chiusura del descrittore
  handler (vedi ADR-IO 7 aggiornato), non ricostruito.

`OperationAtomic` **non** copre nemmeno:
- **Set loose Shapefile**: non crash-atomic per definizione (vedi
  finding #10 chiuso con `RemoteEffect::Partial`).
- **Directory esterne per GeoPackage/FileGDB**: garanzia entro il
  perimetro di ogni driver, non fra filesystem diversi.

L'atomicita' offerta e' quindi in tre stadi:
1. `validate` (leggi + spool completo, no destinazione) → errore
   qui: nessun output.
2. `replay` (rileggi spool → passa al writer) → errore qui:
   nessun output.
3. `publish` (writer rename atomico) → errore qui: nessun output
   se pre-rename, `RemoteEffect::Partial` solo per loose sets.

Lo spool bounded aggiunge **memoria bounded**, non atomicita':
l'atomicita' era gia' presente in `BudgetedReader` pre-Lotto-0.

L'alternativa `Streaming` (`effective_delivery = Streaming`)
rilascerebbe la validation atomicity e richiede un nuovo asse
d'errore `TerminatedAfterAcceptedBatches` con coordinamento
cross-component. Non ratificata in questo pacchetto.

**INV-9 (limite entry come cittadino di prima classe)**
`PipelineLimits::max_input_entries: u64` esiste ed e' controllato dallo
scan di directory prima della somma dei byte. **Corollario**: L0.9
chiuso. Il default va scelto in modo che una directory di
migliaia di file legittimi non sia rifiutata (proposta: `10_000`
entry, coerente con `max_columns` come ordine di grandezza).

**INV-10 (redazione errore strutturale; nessun canale FNV
pubblico; DTO conforme a `plenora-io-error-v1`)**
La `PlenoraIoError` pubblica NON ha un campo `message: String`
alimentato a runtime, e NON espone alcun hash del payload
(`Fingerprint` o simili). Il testo pubblico e' derivato
esclusivamente da un enum tipizzato `PublicMessage` con
parametri appartenenti a un insieme consentito:

- `&'static str` (compile-time, non user-controlled);
- tipi enum interni al workspace (`ErrorCategory`, `ErrorPhase`,
  `IoErrorCode`, `CapabilityReason`, `GeometryType`, ecc.);
- `LayerId`, `FieldId` (indici numerici, non user-controlled).

Il `ContractIdentifier` (nome campo/layer, safe-by-construction)
**non** e' un parametro del messaggio: vive nell'`ErrorContext`
strutturato, e da li' il DTO deriva direttamente il campo `field`
del wire.

**Vietati per costruzione**:
- `String` costruita da valori di cella;
- percorsi assoluti;
- WKB bytes o loro rappresentazioni testuali;
- CRS raw definitions;
- messaggi di errore forniti da dipendenze C (es. GDAL);
- **hash o fingerprint del payload utente** (canale covert:
  rimossi in questa iterazione).

**Enforcement Rust**: la struttura pubblica di `PlenoraIoError` ha
campi privati con getter; `PublicMessage` non ha costruttori che
accettino `String` libera. Il compilatore rifiuta
`format!("... {value}")` propagato come messaggio.

**Enforcement wire**: la serializzazione verso `plenora-io-error-v1`
passa da un DTO privato `PublicErrorDto` che rende deterministicamente
il testo curato in un campo `message: String`: **struttura wire
invariata** (tutti i campi, ordine, tipi identici alla baseline),
`message` **intenzionalmente diverso**. I contenuti sensibili possono confluire
in `row_diagnostics`, gated dalla policy `emit`/`redact` esistente
(contratto `plenora-row-diagnostics-v1`).

**Corollario**: L0.6 chiuso al livello del compilatore (Rust) e
del serializzatore (wire). Vedi sezione dedicata "Redazione
errore strutturale (INV-10) + DTO conforme v1" per la struttura
dei tipi e il rendering.

**INV-11 (deadline cumulativa, non moltiplicata)**
La deadline vive nel `PipelineContext`. Un convert con
`--timeout-ms N` ha `N` millisecondi **totali**, non `N` per il
reader e `N` per il writer. Il modello attuale gia' condivide la
deadline via `Arc<Counters>`; il pacchetto lo formalizza come
invariante.

**INV-12 (concorrenza via `ResourcePool` opzionale)**
Un singolo `PipelineBudget` **non** limita le pipeline
concorrenti verso altri budget. Senza pool: i gauge **memory** e
**spill** esistono e sono **locali al context**; una quota di
concorrenza **non esiste affatto** (nessun gauge locale da
consultare). Chi vuole limitare la somma di piu' pipeline usa un
`ResourcePool` condiviso.

**Composizione `PipelineLimits` ⊓ `ResourcePool`** (regola unica):
- **memory/spill**: **senza pool** la quota e' quella locale di
  `PipelineLimits` e i gauge sono locali al context. **Con pool**
  la quota effettiva e' il **minimo** fra quota locale e quota del
  pool: una lease passa solo se sta sotto **entrambi** e consuma
  **entrambi** i gauge.
- **concurrency**: **senza pool** e' **assente** — `PipelineLimits`
  non ha alcuna quota di concorrenza locale e `lease_concurrency()`
  e' un **no-op** che restituisce sempre una lease. **Con pool** e'
  governata **solo** dal pool.

**Corollario**: reader e writer di uno stesso `convert`
condividono i gauge del proprio context (INV-4); piu' `convert`
in parallelo competono sui gauge solo se agganciati allo stesso
`ResourcePool`. Senza pool, "limitare le pipeline concorrenti"
non e' una promessa che la libreria mantiene.

**INV-13 (dipendenza `model` → `core` vietata; permit opaco
one-shot per l'osservazione dell'input)**
Tutti i tipi del modello budget (`PipelineBudget`,
`PipelineContext`, `OperationBudget`, `PipelineLimits`,
`InternalMemoryLease`, `CountedLease`, `SpillLease`,
`ConcurrencyLease`, `ObservedInput`, `InputPermit`) vivono in
`plenora-io-model`. Il modello **non** importa tipi di
`plenora-io-core`. Dipendenza unidirezionale `core → model`.
Il modello **non** definisce ne' `ReadOptions` ne' `WriteOptions`:
quelle sono factory di opzioni di lettura/scrittura e vivono nel
core (vedi sezione API).

**Permit opaco one-shot per `observe_input`**:

L'osservazione dell'input passa da un `InputPermit`, non da una
struct che il chiamante possa costruire con dati arbitrari:

- Il permit e' un tipo `#[non_exhaustive]` con costruttori
  privati: nessun campo pubblico, nessuna via di costruzione
  esposta al di fuori del modello.
- Un permit e' **emesso una sola volta per pipeline** da
  `PipelineBudgetBuilder::build()?`, che restituisce un
  `PipelineBundle` **opaco**: budget e permit non sono campi
  pubblici. Il permit e' **legato al `PipelineContext`** che lo ha
  emesso: porta l'identita' di quel context e non e' spendibile su
  un context diverso.
- Il permit esce dal bundle **solo** dentro le parti opache
  (`ReadBudgetParts`, `ScanBudgetParts`, `ConvertBudgetParts`)
  prodotte dalle `PipelineBundle::into_*_parts`. Non esiste un
  punto in cui il consumer accoppi a mano un permit con un budget:
  l'unico accoppiamento e' quello emesso da `build()`.
- **Errata S4.b.3 — separabilita'.** La formulazione originale
  diceva che il permit non e' "mai separabile" dalle parti. Non e'
  vero, e non puo' esserlo: il core e' un crate distinto dal
  modello, quindi l'API che gli consente di prendere il permit deve
  essere `pub`, e `pub` significa raggiungibile da chiunque aggiunga
  `plenora-io-model` fra le proprie dipendenze. Rust non ha un
  `pub(workspace)`. La formulazione corretta separa cio' che il
  linguaggio impone da cio' che impone la convenzione:
  - **garantito dal tipo**: il permit non e' costruibile
    dall'esterno (nessun costruttore pubblico, nessun campo
    pubblico), non e' `Clone`, ed e' legato al context che lo ha
    emesso — un permit speso altrove e' un `Err`, non un'osservazione
    sbagliata;
  - **garantito dalla convenzione, e verificato**: il permit e'
    separabile **per move** all'interno del workspace, tramite
    l'unica API di decomposizione `ReadBudgetParts::into_components`,
    marcata `#[doc(hidden)]`. Prima di S4.b.3 esisteva accanto a
    questa un `take_input_permit()` pubblico: due vie per la stessa
    separazione, ridotte a una. Il confine regge su tre fatti che il
    gate `scripts/check_permit_boundary.py` controlla a ogni corsa —
    `publish = false` su entrambi i crate, la marcatura, e l'assenza
    di usi fuori da `plenora-io-model` e `plenora-io-core`.

  Non e' una garanzia piu' debole per rassegnazione: e' la garanzia
  che si puo' effettivamente mantenere, dichiarata per quello che e'.
  Una promessa di impossibilita' che il compilatore non sostiene
  varrebbe meno di un confine convenzionale sorvegliato.
- Il permit e' consumato per `move` da `PipelineContext::
  observe_input(permit) -> Result<SourceFootprint,
  PlenoraIoError>` una volta sola. Il metodo appartiene al context
  e registra il footprint **esclusivamente** nel context che ha
  emesso il permit: nessun footprint puo' finire in un context
  diverso. Restituisce `Err` se il permit proviene da un altro
  context, se `ensure_active()` fallisce (cancellazione o deadline
  scaduta) o se un'osservazione risulta gia' registrata (difesa in
  profondita'). **Un errore non modifica lo stato precedente.** Non
  equivale a dire che lo stato resti `NotObserved`: nel caso del
  secondo publish lo stato precedente e' gia' pubblicato, l'errore lo
  lascia pubblicato, e `ObservedInput` continua a riportare il
  footprint registrato. La pubblicazione e' terminale nelle due
  direzioni — non si ripubblica e non si torna indietro — e il test
  `second_observation_is_rejected_and_keeps_the_published_footprint`
  lo verifica.
  Consumato = non riutilizzabile; non essendo `Clone`, il tipo
  Rust garantisce l'unicita' dell'osservazione.
- Il consumer del permit e' `Source::into_path_checked` in
  `plenora-io-core`, che lo estrae dalle `ReadOptions` ricevute
  (`ReadOptions::take_input_permit()`, **`pub(crate)`**: l'unico
  chiamante legittimo vive in quel crate) e chiama
  `context.observe_input(permit)?` sul context a cui il permit e'
  legato.
- Un permit non consumato al drop del bundle o delle parti non
  invalida nulla: significa che nessuna osservazione dell'input
  e' avvenuta, coerente con `ObservedInput::NotObserved`.
- **`observe_input` non ha parametri oltre al permit.** Byte,
  entry e digest vengono tutti dal context, che li ha accumulati
  durante l'enumerazione via `note_entry_visited(entry)`. Nessuno
  dei tre e' dichiarabile dal chiamante: il footprint pubblicato
  descrive esattamente cio' che il preflight ha misurato. Con i
  byte come parametro sarebbe rimasta una seconda sorgente di
  verita' proprio per la grandezza che governa
  `output_expansion_ratio` — la stessa classe di difetto di L0.10,
  spostata dentro il modello nuovo.

`observe_input` e' l'**unica** fabbrica di `SourceFootprint` e
l'**unico** canale di registrazione: non esiste una funzione
libera `observe_measurement` ne' un metodo separato
`record_footprint`. Costruzione e registrazione del footprint sono
lo stesso atto, legato per costruzione al context del permit. Un
consumer non puo' fabbricare una `SourceFootprint` arbitraria
senza aver ottenuto un permit dal proprio `PipelineBudgetBuilder`.

**Alternative considerate e scartate**:
- `pub(crate)` in model per proteggere `observe_input`: non
  funziona, core e' un altro crate.
- Struct evidence con costruttore pubblico: falsificabile.
- Trait `Sealed`: protegge dall'implementazione, non
  dall'invocazione.
- Feature flag `internal-api`: fragile.

**Corollario per il design dello spool**: `StagedSpool` vive in
`plenora-io-core`. Interagisce col budget attraverso l'API
pubblica di `plenora-io-model::budget`: prende una
`SpillLease(bytes)` dal `PipelineContext` prima di allocare il
file temporaneo; ritorna la lease al drop. Nessun tipo di core
attraversa il confine verso model.

**INV-14 (`FormatDescriptor` costruibile dai driver via
`const_new`)**
`FormatDescriptor` e' `#[non_exhaustive]` per i consumer esterni
ma resta costruibile dai driver del workspace via
`const fn const_new(...)` con tutti gli argomenti posizionali
(un nuovo campo obbligatorio rompe le firme dei driver
volutamente). `read_mode` e' argomento esplicito, non derivato
(vedi INV-7 e sezione descriptor). L'introduzione di un nuovo
campo capability (per esempio `hostile_input_hardened`) segue
la stessa regola.

## Modello raccomandato

### Tipi pubblici

Rappresentazione concettuale. I nomi sono provvisori; le firme
mostrano l'idea, non la sintassi finale.

```
PipelineBudget (root, non Clone — token di costruzione one-shot)
├── PipelineContext (Arc, condiviso Send+Sync)
│   ├── deadline: Instant                                 (INV-11)
│   ├── observed_input: Mutex<ObservedInput>              (INV-6)
│   │     enum { NotObserved, Bytes(u64) }
│   ├── memory: MemoryGauge                                (INV-5)
│   │     gauge lease-based locale, bookkeeping-only
│   ├── spill: SpillGauge
│   │     idem (locale)
│   ├── pool: Option<ResourcePool>                         (INV-12)
│   │     se presente: memory/spill = min(locale, pool);
│   │     concorrenza governata solo da qui. Se assente:
│   │     nessun gauge di concorrenza esiste (lease no-op)
│   ├── observation: Mutex<SourceObservation>              (INV-6/9)
│   │     Collecting { entries, total_bytes, digest }
│   │       -> Published(SourceFootprint), terminale.
│   │     Le tre grandezze sono un aggiornamento unico:
│   │     un rifiuto non ne lascia nessuna aggiornata.
│   ├── cancellation: CancellationToken                    (unico per pipeline)
│   └── limits: PipelineLimits                             (immutable)
│
├── read_budget: OperationBudget (Clone via Arc)
│   ├── shares: Arc<PipelineContext>
│   ├── rows_remaining: AtomicU64                          (INV-3, per-op cumulativo)
│   ├── columns_remaining: AtomicU64
│   ├── geometry_components_remaining: AtomicU64
│   └── output_bytes_remaining: AtomicU64
│
└── write_budget: OperationBudget (Clone via Arc)
    (stessa struttura, contatori indipendenti da read_budget)
```

`PipelineLimits` (immutable, letto solo) e' l'unificazione di
`Limits` + parti cumulative di `ResourceLimits`:

Campi (**tutti privati**, esposti da getter e fluent setter — vedi
sezione API):

- Input: `max_input_bytes: u64`, `max_input_entries: u64` (INV-9).
- Per-operazione (contatori indipendenti reader/writer):
  `max_rows: u64`, `max_columns: u64`,
  `max_geometry_components: u64` (cumulativo per operazione),
  `max_output_bytes: u64`, `output_expansion_ratio: u64` (INV-6).
- Per-cella (WKB, applicato dai driver a singola geometria):
  `max_wkb_cell_bytes: usize`, `max_wkb_components: usize`,
  `max_wkb_depth: usize`, `max_vertices: usize`.
  `max_vertices` e' il tetto globale dei vertici gia' esposto dalla
  CLI come `--max-vertices`: stringe il limite per cella esattamente
  come in `Limits::effective_wkb()`, e il modello unificato lo
  espone come `effective_wkb_components() = min(max_wkb_components,
  max_vertices)`. Senza di esso la migrazione allenterebbe in
  silenzio un tetto che un utente puo' stringere oggi.
- Pipeline-wide (INV-4, INV-11, INV-12):
  `memory_bytes: u64` (gauge, non cumulativo), `spill_bytes: u64`,
  `duration_ms: u64`, `decompression_ratio: u64`.
- Concorrenza: si dichiara sul `ResourcePool` opzionale (vedi
  API), non sul `PipelineLimits`. Un singolo `PipelineBudget`
  senza pool ha concorrenza illimitata verso altri processi.

**Nota su `max_wkb_components`**: era il vero problema di `L0.10` in
combinazione col fix `#3` precedente. Nel modello proposto e'
esplicitamente **per-cella**, mai cumulativo. Il campo cumulativo
dataset-wide e' `max_geometry_components`, distinto per nome per
evitare confusione futura.

### Ownership dei contatori

- `PipelineContext` e' contenuto in un `Arc`. Ogni `OperationBudget`
  ha `Arc<PipelineContext>` e vede lo stesso stato condiviso.
- I contatori cumulativi `Rows`, `Columns`, `GeometryComponents`,
  `OutputBytes` sono **campi dell'OperationBudget**, non del
  context: due `OperationBudget` figli hanno contatori
  indipendenti (INV-3).
- Il gauge `memory` e' nel context: reader e writer competono sulla
  stessa quota di RAM disponibile (INV-4/INV-5). Cio' e' corretto:
  la RAM e' una risorsa fisica del processo, non della singola
  operazione.
- `observed_input_bytes` e' nel context: chiunque puo' leggerlo
  (INV-6), solo il preflight `Source::into_path_checked` lo scrive
  (una volta per pipeline).

### Costruzione end-to-end (INV-2): snippet coerenti

Il model produce **parti opache** (`ReadBudgetParts`,
`ConvertBudgetParts`) che trasportano budget + permit + limiti
fino alle factory del core (`ReadOptions::builder(parts)`,
`WriteOptions::builder(parts)`, `convert(...)`). Il consumer non
manipola direttamente `OperationBudget`/`InputPermit` — riceve le
parti dal model e le passa alle factory del core.

```rust
// open: apertura di un Dataset (fase preflight).
let bundle = PipelineBudget::builder()               // -> PipelineBundle opaco
    .limits(PipelineLimits::default().with_max_rows(1_000_000))
    .cancellation(CancellationToken::new())
    .build()?;
let parts = bundle.into_read_parts();                // ReadBudgetParts
let dataset = plenora_io_core::open(path, ReadOptions::builder(parts).build())?;

// scan: ogni lettura del Dataset esibisce un budget fresco.
let scan_bundle = PipelineBudget::builder()
    .limits(PipelineLimits::default().with_max_rows(500_000))
    .build()?;
let parts = scan_bundle.into_scan_parts(dataset.footprint().snapshot());
let reader = dataset.scan(request, ReadOptions::builder(parts).build())?;

// convert: unico context reader+writer condiviso.
let bundle = PipelineBudget::builder()
    .limits(PipelineLimits::default().with_max_rows(10_000_000))
    .build()?;
let convert_parts = bundle.into_convert_parts();     // ConvertBudgetParts
let (read_parts, write_parts) = convert_parts.into_parts();
let published = plenora_io_core::convert(
    source, destination,
    ReadOptions::builder(read_parts).build(),
    WriteOptions::builder(write_parts).build(),
)?;

// write standalone: nessun input osservato.
let write_bundle = PipelineBudget::builder()
    .limits(PipelineLimits::default().with_max_output_bytes(1 << 30))
    .build()?;
// `into_write_parts` non trasporta il permit: viene droppato con il
// bundle. `observed_input()` resta `NotObserved`.
let parts = write_bundle.into_write_parts();         // WriteBudgetParts
let writer = plenora_io_core::create(sink, plan, WriteOptions::builder(parts).build())?;
```

`ReadBudgetParts`, `ScanBudgetParts`, `ConvertBudgetParts`,
`WriteBudgetParts` sono tipi opachi (nessun campo pubblico); non
sono `Clone`; ognuna esce dal model verso il core e viene consumata
per costruire la relativa options. Non e' possibile derivare un
`OperationBudget` in altro modo (INV-2).

### Scope, Clone, Cancellazione, Concorrenza (contratto esplicito)

Regole vincolanti dei tipi budget. Ognuna e' verificabile con test
di firma o di comportamento.

**Scope temporale**:
- `PipelineBudget`: **una sola invocazione**. Costruito prima di
  `open`/`create`/`convert`, consumato al termine dell'operazione.
  Un `PipelineBudget` non e' pensato come singleton di processo;
  non si crea uno per il processo intero e lo si riusa. Il ciclo
  di vita e': `builder` → `build` → `into_*_parts` → drop di
  `OperationBudget` → drop implicito del context Arc quando
  l'ultima referenza scompare.
- `OperationBudget`: legato al `Dataset`/`Writer` che lo consuma.
  Sopravvive fintanto che il handle di operazione e' vivo. Il
  drop non forza il drop del context (altri sibling potrebbero
  essere ancora attivi).

**Clone**:
- `PipelineBudget`: **non Clone**. E' un token di costruzione;
  clonarlo darebbe due radici che pretendono ownership dello
  stesso context. Le `into_*_parts` sono l'unico modo per derivare
  piu' budget.
- `OperationBudget`: **Clone** (via Arc). Tutti i cloni vedono lo
  stesso stato (stessi contatori cumulativi, stesso context).
  Consente ai driver di clonare il budget per adapter interni
  (batch worker, staging) senza creare contatori paralleli.
  Un clone consumato non doppia il consumo.

**Cancellazione**:
- Il `CancellationToken` **e' unico per pipeline** e vive nel
  `PipelineContext`. `OperationBudget::context().cancellation()`
  restituisce sempre lo stesso token.
- Cancellare il token dal chiamante annulla **contemporaneamente**
  reader e writer del convert (comportamento voluto: un utente
  che preme Ctrl+C vuole fermare tutto).
- I driver non possono creare cancellation token propri e
  scavalcare il token della pipeline. Un token derivato via
  `child()` e' consentito (cancella solo il ramo derivato), ma
  il cancel del padre continua a propagare.

**Concorrenza (thread-safety)**:
- `PipelineContext`: `Send + Sync`. Tutti i suoi contatori sono
  atomici; deadline e limits sono immutabili.
- `OperationBudget`: `Send + Sync`. Contatori cumulativi atomici;
  Arc del context condiviso.
- `InternalMemoryLease`, `SpillLease`, `ConcurrencyLease`:
  `Send`, **non Sync** (una singola lease ha un solo
  "proprietario" del drop; condividerla via `&` fra thread non
  ha senso semantico).
- `CountedLease`: `Send`, non `Sync`, stessa motivazione.
- Due thread possono chiamare `next_batch` su due `LayerReader`
  diversi ottenuti dallo stesso `Dataset`: legale se il driver
  espone `ReaderConcurrency::MultipleIndependentReaders`. Il
  budget condiviso vede le due sequenze di lease in modo
  atomico; non c'e' race sui contatori.
- Due thread che chiamano `next_batch` sullo **stesso**
  `LayerReader` sono un errore di contratto (il reader ha stato
  interno mutabile non protetto da lock — pattern gia' presente
  nel core, questo pacchetto non lo cambia). Il reader e' `Send`
  ma non `Sync`.

**Errori di lease sotto cancellazione o deadline scaduta**:
- Ogni `try_lease` verifica prima `context.ensure_active()`.
- `ensure_active()` restituisce `PlenoraIoError { kind:
  ErrorKind::Cancelled }` se il token e' cancellato, `kind:
  ErrorKind::LimitExceeded(LimitKind::Duration)` se la deadline
  e' scaduta.
- Le due condizioni non sono conflate: un consumer puo'
  distinguere "utente ha chiesto stop" da "budget temporale
  finito".

### Scope dei budget per operazione (Dataset senza budget vivo)

Regola generale: **`Dataset` non incapsula un `PipelineBudget`
vivo**. Il Dataset conserva solo metadata (schema, layers, CRS)
e la `SourceFootprint` osservata dal preflight che ha portato
alla sua apertura. Deadline, cancellazione, contatori e permit
vivono in un `PipelineBudget` **creato per ogni scan/operazione**.
Solo un `convert` condivide un unico context fra reader e writer.

**`open(path, ReadOptions) -> Dataset`**
- Consuma un `PipelineBudget` per la sola fase di apertura:
  preflight della sorgente, ispezione dello schema, lettura dei
  metadata di formato. Il permit dell'input viene consumato qui.
- Al ritorno, il `Dataset` conserva **solo**:
  - `layers: Vec<Layer>` (metadata, immutable);
  - `inspection: Inspection` (immutable);
  - `footprint: SourceFootprint` (byte totali, entry visitate);
  - un handle read-only alla sorgente (es. file handle chiuso o
    ri-apribile).
- Il `PipelineBudget` iniziale e' **droppato** al ritorno di
  `open`. Il context, i contatori e la deadline della fase di
  apertura non sopravvivono.

**`Dataset.scan(request, ReadOptions) -> DatasetReader`** (nuovo
nome per l'attuale `open_layer_reader`)
- Richiede un **nuovo `ReadOptions`** costruito da un **nuovo
  `PipelineBudget`**. Il consumer fornisce esplicitamente
  quel budget: deadline, cancellazione, contatori sono freschi.
- Il `DatasetReader` restituito ha un proprio `OperationBudget`
  con contatori cumulativi che si azzerano per quella
  scansione.
- Corollario: `--max-rows 1000` applicato a una prima scansione
  di 600 righe **non** vincola una seconda scansione dello
  stesso Dataset, se il consumer costruisce un secondo
  `PipelineBudget` per essa. Il consumer che vuole cumulare
  costruisce un solo budget e lo passa a piu' scan
  successivi (raro, richiede un'API dedicata di orchestrazione
  fuori scope Lotto 0).
- `Dataset.footprint()` restituisce la `SourceFootprint`
  memorizzata: il suo `snapshot()` entra nelle `ScanBudgetParts`
  come valore **atteso**, non come valore riusato. I
  `total_bytes`/`entries_visited` che il nuovo context osserva via
  `observe_input(permit)?` sono quelli
  **ricalcolati** dal preflight leggero descritto sotto (il permit
  e' quello del nuovo bundle). Il riuso evita l'**ispezione dei
  contenuti** — parsing, schema, metadata di formato — **non**
  l'enumerazione della sorgente, che `scan()` rifa sempre.

**`convert(source, destination, ReadOptions, WriteOptions) -> Published`**
- **Unico caso** in cui reader e writer condividono un
  `PipelineContext`. Il consumer costruisce **un solo**
  `PipelineBundle`, ne fa `into_convert_parts()`, e con
  `into_parts()` ottiene la coppia `(ReadBudgetParts,
  WriteBudgetParts)` da passare rispettivamente a
  `ReadOptions::builder(...)` e `WriteOptions::builder(...)`.
- Il context condiviso porta: deadline unica per l'intero
  convert, cancellazione unica, `SourceFootprint` osservata
  una volta e visibile al writer per l'expansion ratio.
- Al termine (Ok o Err), tutti i budget sono droppati.

**Semantica di concorrenza**
- `Dataset` (e il suo `OpenDatasetHandle`): `Send + Sync`, letto
  solo (metadata + footprint immutabili). Non ha gauge di
  concorrenza propri: il tracking dei reader vivi spetta al
  `ResourcePool` opzionale del budget della scansione.
- Il `DatasetReader` restituito da `scan()` e' `Send`, **non
  Sync**: stato mutabile interno.
- Due `DatasetReader` distinti dello stesso Dataset con budget
  freschi possono girare su thread diversi se il driver dichiara
  `MultipleIndependentReaders`; se i due `PipelineBudget` sono
  agganciati allo stesso `ResourcePool`, competono sui gauge
  condivisi (memory/spill/concurrency).
- `Writer` interno di `convert`: `Send`, non `Sync`. Non
  esposto pubblicamente.

**Revalidation della `SourceFootprint`**: `Dataset.scan()`
richiede un `SourceFootprintSnapshot` in `ScanBudgetParts`. Prima
di ogni scansione il core esegue **sempre** un preflight leggero.
E' leggero perche' **non apre ne' parsifica i contenuti**, non
perche' salti l'enumerazione della sorgente. Nell'ordine:

1. enumera **tutte** le entry correnti (directory walk completo
   per le sorgenti multi-file);
2. applica `max_input_entries` via `note_entry_visited(entry)` (INV-9)
   durante l'enumerazione, prima di sommare i byte;
3. legge `size` + `mtime` di ogni entry e ricalcola il
   `SourceDigest`, che copre l'insieme dei path normalizzati:
   rileva quindi anche **aggiunte e rimozioni** di entry, non solo
   le mutazioni in-place;
4. confronta il digest ricalcolato con quello atteso; se diverge,
   `scan()` fallisce con
   `PlenoraIoError::Contract(FootprintChanged)` e nulla viene
   osservato;
5. osserva i valori **correnti** con
   `observe_input(permit)?`: il context della
   scansione porta byte ed entry di adesso, non quelli copiati
   dallo snapshot.

**Garanzia onesta**: la revalidation e' **best-effort per
costruzione**, e il pacchetto non promette altro. Size + mtime non
e' uno snapshot forte: una mutazione concorrente che preservi size
e mtime (o un filesystem a granularita' mtime grossolana) non
viene rilevata. La revalidation riduce la finestra di rischio, non
la elimina. Lotto 0 **non** ratifica alcuna variante forte
(content hashing, file identity con handle tenuto aperto,
locking): sarebbero decisioni separate, con costo di I/O e
divergenza di piattaforma propri, fuori da questo pacchetto.

**Tabella riassuntiva scope**:

| Entita' | Parti dal model | Ciclo | Contatori | Cancellazione |
|---|---|---|---|---|
| `open()` | `ReadBudgetParts` (permit consumato dal preflight) | one-shot | budget di open | del budget di open |
| `Dataset` | **nessuno vivo**; solo metadata + `SourceFootprint` | multi-call read-only | — | — |
| `Dataset.scan()` | `ScanBudgetParts` (permit + snapshot atteso) | one-shot per scansione | contatori freschi | del budget della scansione |
| `convert()` | `ConvertBudgetParts` (permit + parti read/write) | one-shot | separati read/write, context condiviso | unica per l'intero convert |
| `create()` write standalone | `WriteBudgetParts` (nessun permit) | one-shot | budget del writer | del budget del writer |

### Lifecycle memoria (INV-5)

Stato:

```
    free_bytes = M
    ┌────────────────────────────────────────────────────────────┐
    │ adapter lease(k)     → InternalMemoryLease(k)              │
    │                       free_bytes -= k                      │
    │ adapter transfer(rb) → drop(InternalMemoryLease(k))        │
    │                       free_bytes += k                      │
    └────────────────────────────────────────────────────────────┘
```

Regole (unica semantica, INV-5):
- Un adapter interno prenota `InternalMemoryLease(k)` prima di
  allocare fisicamente un buffer di `k` bytes.
- La `InternalMemoryLease` e' droppata dall'adapter stesso al
  momento in cui trasferisce il batch al consumer (uscita di
  `next_batch()`), oppure quando il buffer interno viene liberato
  senza consegna (errore prima del transfer).
- Nessuna `commit(k)` per la memoria: il modello e' gauge, non
  cumulativo.
- `commit(k)` resta appropriato per `Rows`, `OutputBytes`,
  `GeometryComponents`: contatori cumulativi che decrescono
  monotonamente. Per quelli si usa `CountedLease`.

### Descriptor: tre assi ortogonali (INV-7)

```rust
/// Cosa fa il parser grezzo del driver.
#[non_exhaustive]
pub enum NativeReadMode {
    StreamingSequential,   // one-pass, emette batch in ordine
    StreamingRandom,       // seek supportato (es. Parquet row group)
    Materialized,          // carica tutto in RAM prima di emettere
}

/// Cosa il consumer osserva a livello di contratto pubblico.
/// Descrive *quando* il primo batch e' visibile e *cosa* succede in
/// caso di errore dopo la consegna.
#[non_exhaustive]
pub enum DeliverySemantics {
    /// Nessun prefisso "accepted" viene consegnato se una
    /// violazione emerge anywhere nella sorgente.
    OperationAtomic,       // default post-Lotto-0
    /// Batch consegnati appena disponibili; errore dopo il primo
    /// batch produce `PlenoraIoError` con `RemoteEffect::Partial`
    /// e categoria `TerminatedAfterAcceptedBatches` (nuova
    /// categoria, richiede bump wire — NON ratificata nel
    /// Lotto 0).
    Streaming,
}

/// Come l'implementazione bounda la memoria interna.
/// Ortogonale alla semantica di consegna: e' un dettaglio
/// implementativo utile per capire l'impronta risorse.
#[non_exhaustive]
pub enum BufferingStrategy {
    /// Nessun buffer interno oltre il batch corrente.
    Passthrough,
    /// Buffer in RAM bounded da `memory_bytes` del PipelineContext.
    InMemoryBounded,
    /// Buffer adattivo: resta in RAM finche' l'occupato sta sotto
    /// `adaptive_memory_threshold` (derivata da `memory_bytes` del
    /// PipelineContext), poi migra su file temporaneo (bounded da
    /// `spill_bytes`). Una volta migrato non torna in RAM. Picco
    /// di RAM = `adaptive_memory_threshold + current_batch`,
    /// **indipendente dalla dimensione totale dell'input**.
    /// Strategia ratificata post-ADR-IO 7.
    AdaptiveMemoryThenDisk,
}
```

Un descrittore dichiara la **tripla** `(native_read_mode,
effective_delivery, buffering)`. Ratificato in questo pacchetto
per l'adapter comune post-ADR-IO 7 opzione A, e **dichiarato dai
driver in M5**, quando i tre campi entrano insieme nel descriptor
(il comportamento e' gia' quello da M2):

- `effective_delivery = OperationAtomic`
- `buffering = AdaptiveMemoryThenDisk`

Combinazioni declared-only (non implementate in Lotto 0):
- `Streaming + Passthrough`: passa direttamente i batch del
  driver nativo. Richiede la nuova categoria d'errore.
- `Streaming + InMemoryBounded`: streaming con throttling. Come
  sopra.
- `OperationAtomic + InMemoryBounded`: comportamento pre-Lotto-0
  (VecDeque). Non piu' selezionabile dopo M2.

**Corollario per il finding L0.4**: il descrittore attuale con
solo `read_mode: ReadMode` conflava questi tre assi in un solo
valore. Un consumer che vede `ReadMode::StreamingSequential` non
poteva sapere se il consumo effettivo era streaming, spooled o
in-memory. La tripla lo esplicita.

## ADR-IO 7 — raccomandazione (opzione A, stato Draft fino a S0)

**Stato dell'ADR**: **Draft**. Il file
`docs/adr/ADR-IO-7-*.md` resta in stato Draft finche' lo step
S0 del Lotto 0 non lo promuove a Accepted come parte della
ratifica di governance. Nessuna riscrittura dell'ADR e'
associata a questo documento.

**Raccomandazione: opzione A (spool bounded)**. Motivazioni:

- **Contratto invariato**: `effective_delivery = OperationAtomic`
  resta il default e il comportamento osservabile per il consumer
  attuale.
- **Memoria bounded**: `BudgetedReader` ha sostituito la `VecDeque`
  con `StagedSpool` (attuato in S2), che scrive Arrow IPC su un file
  temporaneo **senza nome** e ne addebita i byte realmente scritti
  alla quota di spill con lease RAII.
- **Nessuna rottura cross-component**: writer e aggregatori
  esistenti (CLI `convert`, futura data-tools chain) non
  cambiano interfaccia.
- **INV-5 rispettato**: il picco di RAM e' **bounded da
  `adaptive_memory_threshold + current_batch`** — la soglia
  adattiva piu' il solo batch in transito — ed e' **indipendente
  dalla dimensione totale** dell'input: oltre la soglia i batch
  gia' validati vivono sul file di spool, non in RAM. Non e'
  `O(batch_target)`: il bound non scala col numero di batch
  bufferizzati ne' con la dimensione del dataset.

### File di spool: politica di sicurezza (attuata in S2)

Un file di spool contiene i batch validati che passeranno al
writer. E' un canale sensibile: se un altro utente del filesystem
puo' leggerlo, vede il payload post-parsing dell'input; se puo'
scriverlo, puo' iniettare batch arbitrari nella scrittura.

**Il file non ha un nome.** E' creato con `tempfile::tempfile_in`,
che su Unix lo scollega dal filesystem appena aperto e su Windows
lo apre con `FILE_FLAG_DELETE_ON_CLOSE`. Ne discende tutto il
resto:

1. **Nessun altro processo puo' aprirlo**, perche' non esiste un
   path da aprire. La riservatezza non dipende da permessi che il
   filesystem potrebbe non rispettare, ne' da un nome non
   predicibile.
2. **Nessuna finestra TOCTOU** fra creazione e apertura, e nessun
   symlink o reparse point da seguire: non c'e' una seconda
   risoluzione del path.
3. **Nessun orfano da spazzare**, nemmeno dopo un `SIGKILL` o un
   crollo dell'alimentazione: il kernel libera l'inode alla
   chiusura del descrittore. Non serve un `LOCK` per-directory, ne'
   uno sweep all'avvio, ne' la distinzione fra directory vive e
   morte — cioe' spariscono tutti i casi limite che quel
   meccanismo avrebbe dovuto gestire correttamente (PID recycling,
   clock skew, filesystem senza `flock` affidabile, race fra due
   processi che spazzano insieme).
4. **Rilascio anticipato**: a fine rilettura il descrittore viene
   chiuso subito, senza aspettare il drop dello spool. Lo spazio
   torna al volume mentre il consumer sta ancora lavorando sui
   batch ricevuti.

**Configurabilita' del path base**: `PLENORA_SPILL_DIR` sceglie la
directory che ospita l'inode — un tmpfs, un SSD dedicato, un mount
criptato sono tutte scelte legittime. La directory di spool **non
deve** stare sullo stesso filesystem della destinazione del writer:
il replay passa dal filesystem in user space, non da rename
atomici, quindi `EXDEV` non si applica. Se la variabile e'
impostata ma non utilizzabile la creazione **fallisce chiuso**: un
ripiego silenzioso metterebbe i dati su un volume che l'operatore
non ha scelto.

**Quota di spill applicata alle scritture fisiche**: il writer che
avvolge il file prenota la quota **prima di ogni `write` verso il
disco**, sui byte che stanno per essere consegnati. Applicarla piu'
in alto, attorno alla scrittura del batch, la applicherebbe a una
stima — e i byte bufferizzati raggiungerebbero il volume prima che
qualcuno li conti. Le prenotazioni sono RAII e a blocchi, con
ripiego sull'importo esatto quando la quota configurata e' piu'
piccola del blocco: altrimenti un tetto piccolo verrebbe di fatto
arrotondato per eccesso.

**Test dedicati**: `an_unusable_spill_dir_fails_closed_instead_of_falling_back`;
`a_spill_dir_that_is_a_file_is_rejected`;
`an_underestimated_batch_cannot_write_beyond_the_quota`;
`a_quota_smaller_than_the_reservation_chunk_is_usable`;
`reaching_eof_releases_file_and_quota_while_the_spool_is_still_alive`;
`spill_quota_returns_to_the_budget_when_the_spool_is_dropped`.

### Alternative considerate

- **Opzione B (streaming con errore terminale)**: richiede la
  nuova categoria `TerminatedAfterAcceptedBatches` in
  `plenora-io-error-v1`, coordinamento con data-tools e
  database-tools per la semantica di rollback lato consumer. Costo
  cross-component elevato; nessun beneficio finche' non emerge un
  caso d'uso streaming-aware documentato. **Fuori scope Lotto 0**.
- **Opzione C (ibrido opt-in)**: duplica il codice della A e della
  B; utile solo se entrambe hanno un utenza documentata. Oggi solo
  A ce l'ha. **Rimandata**.

## Redazione errore strutturale (INV-10) + DTO conforme v1

**Strategia**: eliminare il canale libero `message: String` dalla
API Rust pubblica, sostituendolo con un enum tipizzato
`PublicMessage`. La serializzazione JSON verso l'envelope
`plenora-io-error-v1` e' fatta da un **DTO dedicato**
(`PublicErrorDto`) che preserva la **struttura wire invariata** del
contratto v1 esistente; solo il testo di `message` e'
intenzionalmente diverso.

**Nessuna esposizione di FNV nel wire pubblico**. Ragionamento:
un `Fingerprint(u64)` derivato dal payload utente e' un canale
covert — piccolo, ma reale. Un consumer che vede lo stesso
fingerprint su input diversi puo' dedurre uguaglianza; un
attaccante che controlla l'input puo' cercare collisioni. Il
Fingerprint non fornisce sicurezza (non e' one-way rispetto a
input controllato) e leaka struttura. Rimosso da tutti i tipi
pubblici e dal wire. Se serve un identificatore stabile per un
errore ricorrente, il chiamante usa i quattro assi
`(category, phase, code, kind)` che sono deterministici e non
derivati da payload.

### Boundary di ingresso per `PublicMessage`

| Consentito | Perche' |
|---|---|
| `&'static str` | compile-time, non user-controlled |
| Enum del workspace (`ErrorCategory`, `IoErrorCode`, `GeometryType`, `WkbFailureKind`, ...) | valori curati |
| `LayerId`, `FieldId`, `RowIndex` | indici numerici, non user-controlled |

`ContractIdentifier` **non compare in questa tabella**: non e' un
ingresso di `PublicMessage`. Vive **solo** nell'`ErrorContext`
strutturato, e' costruito unicamente dai getter del contratto gia'
validato (`Schema::field_by_index`, `Layer::name`), e da li' il DTO
deriva il campo `field` del wire. I nomi dello schema sono gia'
esposti dagli envelope `inspect`/`layers`, quindi non aggiungono
canale nuovo.

### Boundary di rifiuto (impedito dal compilatore)

| Vietato | Motivo |
|---|---|
| `String` da valori di cella | payload derivato dall'input |
| Percorsi assoluti | path fs sensibile |
| WKB bytes / rappresentazioni testuali | payload geometrico |
| CRS raw definitions (WKT/PROJJSON dell'utente) | payload |
| Messaggi di errore da dipendenze C (es. GDAL) | possono contenere path/valori |
| `Fingerprint` o hash di input | canale covert |

### Struttura Rust pubblica

```rust
#[non_exhaustive]
pub struct PlenoraIoError {
    // Campi privati con getter: l'invariante di redazione
    // impedisce mutazione arbitraria dopo la costruzione.
    kind: ErrorKind,
    message: PublicMessage,
    context: ErrorContext,
    // Assi 4 preservati esattamente dal contratto v1.
    category: ErrorCategory,
    phase: ErrorPhase,
    remote_effect: RemoteEffect,
    retry: RetryDisposition,
    code: IoErrorCode,
    row_diagnostics: Option<Box<RowDiagnostics>>,
}

impl PlenoraIoError {
    pub fn kind(&self) -> &ErrorKind;
    pub fn message(&self) -> &PublicMessage;
    pub fn context(&self) -> &ErrorContext;
    pub fn category(&self) -> ErrorCategory;
    pub fn phase(&self) -> ErrorPhase;
    pub fn remote_effect(&self) -> RemoteEffect;
    pub fn retry(&self) -> RetryDisposition;
    pub fn code(&self) -> IoErrorCode;
    pub fn row_diagnostics(&self) -> Option<&RowDiagnostics>;

    // Solo builder tipizzati, nessun costruttore che accetta
    // String libera.
    pub fn contract_violation(kind: ContractViolationKind) -> Self;
    pub fn limit_exceeded(kind: LimitKind) -> Self;
    pub fn wkb(kind: WkbFailureKind) -> Self;
    pub fn cancelled() -> Self;
    // ...
}

#[non_exhaustive]
pub enum ErrorKind {
    ContractViolation(ContractViolationKind),
    LimitExceeded(LimitKind),
    Capability(CapabilityReason),
    CrsUnresolved,
    Wkb(WkbFailureKind),
    Cancelled,
    OutputExists,
    Io(IoErrorKind),
}

#[non_exhaustive]
pub enum PublicMessage {
    Curated(&'static str),
    ContractViolation(ContractViolationKind),
    LimitExceeded(LimitKind),
    // NON esistono varianti "WithContractIdentifier",
    // "WithFingerprint" o "WithArbitraryString": l'eventuale
    // identificatore del contratto vive nell'`ErrorContext`
    // strutturato, non nel messaggio.
}

pub struct ContractIdentifier { /* opaque */ }
impl ContractIdentifier {
    // Costruzione solo tramite un tipo del contratto gia' validato:
    pub fn from_layer(layer: &Layer) -> Self;
    pub fn from_field(schema: &Schema, index: FieldId) -> Self;
    // Nessun `from_string` pubblico.
}

pub struct ErrorContext { /* opaque */ }
impl ErrorContext {
    pub fn driver(&self) -> Option<&'static str>;
    pub fn field(&self) -> Option<FieldId>;
    pub fn layer(&self) -> Option<LayerId>;
    /// Identificatore del contratto validato (nome campo/layer),
    /// safe-by-construction. Il DTO wire ne deriva direttamente il
    /// campo `field`. `None` se l'errore non e' legato a un
    /// identificatore di schema.
    pub fn contract_identifier(&self) -> Option<&ContractIdentifier>;
    pub fn builder() -> ErrorContextBuilder;
}
```

### DTO di serializzazione conforme a `plenora-io-error-v1`

L'envelope wire e' congelato e non cambia. La struttura Rust
sopra e' interna; per emettere il JSON compatibile viene
introdotto un DTO privato:

```rust
// Modulo privato in plenora-io-model, esposto solo tramite
// `impl Serialize for PlenoraIoError` che passa da qui.
#[derive(Serialize)]
struct PublicErrorDto<'a> {
    // Campi identici a plenora-io-error-v1, ordinati e tipati
    // esattamente come oggi.
    category: ErrorCategory,
    phase: ErrorPhase,
    remote_effect: RemoteEffect,
    retry: RetryDisposition,
    code: IoErrorCode,
    /// String prodotta dal rendering di PublicMessage.
    /// L'implementazione garantisce che due chiamate sullo stesso
    /// PublicMessage producano la stessa stringa (determinismo).
    message: String,
    // Campi opzionali del v1 preservati:
    driver: Option<&'a str>,
    field: Option<&'a str>,
    capability_reason: Option<&'a CapabilityReason>,
    row_diagnostics: Option<&'a RowDiagnostics>,
}
```

**Due invarianti distinti sul DTO**:

**Invariante A — struttura wire invariata** (campi, ordine, tipi; `message` a parte)**:**
- Elenco campi JSON, ordine, tipi: identici a quelli emessi
  dalla codebase pre-Lotto-0.
- Valori dei quattro assi: nessuna nuova variante di enum,
  nessuna rimossa; serializzazione `snake_case` invariata.
- Campo `driver`: `Option<String>` (o `Option<&str>`
  serializzato uguale), stesso set di valori gia' emessi dai
  driver.
- Campo `field`: **preservato come nome di campo (`String`)** nel
  wire, per compatibilita' con consumer che lo leggono. Il tipo
  Rust interno usa `FieldId` per costruzione sicura; il DTO fa
  la traduzione da `FieldId` al nome tramite lo schema del layer
  disponibile in fase di rendering. Il nome del campo NON e' un
  valore ostile: e' un identificatore del contratto gia' esposto
  in `inspect`/`layers`. **Questo campo e' esplicitamente
  consentito nel wire** anche se costruito come `String`, perche'
  la sua forma e' vincolata dal contratto del layer.
- Campo `capability_reason`: enum invariato.
- Campo `row_diagnostics`: sottoschema
  `plenora-row-diagnostics-v1` invariato.

Un test di snapshot
`error_envelope_v1_structure_conforms_to_baseline` confronta i
JSON generati dal codice pre-Lotto-0 (fixture baseline) con
quelli post-Lotto-0 rigenerati dal nuovo modello sugli stessi
casi, **normalizzando via il campo `message`** (`s/"message":
".*?"/"message": "<curated>"/` prima del confronto). Fallisce su
qualunque differenza di presenza/ordine/tipo di campo o valore
di enum diverso da `message`. Il testo di `message` e' coperto
dal test B, non da questo.

**`ContractIdentifier` conservato nel contesto strutturato per la
serializzazione di `field`**: il campo `field: Option<String>`
del wire v1 deriva **direttamente** dall'`Option<ContractIdentifier>`
esposto da `ErrorContext::contract_identifier()`, non da una
variante del messaggio. Il tipo `ContractIdentifier` resta nel
modello proprio per garantire questa derivazione: se lo si
rimuovesse, il campo `field` del wire non sarebbe piu' producibile
senza allocare stringhe arbitrarie in violazione di INV-10. Il DTO
usa `ContractIdentifier::as_wire_field()` per ottenere
l'`Option<&str>` da serializzare.

**Invariante B — testo del campo `message` intenzionalmente
cambiato**:
- Il campo `message` resta `String` nel JSON (Invariante A),
  ma il testo prodotto **cambia deliberatamente** rispetto al
  pre-Lotto-0: rendering curato di `PublicMessage` invece di
  `format!` libero.
- Un consumer che grep-fa il testo per estrarre valori
  (pattern esistente ma sconsigliato) va aggiornato.
- Il test snapshot su questo campo e' **separato**
  (`error_message_text_matches_curated_rendering_baseline`) e
  cambia con Lotto 0: la sua baseline viene rigenerata al
  merge di S9 (errori strutturati). L'invariante A resta verde
  attraverso il cambio; l'invariante B viene aggiornato.

Le due invarianti sono verificate da suite separate. Il
cambio del testo di `message` non puo' mascherare una
regressione della struttura del wire e viceversa.

### Migrazione dei call site attuali

Tocca 13 file di driver + core. Ogni `format!("... {value}")`
diventa:
- se il valore era un identificatore del contratto (nome
  campo/layer): `PlenoraIoError::contract_violation(kind).with_id(id)`;
- se il valore era un payload utente: il messaggio omette il
  valore; la variante di `kind` racchiude il contesto tipizzato
  (es. `ContractViolationKind::UnexpectedGeoJsonType { expected }`
  senza allegare il valore osservato);
- se il valore era ridondante (deducibile dagli enum): non
  serviva includerlo.

**Riguardo al finding L0.6 originario**: la formulazione precedente
proponeva un helper `redact_value`. Questo pacchetto lo rigetta
come strategia. La correzione strutturale (rimozione di
`message: String` dalla API pubblica, DTO dedicato per il wire,
niente Fingerprint pubblico) e' quello che il compilatore e il
serializzatore impongono insieme.

Il dettaglio d'implementazione (elenco esaustivo delle varianti,
sostituzione dei call site attuali, migrazione dei test snapshot)
resta scope implementativo L0.6, non decisionale.

## API proposte

Firme illustrative. Nomi e forme finali si decidono al Lotto 0
d'implementazione.

### Modulo `plenora_io_model::budget` (nuovo, sostituisce `limits.rs` e `resource.rs`)

Il modello espone budget e permit. Le factory di opzioni di
lettura/scrittura (`ReadOptions`, `WriteOptions`) vivono nel
core, non qui: il modello non conosce driver ne' format
options.

```rust
/// Root, non Clone (token di costruzione one-shot). Non e'
/// ottenibile separatamente dal permit: `build()` lo consegna
/// dentro un `PipelineBundle` opaco. Non espone `into_*_parts`.
pub struct PipelineBudget { /* opaque */ }

impl PipelineBudget {
    pub fn builder() -> PipelineBudgetBuilder;
    pub fn context(&self) -> &PipelineContext;
}

/// Restituito da `PipelineBudgetBuilder::build`. **Opaco**:
/// nessun campo pubblico, non Clone. Tiene insieme il budget e il
/// permit emessi dalla stessa costruzione; non esiste un modo di
/// incrociare a mano un permit con un budget diverso, perche' le
/// uniche uscite sono le `into_*_parts` qui sotto.
pub struct PipelineBundle { /* opaque */ }

impl PipelineBundle {
    pub fn context(&self) -> &PipelineContext;

    /// Consuma per un `open` (preflight completo della sorgente).
    /// Le parti trasportano il permit non ancora speso.
    pub fn into_read_parts(self) -> ReadBudgetParts;

    /// Consuma per un `scan` su un `Dataset` gia' aperto. Le parti
    /// trasportano permit + snapshot atteso; il core rivalida
    /// sempre lo snapshot con un preflight leggero prima
    /// dell'inizio della scansione, e una discrepanza tra digest
    /// atteso e digest ricalcolato produce `PlenoraIoError::
    /// Contract(FootprintChanged)`.
    pub fn into_scan_parts(self, expected: SourceFootprintSnapshot) -> ScanBudgetParts;

    /// Consuma per un `convert` (reader + writer con context
    /// condiviso). Il permit viaggia sul ramo read.
    pub fn into_convert_parts(self) -> ConvertBudgetParts;

    /// Consuma per un `write standalone`: il permit **non** entra
    /// nelle parti, viene droppato insieme al bundle.
    pub fn into_write_parts(self) -> WriteBudgetParts;
}

// Parti opache: nessun campo pubblico, non Clone. Ogni tipo
// implementa il trait sealed `IntoReadParts` o `IntoWriteParts`
// per alimentare `ReadOptions::builder` / `WriteOptions::builder`.
pub struct ReadBudgetParts { /* opaque */ }
pub struct ScanBudgetParts { /* opaque */ }
pub struct WriteBudgetParts { /* opaque */ }
pub struct ConvertBudgetParts { /* opaque */ }
impl ConvertBudgetParts {
    /// Consuma le parti convert e le divide nei due rami
    /// read/write (contatori indipendenti, stesso `PipelineContext`).
    pub fn into_parts(self) -> (ReadBudgetParts, WriteBudgetParts);
}

// Sigillo: modulo privato di plenora-io-model, non nominabile
// fuori dal crate. Nessun tipo esterno puo' implementare i due
// trait sottostanti, quindi le factory del core accettano solo
// parti prodotte da un `PipelineBundle`.
mod sealed {
    pub trait Sealed {}
    impl Sealed for super::ReadBudgetParts {}
    impl Sealed for super::ScanBudgetParts {}
    impl Sealed for super::WriteBudgetParts {}
}

/// Sealed. Implementato **solo** da `ReadBudgetParts` e
/// `ScanBudgetParts`. Il metodo e' `#[doc(hidden)]` e restituisce
/// un tipo opaco: e' il canale model→core, non un'API d'uso.
/// `ReadBudgetParts` e' la rappresentazione read interna; i suoi
/// campi sono privati e visibili solo dentro `plenora-io-model`.
pub trait IntoReadParts: sealed::Sealed {
    #[doc(hidden)]
    fn into_read_budget_parts(self) -> ReadBudgetParts;
}

impl IntoReadParts for ReadBudgetParts {
    /// Identita': gia' nella rappresentazione interna.
    fn into_read_budget_parts(self) -> ReadBudgetParts { self }
}

impl IntoReadParts for ScanBudgetParts {
    /// Conversione, non identita': le parti scan portano in piu' lo
    /// snapshot atteso. Budget e permit passano **invariati** (nessun
    /// contatore ricreato, nessun permit rigenerato); lo snapshot
    /// finisce nel campo privato `expected`, da cui il core lo legge
    /// con `ReadOptions::expected_footprint()`.
    fn into_read_budget_parts(self) -> ReadBudgetParts {
        let ScanBudgetParts { budget, permit, expected } = self;
        ReadBudgetParts { budget, permit, expected: Some(expected) }
    }
}
// Corollario: `ReadOptions::expected_footprint()` e' `Some` se e solo
// se le opzioni derivano da `ScanBudgetParts`; `None` sul percorso
// `open`, che non ha nulla da rivalidare.

/// Sealed. Implementato **solo** da `WriteBudgetParts` (il ramo
/// write di un convert e' gia' un `WriteBudgetParts`).
pub trait IntoWriteParts: sealed::Sealed {
    #[doc(hidden)]
    fn into_write_budget_parts(self) -> WriteBudgetParts;
}

impl IntoWriteParts for WriteBudgetParts {
    /// Identita'.
    fn into_write_budget_parts(self) -> WriteBudgetParts { self }
}

pub struct PipelineBudgetBuilder { /* opaque */ }
impl PipelineBudgetBuilder {
    pub fn limits(self, limits: PipelineLimits) -> Self;
    pub fn cancellation(self, token: CancellationToken) -> Self;
    /// Facoltativo. Se presente, memory/spill del `PipelineContext`
    /// contano contro **sia** la quota locale **sia** i gauge del
    /// pool (quota effettiva = minimo dei due); la concorrenza e'
    /// governata **solo** dal pool. Senza pool, memory/spill sono
    /// locali e la concorrenza e' un no-op illimitato (INV-12).
    pub fn resource_pool(self, pool: ResourcePool) -> Self;
    pub fn build(self) -> Result<PipelineBundle, PlenoraIoError>;
}

/// Handle condivisibile a un insieme di gauge (memoria, spill,
/// concorrenza) da agganciare a piu' `PipelineBudget` che devono
/// stare sotto una stessa quota. `Clone` via Arc.
pub struct ResourcePool { /* opaque */ }
impl ResourcePool {
    pub fn builder() -> ResourcePoolBuilder;
}

pub struct ResourcePoolBuilder { /* opaque */ }
impl ResourcePoolBuilder {
    pub fn memory_bytes(self, v: u64) -> Self;
    pub fn spill_bytes(self, v: u64) -> Self;
    pub fn concurrent_operations(self, v: u64) -> Self;
    pub fn build(self) -> Result<ResourcePool, PlenoraIoError>;
}

#[non_exhaustive]
pub struct PipelineLimits { /* campi privati */ }
impl PipelineLimits {
    pub fn default() -> Self;
    // fluent setters per ogni quota, evitano struct literal
    pub fn with_max_rows(self, v: u64) -> Self;
    pub fn with_max_input_entries(self, v: u64) -> Self;   // INV-9
    pub fn with_max_wkb_cell_bytes(self, v: usize) -> Self;
    pub fn with_max_vertices(self, v: usize) -> Self;
    // ... uno setter per ogni quota
    /// Tetto effettivo per singola geometria: il minimo fra il
    /// limite per cella e il tetto globale dei vertici. E' la
    /// stessa composizione di `Limits::effective_wkb()`.
    pub fn effective_wkb_components(&self) -> usize;
    // getter espliciti per lettura:
    pub fn max_rows(&self) -> u64;
    pub fn max_input_entries(&self) -> u64;
    // ...
}

/// Arc-shared, Send + Sync.
pub struct PipelineContext { /* opaque */ }
impl PipelineContext {
    pub fn deadline(&self) -> Instant;
    pub fn ensure_active(&self) -> Result<(), PlenoraIoError>;
    pub fn cancellation(&self) -> &CancellationToken;                // INV-4
    pub fn observed_input(&self) -> ObservedInput;                    // INV-6

    /// Osserva l'input: consuma il `permit` (emesso dal bundle di
    /// questo stesso context) e registra il footprint. Unica
    /// fabbrica di `SourceFootprint` e unico canale di
    /// registrazione (INV-13). One-shot per costruzione: il permit
    /// e' `move` e non `Clone`, quindi un secondo `observe_input`
    /// e' impossibile.
    /// `Err` se il permit appartiene a un altro context, se
    /// `ensure_active()` fallisce (cancellazione o deadline
    /// scaduta), o se un'osservazione risulta gia' registrata su
    /// questo context. **Un errore non modifica lo stato
    /// precedente**: la chiamata o pubblica, o non lascia traccia.
    pub fn observe_input(&self, permit: InputPermit)
        -> Result<SourceFootprint, PlenoraIoError>;

    // Gauge lease-based, restituiti dai Drop delle lease (INV-5).
    // memory/spill: quota effettiva = min(PipelineLimits, ResourcePool);
    // la lease consuma entrambi i gauge quando il pool e' presente (INV-12).
    pub fn lease_memory_internal(&self, bytes: u64)
        -> Result<InternalMemoryLease, PlenoraIoError>;
    pub fn lease_spill(&self, bytes: u64)
        -> Result<SpillLease, PlenoraIoError>;
    /// No-op senza `ResourcePool` (restituisce sempre una lease);
    /// conta sui gauge del pool quando presente (INV-12).
    pub fn lease_concurrency(&self)
        -> Result<ConcurrencyLease, PlenoraIoError>;

    pub fn note_entry_visited(&self, entry: &SourceEntry<'_>)
        -> Result<(), PlenoraIoError>;                                // INV-9
}

/// Stato dell'input osservato. INV-6.
#[non_exhaustive]
pub enum ObservedInput {
    NotObserved,
    Bytes(u64),
}

/// Permit opaco one-shot. Non Clone, senza costruttori pubblici;
/// emesso dal `PipelineBudgetBuilder::build` dentro il
/// `PipelineBundle` opaco, da cui esce solo trasportato dalle
/// parti (`ReadBudgetParts`, `ScanBudgetParts`, ramo read di
/// `ConvertBudgetParts`).
pub struct InputPermit { /* opaque */ }

/// Descrizione immutabile dell'input osservato dal preflight.
/// Non ha costruttori pubblici diretti: si ottiene solo
/// consumando un `InputPermit` via `PipelineContext::observe_input`.
#[non_exhaustive]
pub struct SourceFootprint { /* opaque */ }
impl SourceFootprint {
    pub fn total_bytes(&self) -> u64;
    pub fn entries_visited(&self) -> u64;
    /// Snapshot serializzabile del footprint; il consumer lo
    /// conserva come valore **atteso** per un successivo `scan()`
    /// ed evita cosi' l'ispezione dei contenuti, non
    /// l'enumerazione della sorgente (vedi "Revalidation").
    pub fn snapshot(&self) -> SourceFootprintSnapshot;
}

/// Snapshot serializzabile. Puo' viaggiare fuori dal processo.
#[derive(Clone)]
pub struct SourceFootprintSnapshot { /* opaque, serializable */ }
impl SourceFootprintSnapshot {
    pub fn total_bytes(&self) -> u64;
    pub fn entries_visited(&self) -> u64;
    /// Digest **best-effort** dell'insieme di file osservati (path
    /// normalizzati + size + mtime), accumulato entry per entry
    /// durante l'enumerazione. Rileva le mutazioni comuni ma
    /// **non** quelle che preservano size+mtime: il limite e'
    /// dichiarato, non mitigato (vedi "Revalidation").
    pub fn digest(&self) -> SourceDigest;
    /// Confronto di revalidation: byte, entry e digest **insieme**.
    pub fn matches(&self, observed: &SourceFootprintSnapshot) -> bool;
}

/// Identita' di una entry, fornita dal core a `note_entry_visited`.
/// Il path arriva **gia' normalizzato**: la normalizzazione dipende
/// da filesystem e piattaforma, che il modello non conosce.
///
/// I due valori in byte sono distinti e non intercambiabili:
/// `metadata_size` entra nel **digest** ed e' cio' che rende
/// rilevabile una mutazione in place; `charged_input_bytes` conta
/// verso **`max_input_bytes`** ed e' cio' che il bordo si impegna a
/// leggere. Per una directory sono entrambi zero: non c'e'
/// contenuto da leggere, e la dimensione riportata di una directory
/// e' un artefatto del filesystem le cui voci sono gia' nel digest
/// una per una. Le due costruzioni sono metodi distinti perche' la
/// regola resti strutturale e non una convenzione del chiamante.
pub struct SourceEntry<'a> { /* opaque */ }
impl<'a> SourceEntry<'a> {
    pub const fn file(
        path_identity_bytes: &'a [u8],
        size_bytes: u64,
        modified: Option<SystemTime>,
    ) -> Self;
    pub const fn directory(
        path_identity_bytes: &'a [u8],
        modified: Option<SystemTime>,
    ) -> Self;
    pub const fn metadata_size(&self) -> u64;
    pub const fn charged_input_bytes(&self) -> u64;
    pub const fn is_directory(&self) -> bool;
}

/// Digest opaco a 128 bit sull'insieme dei path normalizzati con
/// size + mtime: copre quindi anche aggiunte e rimozioni di entry.
///
/// Accumulato **entry per entry** dentro il `PipelineContext` da
/// `note_entry_visited`, non passato dall'esterno: il valore che la
/// revalidation confronta non deve essere fabbricabile dal
/// chiamante. La combinazione e' **insensibile all'ordine** (XOR
/// dei valori per-entry), perche' l'ordine di enumerazione di una
/// directory non e' stabile fra due scansioni ne' fra due
/// filesystem, e un digest sensibile all'ordine segnalerebbe una
/// mutazione inesistente. La codifica per-entry ha la lunghezza del
/// path in testa, cosi' due insiemi di path diversi non collassano
/// sulla stessa sequenza di byte.
///
/// La funzione e' FNV-1a a 64 bit applicata due volte con basi
/// distinte: non e' crittografica e non deve esserlo — chi puo'
/// riscrivere i file puo' comunque cambiarne il contenuto a
/// dimensione e mtime invariati, che e' esattamente il limite
/// dichiarato dalla garanzia best-effort. Evita inoltre una
/// dipendenza nuova, che nel workspace passa da un gate di pin.
///
/// Rivalidazione = riesecuzione del preflight leggero
/// (enumerazione completa + size/mtime, **nessun** read dei
/// contenuti), un permit consumato, un digest ricalcolato.
/// Confronto uguaglianza obbligatorio prima del riuso.
pub struct SourceDigest([u8; 16]);

/// `OperationBudget` e' un tipo pubblico ma **opaco** (nessun
/// campo pubblico, nessun costruttore pubblico): serve al boundary
/// model→core perche' i driver operino sui contatori, ma **non** e'
/// ri-esportato dalla facade `plenora-io-api`. I driver lo
/// ottengono tramite le parti opache (`ReadBudgetParts`,
/// `WriteBudgetParts`, ecc.) fornite da `ReadOptions`/`WriteOptions`.
/// Clone via Arc; Send + Sync.
pub struct OperationBudget { /* opaque */ }
impl OperationBudget {
    pub fn context(&self) -> &PipelineContext;
    pub fn try_lease(&self, kind: OperationCounter, amount: u64)
        -> Result<CountedLease, PlenoraIoError>;
    /// INV-6: applica expansion_ratio SOLO se ObservedInput::Bytes(n > 0).
    pub fn output_limit(&self) -> u64;
}

/// Lease della memoria che la libreria detiene *internamente*
/// (buffer batch worker, coda spool, staging writer). Send, non Sync.
/// INV-5: rilasciata dagli adapter interni al momento del transfer
/// del batch al consumer.
pub struct InternalMemoryLease { /* RAII */ }
impl InternalMemoryLease {
    pub fn bytes(&self) -> u64;
    // Nessun commit(): gauge lease-based, rilasciata dal Drop.
    // Tipo pubblico ma opaco e workspace-internal: non viene
    // restituito insieme al batch e non e' ri-esportato dalla
    // facade `plenora-io-api`. Il consumer non ne vede mai una.
}

/// Send, non Sync.
pub struct SpillLease { /* RAII */ }
impl SpillLease {
    pub fn bytes(&self) -> u64;
    // Rilasciata dal Drop; il file di spool viene rimosso.
}

/// Send, non Sync.
pub struct ConcurrencyLease { /* RAII */ }
impl ConcurrencyLease {
    // Non ha metodi: conta solo se e' viva.
}

/// Send, non Sync.
pub struct CountedLease { /* RAII per contatori cumulativi */ }
impl CountedLease {
    pub fn commit(self, used: u64) -> Result<(), PlenoraIoError>;
    pub fn release(self) -> Result<(), PlenoraIoError>;
    // Drop senza commit/release = release automatico.
}
```

**Nota su moduli e dipendenze (INV-13)**: tutto sopra vive in
`plenora-io-model::budget`. Il `Cargo.toml` di `plenora-io-model`
**non** dichiara `plenora-io-core` fra le dipendenze. Il metodo
`PipelineContext::observe_input(permit)` consuma
il permit per costruire e registrare la `SourceFootprint`;
`Source::into_path_checked` in core lo invoca con `?` dopo aver
eseguito il proprio preflight. La protezione non e' un grep: e' il
permit opaco, non costruibile e legato al context, unito al
`PipelineBundle` opaco e ai trait sealed `IntoReadParts` /
`IntoWriteParts`, a rendere impossibile un'osservazione fabbricata
o registrata su un context diverso.

### Modulo `plenora_io_core::driver`

`ReadOptions`/`WriteOptions` diventano wrapper del budget invece
di contenere `Limits + ResourceBudget` separati (INV-1, INV-2):

`ReadOptions` e `WriteOptions` vivono in **`plenora-io-core`**
(non in model). Il modello espone solo budget e permit; le
opzioni di formato conoscono i driver.

```rust
// in plenora-io-core::driver
#[non_exhaustive]
pub struct ReadOptions { /* opaque */ }
impl ReadOptions {
    /// Costruisce da parti opache prodotte dal model
    /// (`ReadBudgetParts` o `ScanBudgetParts`). Il bound e' il
    /// trait **sealed** del model: nessun tipo esterno puo'
    /// alimentare questa factory.
    pub fn builder<P: IntoReadParts>(parts: P) -> ReadOptionsBuilder;

    /// Budget del ramo read. E' l'**unico** accesso dei driver al
    /// modello: da qui `context()` (deadline, cancellazione,
    /// lease memoria/spill/concorrenza, `note_entry_visited`) e i
    /// contatori cumulativi via `try_lease`.
    pub fn budget(&self) -> &OperationBudget;

    /// Snapshot atteso: `Some` solo se le opzioni derivano da
    /// `ScanBudgetParts`. Lo consuma il preflight leggero di
    /// `Dataset::scan`.
    pub fn expected_footprint(&self) -> Option<&SourceFootprintSnapshot>;

    /// Estrae il permit trasportato dalle parti. `None` se gia'
    /// estratto o se le parti non ne trasportavano. Il permit resta
    /// legato al context di queste stesse opzioni, quindi estrarlo
    /// non consente alcun incrocio.
    ///
    /// **`pub(crate)` e non `pub`** (errata S4.b.3): l'unico
    /// chiamante legittimo e' `Source::into_path_checked`, che vive
    /// nello stesso crate. Esporlo darebbe a un driver — o domani
    /// alla facade — un secondo punto da cui separare il permit dal
    /// proprio context.
    pub(crate) fn take_input_permit(&mut self) -> Option<InputPermit>;

    pub fn assume_crs(&self) -> Option<&str>;
    pub fn format_options(&self) -> &BTreeMap<String, String>;
}

pub struct ReadOptionsBuilder { /* opaque */ }
impl ReadOptionsBuilder {
    pub fn assume_crs(self, crs: impl Into<String>) -> Self;
    pub fn format_option(self, k: impl Into<String>, v: impl Into<String>) -> Self;
    pub fn build(self) -> ReadOptions;
}

#[non_exhaustive]
pub struct WriteOptions { /* opaque */ }
impl WriteOptions {
    /// Da `WriteBudgetParts` (standalone) o dal ramo write di
    /// `ConvertBudgetParts`. Bound **sealed** come sopra.
    pub fn builder<P: IntoWriteParts>(parts: P) -> WriteOptionsBuilder;

    /// Budget del ramo write: contatori cumulativi indipendenti da
    /// quelli del read, stesso `PipelineContext` in un convert.
    /// Nessun `take_input_permit`: il ramo write non osserva input.
    pub fn budget(&self) -> &OperationBudget;

    pub fn durable(&self) -> bool;
    pub fn format_options(&self) -> &BTreeMap<String, String>;
}
```

`Default` **non e' implementato** ne' per `ReadOptions` ne' per
`WriteOptions`: non esiste un budget di default sensato. Il
consumer parte sempre da `PipelineBudget::builder()`, produce
parti opache tramite `into_read_parts` / `into_scan_parts` /
`into_convert_parts` / `into_write_parts` **sul `PipelineBundle`**,
e le passa alle factory del core.

**Chi consuma le options e chi esegue il preflight** (una riga per
API del core; le options sono sempre prese **per valore**):

| API del core | Options consumate | Preflight |
|---|---|---|
| `open(path, ReadOptions) -> Result<Dataset>` | da `ReadBudgetParts` | **completo**: `Source::into_path_checked` estrae il permit con `take_input_permit()` ed enumera la sorgente chiamando `note_entry_visited(entry)` una volta per voce — che applica insieme `max_input_entries`, `max_input_bytes` sui byte addebitati e il digest — poi `context.observe_input(permit)?` pubblica il footprint accumulato |
| `Dataset::scan(&self, request, ReadOptions) -> Result<DatasetReader>` | da `ScanBudgetParts` | **leggero** (= niente parsing dei contenuti, **non** niente walk): enumera tutte le entry correnti applicando `max_input_entries` (`note_entry_visited`), legge size+mtime, ricalcola il `SourceDigest` e lo confronta con `expected_footprint()`; divergenza (inclusa aggiunta/rimozione di entry) → `Contract(FootprintChanged)`; poi `observe_input(permit)?` pubblica i valori **correnti**, riaccumulati dall'enumerazione appena eseguita |
| `convert(source, destination, ReadOptions, WriteOptions) -> Result<Published>` | da `ConvertBudgetParts::into_parts()` | come `open`, sul **solo** ramo read; il writer legge `observed_input()` dal context condiviso (INV-6) |
| `create(sink, plan, WriteOptions) -> Result<Writer>` | da `WriteBudgetParts` | **nessuno**: nessun permit trasportato, `ObservedInput::NotObserved` |

### Modulo `plenora_io_core::descriptor` (INV-7, INV-14)

```rust
#[non_exhaustive]
pub struct FormatDescriptor {
    // Tutti i campi privati; accesso solo via getter (INV-14
    // combinato con `#[non_exhaustive]`: struct literal vietato,
    // costruzione via `const_new`).
    id: &'static str,
    // ... altri campi esistenti ...

    // INV-7: tre nuovi campi + legacy read_mode PRESERVATO come
    // valore dichiarato dal driver (non derivato).
    read_mode: ReadMode,                          // legacy, dichiarato
    native_read_mode: NativeReadMode,             // nuovo, dichiarato
    effective_delivery: DeliverySemantics,        // nuovo, dichiarato
    buffering: BufferingStrategy,                 // nuovo, dichiarato
}

impl FormatDescriptor {
    /// Costruttore const per i driver del workspace (INV-14).
    /// `read_mode` e' un parametro esplicito: preserva
    /// byte-per-byte il valore che ogni driver dichiara oggi.
    /// Non c'e' un mapping automatico da `native_read_mode`.
    pub const fn const_new(
        id: &'static str,
        read_mode: ReadMode,                     // legacy, driver-dichiarato
        native_read_mode: NativeReadMode,
        effective_delivery: DeliverySemantics,
        buffering: BufferingStrategy,
        // ... resto dei campi obbligatori (multi_layer,
        //     reader_concurrency, projection_support, ecc.)
    ) -> Self;

    pub const fn id(&self) -> &'static str;
    pub const fn read_mode(&self) -> ReadMode;             // legacy
    pub const fn native_read_mode(&self) -> NativeReadMode;
    pub const fn effective_delivery(&self) -> DeliverySemantics;
    pub const fn buffering(&self) -> BufferingStrategy;
}
```

**`read_mode` legacy: preservato driver-per-driver, byte-per-byte**:

Il campo `read_mode` di `cli-protocol-v1` emette oggi tre
possibili valori — `StreamingSequential`, `StreamingColumnar`,
`Materializing` — con la distribuzione seguente sui driver:

| Driver | `read_mode` legacy attuale |
|---|---|
| csv, geojson, gpkg, dxf, shp, kml, ipc, xls | `StreamingSequential` |
| geoparquet | `StreamingColumnar` |
| filegdb | `Materializing` |

Post-Lotto-0 ogni driver **dichiara esplicitamente** i quattro
campi (`read_mode` + le tre nuove dichiarazioni) in `const_new`.
Il valore di `read_mode` **non e' derivato** da
`native_read_mode`: ogni driver preserva il proprio valore
esattamente come e' oggi. Nessun consumer del wire v1 osserva
alcun cambiamento nel campo `read_mode` per alcun driver.

I nuovi campi `native_read_mode`, `effective_delivery`,
`buffering` sono **additivi** nel JSON di `catalog`. Un consumer
legacy li ignora; un consumer nuovo puo' correlare
`read_mode == StreamingColumnar` con
`native_read_mode == StreamingRandom` (per GeoParquet) o
`read_mode == Materializing` con `native_read_mode == Materialized`
(per FileGDB), ma la correlazione non e' automatica: e' scelta
del driver.

**Back-compat sul campo `read_mode`**:
- Rimuoverlo o cambiare la forma del wire richiederebbe bump a
  `cli-protocol-v2`. Il Lotto 0 NON introduce il bump.
- Il valore dichiarato oggi da ogni driver e' preservato dal
  test snapshot `catalog_v1_read_mode_per_driver_unchanged`
  (vedi tabella decisioni → test).

## Piano di migrazione

Migrazione in **cinque step** interni al Lotto 0. Ognuno lascia il
workspace verde su fmt/clippy/test; il nuovo modello convive con
l'esistente fino allo step finale, quando quello vecchio e' rimosso
in un unico commit atomico.

### M1 — Introduzione di `PipelineBudget` e `PipelineLimits` accanto al vecchio modello

- Nuovo modulo `plenora-io-model::budget` con i tipi elencati.
- `plenora-io-model::Limits` e `plenora-io-model::ResourceBudget`
  restano invariati (compatibilita' interna).
- Suite di test dedicata sui nuovi tipi (>=20 test): builder,
  lease memoria, `into_convert_parts` + `into_parts`,
  `observe_input`, entry gauge.

**Criteri di uscita**: nuovi tipi compilano, test dedicati verdi.
Nessun cambio al comportamento del core.

### M2 — Sostituzione del `BudgetedReader` con lo `SpooledReader` (attuato in S2)

- Nuovo `plenora-io-core::driver::spool` con `StagedSpool` che
  serializza `RecordBatch` in Arrow IPC su un file temporaneo
  **senza nome** (`tempfile::tempfile_in`), addebitandone alla quota
  di spill i **byte realmente scritti** con lease RAII. La stima di
  occupazione in RAM non e' la stessa grandezza dei byte su disco, e
  un `commit` avrebbe consumato la quota per sempre anche dopo la
  rimozione del file.
- **Nessun ponte verso il modello nuovo**: lo spool di M2 e' scritto
  contro il `ResourceBudget` legacy, quindi in M2 un solo modello
  tocca i contatori. `Rows`, `Columns`, `GeometryComponents`,
  `OutputBytes` e `ConcurrentOperations` restano interamente del
  modello legacy e sono attraversati **esattamente una volta**; S4
  migra le chiamate dello spool insieme al resto.
- **Boundedness indipendente dai dati**: ogni batch bufferizzato
  costa almeno `PER_BATCH_OVERHEAD_BYTES`, anche se non ha righe o
  colonne. Senza quel minimo una sorgente che produce batch vuoti in
  serie non farebbe mai scattare la soglia, e la boundedness si
  reggerebbe sull'ipotesi che ogni batch porti dati — cioe' proprio
  cio' che una sorgente ostile non fa.
- **Interrompibilita'**: migrazione e rilettura controllano
  cancellazione e deadline a ogni batch. Sono le due sequenze lunghe
  dello spool, e senza controllo un Ctrl+C non avrebbe effetto fino
  all'ultimo batch.
- **La sonda di EOF resta dentro quota**: quando righe o output sono
  esauriti il reader prova comunque a leggere per distinguere la fine
  della sorgente da una violazione, ma lo fa sotto una lease di
  memoria. Senza memoria residua non si sonda affatto e si fallisce
  chiuso: materializzare fuori quota sarebbe esattamente cio' che il
  budget vieta.
- `BudgetedReader` sostituito internamente: il consumer non si
  accorge del cambio; il picco di RAM passa da O(dataset) a
  `adaptive_memory_threshold + current_batch`, bound indipendente
  dalla dimensione totale dell'input (non `O(batch_target)`).
- Descriptor: **nessun campo nuovo in M2**. Il comportamento di
  consegna resta operation-atomic come oggi, ma i tre campi
  (`native_read_mode`, `effective_delivery`, `buffering`) entrano
  tutti insieme in **M5** (INV-7): fino ad allora
  `effective_delivery` non e' dichiarato da alcun driver ne'
  osservabile nel wire `catalog`. M2 cambia l'implementazione, M5
  la rende dichiarata.
- Test end-to-end su dataset > `memory_bytes` default: prima falliva,
  ora passa.

**Criteri di uscita**: nessuna regressione test; benchmark bounded
sul percorso convert non peggiora oltre veto; **parita' dei limiti
pre/post M2** — un test matriciale `limit_parity_pre_and_post_m2`
esercita al valore di soglia ogni asse (`max_rows`, `max_columns`,
`max_geometry_components`, `max_output_bytes`, `max_input_bytes`,
`max_input_entries`, `memory_bytes`, `spill_bytes`, `duration_ms`,
`concurrent_operations`) e verifica che esito, `LimitKind` emesso e
quantita' consumata siano **identici** al percorso pre-M2: nessun
asse allentato dal bridge, nessun asse contato due volte.

### M3 — Migrazione API di `ReadOptions`/`WriteOptions` al nuovo budget

- `ReadOptions::builder(parts)` / `WriteOptions::builder(parts)`
  accettano parti opache dal model invece di `limits +
  resource_budget` separati.
- Aggiornamento CLI: `resource_budget_from_limits` /
  `conversion_budgets_from_limits` sostituiti dalla catena
  `PipelineBudget::builder().limits(...).build()?` +
  `PipelineBundle::into_convert_parts()` + `into_parts()`.
- Aggiornamento tutti i driver (13 file di sorgente): sostituiscono
  `opts.limits.*` e `opts.resource_budget.*` con accessi al
  `PipelineContext` ottenuto dalle parti.
- INV-6 attivo: writer ora legge `observed_input_bytes` dal context
  condiviso; `L0.10` chiuso.
- **Rimozione contestuale dell'applicazione legacy di
  `max_input_bytes`** in `Source::into_path_checked`: dal momento in
  cui l'enumerazione passa da `note_entry_visited`, il tetto e'
  applicato dal modello nuovo e lasciarlo anche nel vecchio
  significherebbe applicarlo due volte. Deve avvenire nello stesso
  commit, non dopo.
- **Handoff atomico della lease di memoria** (criterio obbligatorio).
  Oggi, in `reader_adapters.rs`, la lease di materializzazione viene
  droppata **prima** che `spool.push()` prenda quella di residenza:
  fra le due c'e' una finestra in cui il batch esiste in RAM e non e'
  contabilizzato da nessuno. Con un budget condiviso — cioe' proprio
  il caso di `convert` — un'altra operazione puo' infilarsi in quella
  finestra e prenotare memoria che di fatto non c'e'.
  Tenere entrambe le lease vive contemporaneamente non e' la
  soluzione: conterebbe due volte lo stesso batch, con la
  prenotazione larga (target + cella) sommata all'occupazione reale,
  e una quota stretta fallirebbe su un batch che ci sta — e' l'errore
  che ha fatto fallire il reader KML in S2.b.
  La migrazione deve quindi introdurre un **trasferimento**: la
  prenotazione passa dal materializzatore allo spool senza tornare al
  gauge nel mezzo, ridimensionandosi dalla quota larga a quella
  effettiva in un'unica operazione atomica sul gauge. Il modello
  nuovo ha il punto giusto dove metterlo — `PipelineContext` possiede
  il gauge — mentre il `ResourceLease` legacy non sa ridimensionarsi.
  Test richiesto: `memory_handoff_leaves_no_unaccounted_window`, che
  osserva il gauge da un secondo thread durante il transito e
  verifica che non scenda mai sotto l'occupazione reale del batch.

**Criteri di uscita**: tutti i test verdi, gate anti-panic verde,
nessuna quota applicata due volte fra modello nuovo e legacy, e
l'handoff della memoria senza finestra scoperta verificato dal test
dedicato.

#### Errata S4.b — forma effettiva del bordo migrato

Il pacchetto prevedeva la sostituzione diretta dei campi di
`ReadOptions`/`WriteOptions`. L'attuazione la esegue in due tempi per
tenere ogni sottocommit verde, e la forma intermedia va registrata
perche' non e' quella descritta sopra:

1. I tre campi legacy non sono rimossi ma **resi inaccessibili**,
   racchiusi in un campo privato `payload: BudgetPayload`. L'enum ha
   due varianti, `Legacy` e `Pipeline`, e nessuna combinazione mista:
   con tre `Option` affiancati esisterebbe lo stato "budget nuovo e
   limiti vecchi entrambi presenti", in cui nessun percorso saprebbe
   quale dei due lo governa.
2. Gli accessori sono centralizzati sul payload e restituiscono
   **scalari immutabili** — quote, non contatori. Non esiste, e il
   pacchetto ne vieta l'introduzione, un accessore che ricostruisca un
   `Limits` nel ramo `Pipeline`: sarebbe la copia fra modelli che la
   migrazione deve evitare.
3. Il ripiego da `Pipeline` a `Legacy` non esiste. Un driver non
   ancora migrato che riceva opzioni del modello nuovo ottiene un
   `Unsupported` tipizzato.
4. Il tipo transitorio, la variante `Legacy` e `Default` sono privati
   o interni al workspace, non sono ri-esportati e **non saranno mai
   visibili dalla facade**. La loro rimozione e' M4/S4.e.

Il **trasferimento della lease allo spool** descritto sopra non e'
attuato in S4.b: `StagedSpool::push` riceve ancora una lease del
modello vecchio. Il meccanismo esiste da S4.a
(`InternalMemoryLease::shrink_to`) ed e' cablato sul percorso reale in
S4.d, insieme al cambio semantico del preflight. In S4.b la firma di
`Source::into_path_checked` e' gia' quella definitiva — le due sole
quote che consulta invece di un `Limits` intero — ma la semantica e'
invariata: anticipare il cambio senza rimuovere nello stesso atto i
controlli legacy applicherebbe le stesse quote due volte.

**Criterio di parita' aggiunto**: a configurazione equivalente i due
rami devono produrre gli stessi valori scalari, verificato da test
dedicati e ancorato ai valori attesi — non alla sola uguaglianza fra i
rami, che due rami rotti allo stesso modo soddisferebbero.

#### Gate S4.b.3 — "costruibile" non e' "utilizzabile"

`ReadOptions::from_read_parts` esiste ed e' coperto da test dal commit
S4.b, ma il **percorso comune** — l'adapter di lettura e lo
`StagedSpool` — prenota ancora memoria con la `ResourceLease` del
modello legacy. Un driver che costruisse opzioni `Pipeline` oggi
otterrebbe un oggetto formalmente corretto e un comportamento a meta':
i contatori di riga dal modello nuovo, la memoria dei batch da quello
vecchio, e la finestra non contabilizzata che
`InternalMemoryLease::shrink_to` esiste per chiudere resterebbe aperta
proprio sul percorso che dovrebbe averla chiusa.

**Il ramo `Pipeline` non e' dichiarabile utilizzabile prima
dell'handoff reale.** L'handoff resta prerequisito iniziale di S4.d —
non entra in S4.b.3, che e' una riconciliazione e non deve trascinare
la riscrittura dello spool — ma il vincolo e' attivo da subito, e
meccanico: il gate `scripts/check_pipeline_branch_gate.py` fallisce se
un crate fuori da `plenora-io-core` costruisce opzioni `Pipeline`
mentre anche una sola delle tre condizioni manca:

1. `spool.rs` e `reader_adapters.rs` liberi da `ResourceLease`;
2. `InternalMemoryLease` effettivamente usato nel core;
3. il test end-to-end
   `handoff_reale_della_memoria_senza_bridge_legacy`, che costruisce le
   opzioni con `from_read_parts`, apre e legge **davvero** attraverso
   adapter e spool con `shrink_to` + move, e lo fa senza passare dal
   ponte legacy.

**S4.c non chiude finche' il gate vincola.** I driver possono essere
preparati — accessori, firme, rimozione degli usi legacy residui — ma
nessuno di essi passa al ramo `Pipeline` prima che le tre condizioni
siano soddisfatte. Il gate si disattiva da solo quando lo sono, e va
rimosso con il ponte in S4.e.

#### Errata S4.c — cosa contiene davvero il sottopasso driver

Con quel vincolo attivo, S4.c non e' "i driver passano al modello
nuovo" ma **"i driver smettono di scegliere il modello"**. E' la
condizione che rende possibile il cambio atomico di S4.d.

Tre punti d'ingresso nel core prendono le opzioni invece dei pezzi
gia' estratti — `preflight_source`, `with_read_budget`,
`with_write_validation` — piu' tre accessori neutri per i percorsi su
misura: `max_vertices()`, `ensure_active()`, `resource_budget()`.
Dopo S4.c il ponte verso il modello legacy e' nominato **solo** dentro
`plenora-io-core`, e l'inventario lo verifica come regola strutturale,
non come conteggio.

**`ensure_active()` stringe il ramo legacy, deliberatamente.**
`ResourceBudget::ensure_active` guarda solo la deadline; il context
del modello nuovo guarda deadline **e** cancellazione. Allineare i due
rami ora evita che i cicli dei driver cambino comportamento il giorno
del passaggio a `Pipeline`. E' l'unico cambio semantico di S4.c, ed e'
volontario.

#### S4.d parte 0 — ownership delle opzioni e gate irrobustiti

Tre chiusure preliminari, prima dell'handoff vero e proprio.

**`FormatDriver::open` consuma le opzioni per valore.** Il contratto
ratificato lo diceva gia'; l'attuazione era rimasta a
`&ReadOptions`, e in quella forma il preflight **non puo'** estrarre
il permit, perche' `take_input_permit(&mut self)` non e' chiamabile
attraverso un riferimento condiviso. Le vie per conservare
`&ReadOptions` — un `Mutex<Option<InputPermit>>`, o un permit clonato
— sono escluse: reintrodurrebbero uno stato mutabile nascosto dietro
una firma immutabile e la possibilita' di osservare due volte lo
stesso input, cioe' esattamente cio' che il permit esiste per
impedire. `preflight_source` prende ora `&mut ReadOptions`; consumare
il permit non consuma le opzioni, che restano leggibili dall'adapter.
Il consumo effettivo resta S4.d: qui cambia la forma, non la
semantica.

**Il perimetro dei gate testuali era troppo stretto.** Guardavano solo
`crates/*/src/**`: un test d'integrazione in `tests/`, un benchmark in
`benches/`, un `examples/` o un `build.rs` potevano attraversare il
confine del permit senza che nulla lo vedesse — e sono proprio i posti
dove si scrive codice di servizio con meno attenzione. Riconoscevano
inoltre la sola forma a metodo, mentre
`ReadBudgetParts::into_components(parts)` in UFCS fa la stessa cosa; e
cercavano `publish = false` come testo, che una riga commentata
avrebbe soddisfatto. Ora il perimetro comprende ogni `.rs` di ogni
crate piu' `fuzz/`, le forme riconosciute includono UFCS e il
riferimento a funzione senza chiamata, e il manifesto e' letto come
TOML.

**Il gate dell'handoff si accontentava di una menzione.** Il nome del
test bastava trovarlo in un punto qualsiasi del crate: un commento che
spiegasse cosa mancava lo avrebbe sbloccato. Le condizioni sono ora
ancorate ai due file del percorso comune e descrivono **responsabilita'
distinte**, non la presenza degli stessi simboli in entrambi:
`reader_adapters.rs` acquisisce la lease, chiama `.shrink_to(...)` e la
cede; `spool.rs` la riceve e la custodisce, e **non** deve acquisirne
una seconda ne' ridurla di nuovo. Chiedere `shrink_to` in entrambi
avrebbe spinto verso una chiamata duplicata, o verso un commento messo
li' per accontentare il gate.

Il sorgente viene inoltre **spogliato di commenti e stringhe** prima
delle regex, e i moduli `#[cfg(test)]` sono esclusi dalle sole regole
di responsabilita': un `/* #[test] fn handoff_reale... */` non
descrive codice che gira, e un helper di test che imita l'adapter
acquisisce legittimamente una lease.

#### S4.d — handoff reale, preflight osservante, controlli legacy rimossi

Il sottopasso e' atomico per necessita': consumo del permit, rimozione
dei controlli duplicati e migrazione del percorso comune sono lo stesso
cambiamento visto da tre lati.

`Source::into_path_observed` sostituisce `into_path_checked`. Enumera
chiamando `note_entry_visited` una volta per voce **scoperta** — non al
prelievo, cosi' la coda resta bounded — e spende il permit in
`observe_input` a enumerazione conclusa. I controlli vecchi non sono
stati spostati: sono spariti nello stesso atto in cui il context ha
iniziato ad applicarli.

Adapter e spool prendono un `OperationBudget`: contatori dai suoi
gauge, memoria e spill dal context. `with_read_budget` accetta ora
**solo** il modello unificato. L'adapter prenota largo, misura, riduce
con `shrink_to` all'ingombro reale piu' `PER_BATCH_OVERHEAD_BYTES`, e
sposta la stessa lease nello spool per `move`.

**Due allineamenti semantici, entrambi voluti.** La quota di spill dei
driver DXF/KML/XLSX non e' piu' consumo definitivo ma occupazione
trattenuta, restituita al drop del file temporaneo. E la concorrenza
vive nel pool (INV-12): senza pool la lease e' un no-op, quindi i test
che verificano il tetto usano ora un `ResourcePool` esplicito.

**Il gate `check_pipeline_branch_gate.py` si e' sbloccato da solo**, ed
e' rimovibile con il ponte in S4.e.

#### Errata S4.d.1 — due difetti dell'handoff e due guardie

**L'ingombro strutturale non era sempre coperto.** La prenotazione
valeva `target_bytes + max_wkb_cell_bytes`, l'ingombro contabilizzato
`bytes + PER_BATCH_OVERHEAD_BYTES`: quando la somma dei primi due
stava sotto l'overhead, allo spool arrivava una lease piu' piccola del
batch. L'overhead entra ora nella prenotazione di **memoria** e solo in
quella — quella di output conta byte prodotti, non occupazione interna
— e prima della cessione un controllo esplicito
`accounted <= memory_lease.bytes()` fallisce chiuso.

**Il pool non entrava nel dimensionamento.** `remaining_memory()`
riportava il solo gauge locale mentre `lease_memory_internal` compone
locale e pool (INV-12): con pool stretto l'adapter chiedeva piu' di
quanto entrasse e falliva invece di spillare, e
`adaptive_memory_threshold` calcolava una soglia irraggiungibile. Il
context espone ora residuo e capacita' **effettivi**, minimo fra
locale e pool, per memoria e spill.

**Identita' del percorso.** `to_string_lossy` faceva collassare due
percorsi Unix non-UTF-8 distinti sullo stesso digest. Si usano ora i
byte nativi dell'`OsStr`, con una codifica per piattaforma.

Il campo si chiama `path_identity_bytes` e non piu' `normalized_path`:
il nome vecchio prometteva una normalizzazione che nessuno faceva —
`a/../b` e `b` restano distinti — e il pacchetto la dava per acquisita.
E' una codifica **senza perdita del percorso lessicale**, non
un'identita' canonica del filesystem. La canonicalizzazione resta
esclusa: `fs::canonicalize` segue i symlink, che il preflight rifiuta,
e farla qui allargherebbe il contratto proprio dove e' stato
ristretto.

**Cancellazione per voce**, non per directory: il controllo e' in testa
a `scopri`.

**Due guardie direzionali.** `richiede_modello_legacy` e
`richiede_modello_unificato` sostituiscono l'unica precedente, che
diceva "componente non ancora migrato" anche dove il componente e'
migrato e sono le opzioni a essere vecchie. L'inventario le conta
separatamente: la prima e' debito e deve scendere, la seconda e'
progresso.

**S4.c e S4.d si dichiarano chiusi.**

#### S4.e — il modello legacy non esiste piu'

Ultimo sottopasso. Spariscono `BudgetPayload::Legacy` e l'enum
stesso, i `Default` delle opzioni, i costruttori e gli accessori
legacy, entrambe le guardie direzionali, e i tipi `Limits`,
`ResourceBudget`, `ResourceLease`, `ResourceKind`, `ResourceLimits`
con l'intero `resource.rs`. In `limits.rs` resta il solo `WkbLimits`,
che e' un tipo del contratto e non del budget.

Le opzioni **non hanno `Default`**: portano un `OperationBudget`, che
nasce da una costruzione che puo' fallire, e un `Default` avrebbe
dovuto scegliere fra il panico e quote inventate.

Il ramo di scrittura entra nella pipeline: il writer preleva dai
contatori dell'operazione e la memoria dello staging e' una
`InternalMemoryLease`. `convert` usa **un solo context** — i due rami
escono dallo stesso `ConvertBudgetParts`, con contatori indipendenti
e memoria, spill e deadline condivisi — che chiude anche la parte di
INV-6 che due budget scollegati rendevano irraggiungibile.

**Inventario a zero su tutte le categorie**, senza eccezioni ne'
tetti. I gate `check_legacy_budget_inventory.py` e
`check_pipeline_branch_gate.py` sono rimossi: non sorvegliano piu'
nulla, perche' ora e' il compilatore il gate — `ReadOptions::default()`
non compila, `Limits` non esiste, un driver non puo' nominare un
budget legacy perche' il tipo non c'e'. Una regressione non e'
distratta: e' inesprimibile.

**M4 del piano di migrazione e' attuato. S4 e' chiuso.**

### M4 — Rimozione del vecchio modello

- `plenora-io-model::Limits`, `ResourceBudget`, `ResourceLimits`,
  `ResourceLease` rimossi.
- Un unico commit che chiude la migrazione.
- Gate CI aggiunto: grep che vieta ricomparse dei nomi vecchi.

**Criteri di uscita**: nulla del vecchio nome ricomparibile.

### M5 — Descriptor semantic split (INV-7)

- Aggiunti `native_read_mode`, `effective_delivery` e `buffering`
  a `FormatDescriptor`.
- `read_mode` resta **dichiarato esplicitamente** da ogni driver
  in `const_new`, invariato nel wire: nessun `#[deprecated]`,
  nessun mapping automatico da `native_read_mode`.
- Documentazione aggiornata in ADR e README.
- CIA registrata.

**Criteri di uscita**: descriptor coerente col comportamento reale
osservato negli step M2/M3.

## Impatti contrattuali

**Contratto CLI `cli-protocol-v1.json`**:
- Envelope invariati.
- Nuova invariante di redazione (INV-10) implica riscrittura del
  `message` in alcuni envelope `plenora-io-error-v1`; il testo del
  messaggio diventa curato invece che formattato al volo, ma il
  contract dichiara solo la presenza del campo, non la sua forma.
  **Impatto**: gli snapshot test dei messaggi vanno aggiornati;
  nessun cambio di wire format.
- Nessun bump `cli-protocol-v2`.

**Contratto errore `plenora-io-error-v1`**:
- Nessun nuovo campo, nessun campo rimosso.
- Set di categorie/fasi/effetti/retry invariato.
- **Cambio osservabile**: il testo di `message` non incorpora piu'
  valori derivati da input utente. Un consumer che grep-fa il
  messaggio per estrarre valori (pattern sbagliato ma esistente in
  natura) va aggiornato. Dichiarare in nota di release.

**Descrittori di formato**:
- Nuovi campi `native_read_mode`, `effective_delivery`,
  `buffering` in `catalog`. **Additivo** al wire v1.
- Campo `read_mode` **mantenuto** in `catalog`, dichiarato
  esplicitamente driver-per-driver e invariato nel wire (nessun
  mapping a `native_read_mode`). NON viene rimosso in questo
  pacchetto; la rimozione richiede un futuro bump
  `cli-protocol-v2`. Nessuna deprecazione dell'API Rust interna:
  il campo resta un valore di prima classe dichiarato in
  `const_new`.

**API Rust interna** (crate del workspace):
- `plenora-io-model::Limits` → rimosso in M4.
- `plenora-io-model::ResourceBudget` → rimosso in M4, sostituito da
  `budget::PipelineBudget` + `budget::OperationBudget`.
- `plenora-io-core::driver::ReadOptions.limits` → sostituito da
  `budget`.
- Tutti i 13 file driver toccati in M3.

**Release manifest `release/1.1.0.json`**:
- Nome coerente con la finestra di release in corso
  (post-hardening 1.1.0-candidate). Il Lotto 0 chiude i 10
  finding residui **all'interno** della finestra 1.1.0; non e' un
  bump minor separato.
- `functional_delta.contracts = "extended"` (INV-7: nuovi campi
  descriptor additivi).
- `functional_delta.error_semantics = "extended"` (INV-10: forma
  del messaggio cambia).
- `functional_delta.cleanup = "behavior_changing"` (10 finding).
- `wire_formats = "unchanged"` (nessun bump v2).

## Rischi

**R1 — Regressione performance su convert**. Lo spool su disco (M2)
introduce I/O aggiuntivo per la scrittura IPC + rilettura. Su
dataset piccoli (<< memory budget) il costo puo' dominare.
Mitigazione: la strategia ratificata e' `AdaptiveMemoryThenDisk` —
finche' l'occupato sta sotto `memory_bytes / 2` il buffer resta in
RAM come oggi e non tocca il disco; migra su file solo se il
dataset non ci sta. La soglia va misurata con benchmark bounded
prima del merge M2.

**R2 — Interazione con driver che gia' fanno spool proprio (XLSX,
DXF)**. Alcuni driver hanno gia' un loro spool interno via
`calamine` / iteratori progressivi. Aggiungere lo spool comune
sopra rischia di raddoppiare il costo. Mitigazione: driver che
espongono `native_read_mode = StreamingSequential` genuino possono
bypassare lo spool comune quando il consumer lo consente esplicitamente
(configurazione futura, fuori Lotto 0).

**R3 — Migrazione dei call site driver in M3**. 13 file, ognuno con
pattern proprio di uso di `opts.limits.*` e `opts.resource_budget.*`.
Rischio di svista. Mitigazione: gate CI di grep sulle vecchie API
attivato al termine di M3.

**R4 — Redazione errore in S9 (L0.6)**. La rimozione della
capacita' di formattare messaggi liberi cambia il testo di errori
che i test snapshot congelano. Rischio: la migrazione fa saltare
molti test in bulk. Mitigazione: passo dedicato dopo S4 con
riscrittura sistematica dei test snapshot del solo campo
`message` (Invariante B della sezione DTO), lasciando invariata
l'Invariante A sulla struttura del wire; il pacchetto d'errore
va progettato affinche' ogni variante dell'enum abbia un
`Display` deterministic.

**R5 — Perdita di informazione diagnostica per gli operatori**. Un
messaggio curato "type non supportato" invece di "type 'FooBar' non
supportato" perde contesto per chi debugga. Mitigazione: il
valore reale finisce in `row_diagnostics` gated dalla policy
`emit`/`redact` gia' esistente (contratto
`plenora-row-diagnostics-v1`), non nel messaggio. Un operatore con
policy `emit` sui campi rilevanti vede tutto; un consumer pubblico
con policy `redact` vede solo il messaggio curato + gli assi
dell'errore. **Non** si usa un `Fingerprint` pubblico (rimosso in
questa iterazione, vedi INV-10).

**R6 — `max_input_entries` con default sbagliato**. Un default
troppo basso rifiuta directory legittime; troppo alto non protegge.
Mitigazione: partire con `10_000` (ordine di `max_columns`),
misurare su dataset reali (Shapefile companion sets di grandi
progetti raggiungono qualche decina di entry per layer; `.gdb`
directory possono superare il centinaio) e riaggiustare via CI
prima di M4.

**R7 — Governance**. Il pacchetto tocca l'API pubblica del model e
del core: richiede ADR nuova, CIA multipla, non solo commit
tecnici. Rischio politico piu' che implementativo.

## Ordine d'implementazione

Sequenza intra-Lotto 0: **13 step (S0-S12 inclusi)**. I finding non
citati esplicitamente in uno step sono aperti come task paralleli
quando il modello lo consente.

| Step | Contenuto | Chiude | Prerequisiti | Stima gg-persona |
|---|---|---|---|---|
| **S0** | Ratifica di questo pacchetto; promozione ADR-IO 7 da Draft a Accepted; CIA sul modello budget | (governance) | — | 2-3 |
| **S1** | M1 introduzione `PipelineBudget` accanto al vecchio | INV-1..-3 preparazione | S0 | 3-4 |
| **S2** | M2 `StagedSpool` e `SpooledReader`; benchmark bounded | INV-5, INV-8, L0.3, L0.4 | S1 | 5-7 |
| **S3** | L0.9 `max_input_entries` integrato in `PipelineContext` | INV-9 | S1 | 1-2 |
| **S4** | M3 migrazione API `ReadOptions`/`WriteOptions`; CLI aggiornata | INV-2, INV-4, INV-6, L0.2, L0.10 | S1, S2 | 4-6 |
| **S5** | L0.1 propagazione limiti in inferenza CSV/GeoJSON/XLSX | (usa il nuovo budget) | S4 | 3-4 |
| **S6** | L0.7 schema dichiarativo `format_options` (design in doc separato, poi implementazione) | (necessario per comando `options` post-facade) | S4 | 4-6 |
| **S7** | M4 rimozione vecchio modello; gate CI di grep | pulizia | S4, S5 | 1-2 |
| **S8** | M5 descriptor split `native`/`effective`; CIA | INV-7 | S2, S4 | 2-3 |
| **S9** | Bozza L0.6 → implementazione errori strutturati | INV-10 | S4 | 5-7 |
| **S10** | L0.5 validazione completa covering GeoParquet 1.1 (tipi, unicita', coerenza) | driver-specific | indipendente | 2-3 |
| **S11** | L0.8 `wkb_shape` ispeziona figli collection | driver-specific | indipendente | 1-2 |
| **S12** | L6 pre-scansione WKT/GeoJSON, in-parse depth/components, fuzz target dedicati, `hostile_input_hardened` dichiarato per-driver | L6 (ratificato A) | S4, S6 | 6-9 |

Il grafo di dipendenza consente overlap significativo:
- S3 in parallelo a S2.
- S10 e S11 completamente indipendenti, in parallelo a S1-S9.
- S5 in parallelo a S6 dopo che S4 chiude.
- S9 puo' iniziare dopo S4 e proseguire in parallelo a S5-S8.
- S12 in parallelo a S9 (dipende da S4 e S6, entrambi chiusi
  entro la meta' del lotto).

## Stima aggiornata

Le due unita' di misura sono distinte e non intercambiabili:

- **Effort** = giorni-persona: la somma del lavoro tecnico
  necessario, indipendente dal numero di persone.
- **Calendario** = settimane: il tempo di attraversamento dato un
  team di dimensione fissa. Non scala linearmente con il numero
  di persone (dipendenze, code review, governance).

**Effort totale (giorni-persona)**: **46-68**. Include tutti i
13 step (S0-S12, S12 = L6-A obbligatorio) piu'
conformance/matrici/evidence per la release stabile 1.1.0.

Ripartizione per step:
- S0 governance: 2-3 gg-persona.
- S1 nuovo modello: 3-4 gg-persona.
- S2 spool + benchmark: 5-7 gg-persona.
- S3 max_input_entries: 1-2 gg-persona.
- S4 migrazione API: 4-6 gg-persona.
- S5 propagazione limiti inferenza: 3-4 gg-persona.
- S6 format options schema: 4-6 gg-persona.
- S7 rimozione vecchio modello: 1-2 gg-persona.
- S8 descriptor split: 2-3 gg-persona.
- S9 errori strutturati: 5-7 gg-persona.
- S10 covering GeoParquet: 2-3 gg-persona.
- S11 wkb_shape figli: 1-2 gg-persona.
- S12 L6 pre-scansione WKT/GeoJSON: 6-9 gg-persona.
- Conformance cross-component: 3-4 gg-persona.
- Matrici multipiattaforma: 2-3 gg-persona.
- Evidence base + CIA formali: 2-3 gg-persona.

**Calendario stimato** (assunzioni esplicite, non scaling lineare):

| Dimensione team | Calendario | Note |
|---|---|---|
| 1 persona | 12-16 settimane | Sequenziale worst-case; niente overlap. |
| 2 persone | 7-10 settimane | Overlap S3+S2, S10+S11 con S1-S9, S5+S6 dopo S4, S12 in parallelo a S9. |
| 3 persone | 5-8 settimane | Overlap piu' spinto ma marginale: S0→S1→S2→S4 restano sequenziali. |

Le stime calendario **non includono** il tempo di ratifica di
governance (ADR-IO 7 e CIA multipla): dipende dai cicli
decisionali, non dal lavoro tecnico.

**Esclude** (non Lotto 0, contati separatamente in altri
documenti):
- Facade Rust `plenora-io-api`.
- CLI comandi nuovi (`formats`, `options`, `schema`, `validate`).
- SDK Python.
- Wheel matrix.

## Trattamento di L6 (parser progressivo WKT/GeoJSON): incluso come S12 obbligatorio

**Ratificato**: L6-A. L6 e' incluso nel Lotto 0 come step S12
obbligatorio. La release stabile 1.1.0 supporta input non fidati
(entro i limiti configurati). Non esistono opzioni alternative in
questo pacchetto.

**Contenuto S12**:
- Implementa l'opzione C della proposta
  `PROPOSAL-L6-progressive-wkt-geojson.md` (pre-scansione lineare
  del payload prima del parse) per entrambi i formati coinvolti.
- Applica `max_depth` e `max_components` **in-parse** su WKT
  (CSV/XLSX) e su GeoJSON, sostituendo il cap solo-byte
  attualmente in uso.
- Introduce fuzz target dedicato per la pre-scansione e per il
  parse bounded; li aggiunge a `scripts/fuzz-smoke.sh`.
- Registra un benchmark differenziale (pre-scan vs parse) su
  fixture rappresentative; il veto prestazionale si applica come
  per gli altri step (regressione mediana oltre il 10% =
  revisione).

**Effort S12**: 6-9 giorni-persona.

**Effetto sulla release**: `hostile_input_hardened` e' una
capability **per-driver**, non un flag globale. Ogni entry di
`catalog` espone un campo booleano `hostile_input_hardened`
dichiarato dal `FormatDescriptor` del driver. Post-S12 i
driver WKT/GeoJSON (CSV, XLSX, GeoJSON) lo espongono a `true`;
i driver che non usano parser WKT/GeoJSON (Shapefile, IPC,
GeoParquet binario, GPKG WKB, FileGDB, KML, DXF) lo espongono
come `true` solo se il loro parser rispetta le stesse invarianti,
altrimenti a `false` con motivazione documentata nel descriptor.
Le note di release elencano formato-per-formato il valore atteso
e i limiti applicabili (`max_wkb_components`, `max_wkb_depth`,
`max_wkb_cell_bytes`, `max_input_bytes`, `max_input_entries`).

## Chiusura formale della stabilizzazione

La stabilizzazione del core (Lotto 0 → release stabile 1.1.0) si
considera conclusa **solo** quando **tutti** i seguenti punti sono
verificati con **evidenza materiale** (codice merged + test
verdi + artefatti CI), non semplicemente documentati:

1. Tutti e 10 i finding L0.1-L0.10 chiusi tramite **codice
   verificato**:
   - modifica del codice mergiata su `main`;
   - almeno un test di accettazione (unit, integration o
     fuzz) che copra il comportamento corretto e regge la
     regressione;
   - un finding **non** e' considerato chiuso se coperto solo
     da documentazione o dichiarazione di contratto; deve
     avere codice + test che rendano la regressione visibile.
2. Finding L6 chiuso con lo stesso criterio (codice + test +
   fuzz): pre-scansione applicata, in-parse depth/components
   attivi, fuzz target verdi. Flag `hostile_input_hardened:
   true` emesso in `catalog`.
3. Nessuna incoerenza fra descriptor e comportamento
   (INV-7 rispettato per ogni driver), verificata da un test
   automatico che compari il valore dichiarato del descrittore
   con il comportamento osservato in un test end-to-end.
4. Limiti, cancellazione ed errori uniformi in tutti i driver
   (INV-1, INV-4, INV-10), verificati da suite trasversale.
5. Suite tecniche verdi:
   - `cargo test --workspace --all-targets --all-features`;
   - `cargo fmt --check`;
   - `cargo clippy --workspace --all-targets --all-features`
     (all + pedantic + nursery);
   - gate anti-panic (`cargo clippy --lib --bins --exclude
     plenora-bench --exclude plenora-fuzz` con lint stretti);
   - `scripts/fuzz-smoke.sh` verde (senza quarantena estesa);
   - benchmark bounded documentato: baseline pre-Lotto-0 vs
     post-Lotto-0 registrata in
     `docs/assurance/CHANGE_IMPACT_YYYY-MM-DD_LOTTO_0_PERFORMANCE.md`,
     nessuna regressione mediana oltre il veto (dichiarato al
     10%).
6. Matrici multipiattaforma verdi:
   - Linux (Ubuntu 22.04 e 24.04) + GDAL matrix;
   - Windows Server 2022 + GDAL nativo;
   - macOS 14 arm64.
7. Verifiche cross-component:
   - roundtrip IO-tools ↔ data-tools su fixture reali;
   - roundtrip IO-tools ↔ database-tools (PostgreSQL, MySQL)
     su fixture reali;
   - snapshot degli envelope `plenora-io-error-v1` con **struttura
     wire invariata** (campi/ordine/tipi identici alla baseline) e
     `message` intenzionalmente diverso (vedi INV-10 + DTO).
8. Evidence base same-SHA nuova (`release/1.1.0.json` con
   `functional_delta` come dichiarato in "Impatti contrattuali"),
   con:
   - workflow CI same-SHA verde;
   - link agli artefatti (LCOV coverage, fuzz corpus, benchmark
     JSON);
   - CIA registrata sotto `docs/assurance/` per: introduzione
     `PipelineBudget`, sostituzione `BudgetedReader` con spool,
     redazione errore strutturata, eventuale L6.

Solo dopo il tag della release stabile 1.1.0 si riprendono,
nell'ordine gia' concordato: facade Rust → CLI completo → SDK
Python → wheel packaging.

## Tabella decisione → API concreta → test di accettazione

Riga per riga, ogni decisione di questo pacchetto proiettata su un
elemento di API pubblica identificabile e su un criterio di test
verificabile. Nessuna decisione senza copertura API + test.

| Decisione | API concreta (dopo Lotto 0) | Test di accettazione |
|---|---|---|
| **INV-1** modello unico dei limiti | `plenora_io_model::budget::PipelineLimits` con setter fluent e getter per ogni quota, `max_vertices` incluso, piu' `effective_wkb_components()`; `Limits` e `ResourceLimits` rimossi | Gate CI `check_no_legacy_budget.py` (attuato in S4.e, **permanente**): vieta `Limits` — il tipo esatto, non `PipelineLimits` ne' `WkbLimits` — `ResourceLimits`, `ResourceBudget`, `ResourceLease`, `ResourceKind`, `BudgetPayload`, i costruttori `from_legacy`, gli accessori `legacy_*` e `resource_budget`, il `Default` di `ReadOptions`/`WriteOptions`, e l'esistenza stessa di `resource.rs`. Scandisce ogni `.rs` di ogni crate piu' `fuzz/`, non i soli `src/`, e spoglia commenti e stringhe: un commento che spiega perche' un tipo e' stato rimosso ne nomina il nome. Non ha tetti — dichiara uno stato raggiunto, non una migrazione in corso — a differenza dell'inventario a soglie decrescenti che ha sostituito e che e' stato rimosso con S4.e. Test unit `pipeline_limits_default_has_no_zero_field`; `every_quota_has_a_setter_and_a_getter_that_agree`; `unified_defaults_stay_at_the_tightest_historical_values` (fissa i valori attesi con l'origine accanto: il confronto con le strutture legacy non e' piu' scrivibile, il requisito si'); `effective_wkb_components_is_tightened_by_max_vertices` |
| **INV-2** costruzione unica | `PipelineBudget::builder().build() -> Result<PipelineBundle>` **opaco**; `PipelineBundle::into_read_parts` / `into_scan_parts(expected)` / `into_convert_parts` / `into_write_parts` producono parti opache che, via i trait **sealed** `IntoReadParts`/`IntoWriteParts`, sono l'unica via verso le factory `ReadOptions::builder(parts)` / `WriteOptions::builder(parts)` del core; i driver leggono il modello solo da `ReadOptions::budget()` / `WriteOptions::budget()` | Doc test compile-fail: `OperationBudget { .. }` non compila; `ReadBudgetParts { .. }` non compila; `let PipelineBundle { budget, permit } = ..` non compila (campi non pubblici); `impl IntoReadParts for MyParts` non compila (sealed); test unit `pipeline_builder_yields_opaque_bundle` |
| **INV-3** no doppio conteggio in convert | `PipelineBundle::into_convert_parts()` restituisce `ConvertBudgetParts`; `into_parts(self) -> (ReadBudgetParts, WriteBudgetParts)` da' due rami a contatori cumulativi indipendenti sotto lo stesso `PipelineContext` | Test unit `convert_of_n_rows_with_max_rows_n_succeeds` (esattamente `--max-rows N` per un dataset di N righe); test `read_and_write_counters_do_not_share_atomic_ptr` |
| **INV-4** grandezze pipeline-wide condivise | `PipelineContext` fields: `deadline`, `observed_input`, `memory`, `spill`, `pool: Option<ResourcePool>`, `entries_visited`, `cancellation` | Test unit `context_arc_is_shared_between_split_children`; test `cancel_pipeline_cancels_both_operation_budgets` |
| **INV-5** memoria posseduta internamente, rilasciata al transfer | `InternalMemoryLease` in `plenora-io-model::budget`: tipi **pubblici ma opachi, workspace-internal** — non restituiti insieme al batch e non ri-esportati dalla facade `plenora-io-api`; rilascio nel corpo dell'adapter interno al `return Ok(Some(batch))` di `next_batch` | Test integration `dataset_reader_releases_internal_memory_lease_on_batch_transfer`; test `long_lived_dataset_never_accumulates_memory` |
| **INV-6** `ObservedInput` tipizzato + `output_expansion_ratio` corretto | `PipelineContext::observed_input() -> ObservedInput` derivato dallo stato di osservazione; `OperationBudget::output_limit()` con regole esplicite per `NotObserved`, `Bytes(0)`, `Bytes(n>0)`; il prelievo di `OutputBytes` proietta il consumo e sottrae la quota nella **stessa** osservazione atomica, cosi' richieste concorrenti non superano insieme il tetto derivato | Test unit `output_limit_no_expansion_when_not_observed`; `output_limit_no_expansion_when_bytes_zero`; `output_limit_applies_expansion_when_bytes_positive`; test integration `convert_writer_sees_input_observed_by_reader`; test concorrente `output_bytes_ceiling_holds_under_concurrent_requests` |
| **INV-7** descriptor tre assi | `FormatDescriptor` con `native_read_mode`, `effective_delivery`, `buffering` (piu' `read_mode` legacy) | Test snapshot `catalog_envelope_v1_includes_legacy_read_mode_and_new_axes`; test trasversale `every_driver_declares_delivery_matching_actual_behavior` |
| **INV-8** validation atomicity + publish atomicity + IPC replay tipizzato | Struttura interna del `SpooledReader` con fase `validate` → `replay` → `publish`; errori di replay come `ErrorKind::Io(IoErrorKind::SpoolReplay)` o `ErrorKind::Contract(SpoolCorruption)` | Test end-to-end `convert_with_invalid_source_leaves_destination_absent`; test `spool_replay_error_after_validation_produces_typed_error_and_leaves_destination_absent` (simula corruzione del file spool) |
| **INV-9** `max_input_entries` + `max_input_bytes` atomici | `PipelineLimits.max_input_entries: u64`; `PipelineContext.note_entry_visited(entry)` applica in un solo atto, sotto lo stesso mutex, conteggio entry, byte addebitati e digest. I controlli precedono ogni scrittura, quindi un rifiuto non lascia nulla di aggiornato. Le directory contano come entry e addebitano zero byte | Test unit `directory_scan_with_10001_entries_rejects_with_typed_error` (default `10_000`); `custom_max_input_entries_is_honored`; `entry_beyond_max_input_bytes_is_rejected`; `rejected_entry_leaves_no_partial_update`; `rejected_entry_does_not_enter_the_digest`; `directories_count_as_entries_without_charging_bytes`; `note_entry_visited_after_publication_is_rejected` |
| **INV-10** redazione strutturale + DTO conforme v1 | `PlenoraIoError` con campi privati + `PublicMessage` enum (senza `WithContractIdentifier`); `Option<ContractIdentifier>` nell'`ErrorContext`, da cui il DTO deriva `field` del wire; `PublicErrorDto` privato serializza a JSON v1 | Doc test compile-fail: `PlenoraIoError { message: "raw".to_string(), .. }` non compila; test snapshot `error_envelope_v1_structure_conforms_to_baseline` (struttura wire invariata, normalizza `message` prima del confronto); test separato `error_message_text_matches_curated_rendering_baseline` sul solo campo `message` |
| **INV-11** deadline cumulativa | `PipelineContext.deadline: Instant` unico | Test `convert_with_timeout_50ms_fails_within_60ms_total` (non 100ms totali) |
| **INV-12** concorrenza via `ResourcePool` + composizione | `ResourcePool::builder().concurrent_operations(n).build()`; agganciato via `.resource_pool(pool)`. **Senza pool**: memory/spill sono gauge **locali** al context (quota = `PipelineLimits`), concorrenza **assente** e `lease_concurrency()` no-op. **Con pool**: memory/spill = min(quota locale, pool) con consumo di entrambi i gauge, concorrenza governata **solo** dal pool | Test `memory_lease_is_local_and_enforced_without_pool`; `memory_lease_uses_min_of_local_and_pool_quota`; `lease_concurrency_is_noop_without_pool`; `two_pipelines_sharing_pool_compete_on_concurrency_gauge`; `pipeline_without_pool_does_not_count_against_others` |
| **INV-13** model→core vietato + permit opaco one-shot | `plenora-io-model::InputPermit` opaco non-Clone senza costruttori pubblici, emesso dentro il `PipelineBundle` **opaco** da `PipelineBudgetBuilder::build`; esce solo dentro le parti, e da queste si separa **per move** attraverso l'unica API di decomposizione `ReadBudgetParts::into_components`, `#[doc(hidden)]` e workspace-internal (errata S4.b.3: non "mai separabile" — fra crate distinti Rust non lo impone); consumato per `move` da `PipelineContext::observe_input(permit) -> Result<SourceFootprint, PlenoraIoError>` (legato al context che lo ha emesso), estratto dal core con `ReadOptions::take_input_permit()`, `pub(crate)`; nessun import di `plenora-io-core` dal `Cargo.toml` di `plenora-io-model` | Gate CI `check_api_boundary.py` verifica assenza di `plenora-io-core` fra le dep di `plenora-io-model`; gate CI `check_permit_boundary.py` verifica `publish = false` sui due crate, la marcatura `#[doc(hidden)]`, l'assenza di un `take_input_permit` pubblico nel modello e l'assenza di usi della decomposizione fuori da model/core; doc test compile-fail `let _ = permit.clone();`; `let p = InputPermit { .. };`; `bundle.permit` (campo inesistente); test `observe_input_consumes_permit_by_move_and_second_call_does_not_compile` |
| **INV-14** `FormatDescriptor` const-costruibile dai driver | `FormatDescriptor::const_new(...)` accessibile in contesto const | Compile test: ogni driver dichiara `static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(...)`; verifica di build |
| **ADR-IO 7 A** spool bounded + file di spool senza nome | `StagedSpool` in `plenora-io-core::driver::spool`; file creato con `tempfile::tempfile_in`, scollegato dal filesystem all'apertura su Unix e `FILE_FLAG_DELETE_ON_CLOSE` su Windows; nessun path apribile, nessun orfano, nessuno sweep; `PLENORA_SPILL_DIR` sceglie il volume e fallisce chiuso se inutilizzabile; quota di spill RAII applicata alle scritture fisiche | Test `dataset_over_memory_bytes_succeeds_via_spool`; `an_unusable_spill_dir_fails_closed_instead_of_falling_back`; `a_spill_dir_that_is_a_file_is_rejected`; `an_underestimated_batch_cannot_write_beyond_the_quota`; `a_quota_smaller_than_the_reservation_chunk_is_usable`; `reaching_eof_releases_file_and_quota_while_the_spool_is_still_alive`; `spill_quota_returns_to_the_budget_when_the_spool_is_dropped`; `a_corrupted_spool_fails_typed_instead_of_truncating_silently` |
| **L0.1** propagazione limiti in inferenza | `infer_types(path, &PipelineLimits)` / `infer_schema(path, &PipelineLimits)` / XLSX WKT parse con `PipelineLimits` reale (letto dal `PipelineContext`) | Test `inference_uses_configured_wkt_cell_bytes_not_default`; test `inference_respects_max_input_entries` |
| **L0.5** validazione covering GeoParquet | Verifica di tipi FLOAT/DOUBLE, unicita' dei nomi covering, presenza contestuale prima dello strip | Test `geoparquet_covering_with_non_float_column_is_rejected`; test `geoparquet_covering_duplicated_names_is_rejected` |
| **L0.7** schema dichiarativo `format_options` | Registry `plenora-io-model::format_options` con `FormatOptionsSchema` per driver; chiavi sconosciute → `PlenoraIoError::Unsupported`; valori invalidi → error tipizzato | Test snapshot `every_driver_has_a_schema_for_options`; test `unknown_option_key_produces_typed_error_not_silent_ignore`; test `unknown_compression_value_produces_typed_error_not_snappy_default` |
| **L0.8** `wkb_shape` figli collection | `wkb_shape` ispeziona ricorsivamente i figli e propaga `Empty` se tutti empty | Test `multipoint_of_two_empty_points_is_empty`; test `geometrycollection_of_empty_children_is_empty` |
| **L6 (ratificato A, S12)** parser progressivo | Pre-scansione lineare WKT/GeoJSON + in-parse `max_depth`/`max_components`; capability **per-driver** `hostile_input_hardened: bool` dichiarata nel `FormatDescriptor` di ogni driver e riemessa in `catalog` entry-per-entry | Test `wkt_prescan_rejects_depth_over_limit_without_allocating_ast`; `geojson_prescan_rejects_components_over_limit`; fuzz target `wkt_prescan_bounded`, `geojson_prescan_bounded` in `scripts/fuzz-smoke.sh` verdi; snapshot `catalog_v1_hostile_input_hardened_per_driver` verifica il valore atteso per ogni driver |
| **`SourceFootprint` snapshot + revalidation (best-effort)** | `SourceFootprint::snapshot() -> SourceFootprintSnapshot` con `digest()` **best-effort** accumulato entry per entry da `note_entry_visited` (XOR di FNV-1a, insensibile all'ordine) e `matches()` che confronta byte, entry e digest insieme; `PipelineBundle::into_scan_parts(expected)` accetta lo snapshot; il core rivalida **sempre** con preflight leggero — enumerazione completa delle entry + `max_input_entries` + size/mtime, senza parsing dei contenuti — e produce `PlenoraIoError::Contract(FootprintChanged)` se diverge. **Nessuna variante forte ratificata in Lotto 0** (niente content hashing, file identity o locking) | Test `footprint_digest_is_stable_for_the_same_entry_set`; `footprint_digest_is_order_insensitive`; `footprint_digest_detects_added_and_removed_entries`; `footprint_digest_detects_rename_size_and_mtime`; `footprint_digest_separates_paths_that_share_a_concatenation`; `snapshot_matches_only_when_bytes_entries_and_digest_agree`; `snapshot_roundtrips_through_serde_without_losing_the_digest`; `scan_with_matching_snapshot_succeeds`; `scan_with_stale_snapshot_returns_footprint_changed`; `scan_detects_added_and_removed_entries`; `scan_preflight_applies_max_input_entries`; `scan_observes_current_bytes_and_entries_not_snapshot_values` |
| **`observe_input` fabbrica unica del footprint** | `PipelineContext::observe_input(permit) -> Result<SourceFootprint, PlenoraIoError>`, senza altri parametri; consuma il permit per `move`; legato al context che lo ha emesso; byte, entry e digest vengono tutti dallo stato accumulato, non da valori dichiarabili; la pubblicazione e' **terminale**, quindi ogni entry successiva e' rifiutata; su `Err` lo stato resta `Collecting` | Doc test compile-fail: `SourceFootprint { .. }` non compila; test `observe_input_consumes_permit_and_yields_footprint`; `observe_input_with_permit_from_other_pipeline_is_rejected`; `observe_input_err_leaves_observed_input_not_observed`; `second_observation_is_rejected_and_keeps_the_published_footprint` |

Ogni test elencato **deve essere committato** insieme al codice
che lo copre. Il gate di release verifica presenza dei nomi di
test attesi (statica), non solo la loro riuscita.

## Diff delle sezioni modificate

Passata di correzione chirurgica, senza ampliare il documento.
Le vecchie API/sezioni normative sono state sostituite in place;
ogni tipo/API ha una sola definizione normativa.

Correzioni applicate in questa iterazione:

- **PipelineLimits**: eliminati i `pub` field (residuo INV-1);
  ora solo campi privati elencati testualmente + setter fluent
  + getter espliciti nella sezione API. `concurrent_operations`
  spostata su `ResourcePool`.
- **API Budget**: rimossi `into_read_options` /
  `into_write_options` dal model; introdotti tipi opachi
  `ReadBudgetParts` / `ScanBudgetParts` / `ConvertBudgetParts`
  / `WriteBudgetParts` che il model produce e le factory del
  core (`ReadOptions::builder(parts)` /
  `WriteOptions::builder(parts)`) consumano; `ConvertBudgetParts`
  espone `into_parts(self) -> (ReadBudgetParts, WriteBudgetParts)`.
  `OperationBudget` e le lease: tipi **pubblici ma opachi**, non
  ri-esportati dalla facade.
- **Permit**: rimossi `InputObservationToken` e
  `PreflightEvidence` dalle sezioni normative (restano solo in
  "Alternative scartate"); l'osservazione passa da
  `PipelineContext::observe_input(permit) ->
  Result<SourceFootprint, PlenoraIoError>`, unico canale di
  costruzione e registrazione, legato al context che ha emesso il
  permit (niente `observe_measurement` libera, niente
  `record_footprint`). `SourceFootprint` opaca, con `snapshot() ->
  SourceFootprintSnapshot { digest best-effort }` per il riuso in
  `scan()`.
- **Snapshot / revalidation**: `Dataset.scan()` richiede
  `SourceFootprintSnapshot` esplicito nelle `ScanBudgetParts`; il
  core lo rivalida **sempre** (preflight leggero + digest);
  divergenza → `PlenoraIoError::Contract(FootprintChanged)`. La
  garanzia resta **best-effort**: nessuna variante forte (content
  hashing, file identity, locking) e' ratificata in Lotto 0.
- **Dataset**: rimossa ogni menzione di gauge di concorrenza
  sul Dataset. La concorrenza vive nel `ResourcePool` del budget
  della singola scansione (INV-12 riscritto).
- **ResourcePool**: aggiunto come tipo opaco condiviso
  (memory/spill/concurrency). Un `PipelineBudget` senza pool ha
  gauge locali; senza pool la libreria **non** promette di
  limitare pipeline concorrenti (INV-12 chiarito).
- **`read_mode` mapping**: rimosso ogni riferimento a
  `ReadMode::from_native` automatico (residuo). Ogni driver
  dichiara esplicitamente `read_mode` in `const_new`; nessuna
  derivazione (INV-7).
- **Spill cleanup**: superato in S2. Il file di spool non ha nome,
  quindi non esistono orfani da spazzare e il cleanup — con lock,
  ownership, rimozione ricorsiva e i loro casi limite — non serve
  piu'. Vedi "File di spool: politica di sicurezza".
- **`hostile_input_hardened` per-driver**: eliminato il flag
  globale. Ogni `FormatDescriptor` dichiara la propria
  capability; `catalog` la emette entry-per-entry.
- **DTO test wire**: snapshot strutturale normalizza `message`
  prima del confronto; test separato per il testo di `message`.
  `ContractIdentifier` conservato come `Option<ContractIdentifier>`
  nel contesto strutturato dell'errore (`ErrorContext`), da cui il
  DTO deriva direttamente il campo `field` safe-by-construction; non
  compare fra gli ingressi consentiti di `PublicMessage`.
- **Editorial**: `&Limits` sostituito con `&PipelineLimits`
  nella tabella decisioni; conteggio step corretto a **13**
  (S0-S12 inclusi); ADR-IO 7 stato **Draft** fino a S0.

**Errata S2 (2026-08-16)** — chiuse in implementazione, non rinviate.
- Il ponte `PipelineContext::delegating_to_legacy` **non serve** ed e'
  rimosso dal piano. Lo `StagedSpool` di M2 e' scritto contro il
  `ResourceBudget` legacy, quindi in M2 un solo modello tocca i
  contatori: non c'e' nulla da ponteggiare e il doppio conteggio che
  il ponte doveva evitare non esiste. S4 migra le chiamate dello
  spool insieme al resto.
- La directory di spill 0700 con sweep degli orfani su lock e'
  sostituita da un file **senza nome** (`tempfile::tempfile_in`:
  scollegato all'apertura su Unix, `FILE_FLAG_DELETE_ON_CLOSE` su
  Windows). Nessun path che un altro processo possa aprire, nessun
  orfano da spazzare dopo un SIGKILL, nessuna finestra TOCTOU,
  nessun symlink da seguire. `PLENORA_SPILL_DIR` resta per scegliere
  il volume e fallisce chiuso se inutilizzabile.
- La quota esaurita non preempte piu' la scoperta dell'EOF: un
  dataset di N righe con quota N deve riuscire, e prima falliva.

**Errata S1.2 (2026-08-16)** — chiuse in implementazione, non rinviate.
- `observe_input(permit, bytes)` → `observe_input(permit)`. Anche i
  byte vengono ora dallo stato accumulato: erano l'ultima grandezza
  del footprint che il chiamante poteva dichiarare senza averla
  misurata, e governano `output_expansion_ratio`.
- `SourceEntry` distingue `metadata_size` (entra nel digest) da
  `charged_input_bytes` (conta verso `max_input_bytes`), con
  costruzioni separate `file()` e `directory()`: una directory conta
  come entry e addebita zero byte.
- L'osservazione e' una **state machine sotto mutex**
  (`Collecting { entries, total_bytes, digest }` → `Published`), non
  tre atomiche indipendenti. Conteggio, byte e digest si aggiornano
  insieme dopo tutti i controlli; la pubblicazione e' terminale.
- Il prelievo di `OutputBytes` usa un unico loop CAS che proietta il
  consumo e sottrae la quota sulla stessa osservazione.
- L'identita' di pipeline si alloca con incremento **checked**: un
  `fetch_add` che avvolge riassegnerebbe identita' gia' consegnate,
  rendendo il permit dell'una spendibile sull'altra.

**Errata S1.1 (2026-08-16)** — chiuse in implementazione, non rinviate.
- `observe_input(permit, bytes, entries)` → `observe_input(permit,
  bytes)`. Entry e digest vengono dal context che li ha accumulati:
  la firma precedente li rendeva parametri, quindi fabbricabili dal
  chiamante, e creava due sorgenti di verita' per lo stesso dato.
- `note_entry_visited()` → `note_entry_visited(entry: &SourceEntry)`.
  Il digest e' definito sull'insieme delle entry, e l'insieme delle
  entry e' esattamente cio' che quel metodo gia' attraversava: senza
  identita' il footprint avrebbe dichiarato N entry senza sapere
  quali, e il `SourceDigest` non sarebbe stato costruibile.
- `PipelineLimits` acquisisce `max_vertices: usize` e
  `effective_wkb_components()`. `--max-vertices` e' un flag vivo
  della CLI: senza questa quota la migrazione avrebbe allentato in
  silenzio un tetto che l'utente puo' stringere oggi.

**INV-5 (semantica memoria)** — sostituita.
- Prima: due varianti (bookkeeping-token-tenuto-dal-consumer +
  release-al-transfer).
- Dopo: **una sola** semantica, `InternalMemoryLease` rilasciata
  al transfer del batch al consumer. Rimosse tutte le frasi sul
  "drop del batch consumer" o su "consumer che tiene viva una
  lease insieme al batch". Aggiornato anche lo state model di
  "Lifecycle memoria" per riflettere la nuova semantica unica.

**INV-13 (dipendenza model→core, capability)** — sostituita.
- Prima: `InputObservationToken` + `PreflightEvidence` con
  costruttore pubblico. Falsificabile.
- Dopo: `InputPermit` opaco, non `Clone`, senza costruttori
  pubblici, emesso una volta dal `PipelineBudgetBuilder::build`
  dentro il `PipelineBundle` opaco, trasportato dalle parti e
  consumato per `move` dal `Source::into_path_checked` (che lo
  estrae con `ReadOptions::take_input_permit()`).
  `SourceFootprint` distinta come descrizione dell'input osservato.

**API modulo `plenora_io_model::budget`** — sostituita.
- Prima: model esponeva `ReadOptions`/`WriteOptions` +
  `PipelineBudget.into_read_options()` /
  `into_write_options()`.
- Dopo: model espone solo budget e permit; `ReadOptions` e
  `WriteOptions` migrate in `plenora-io-core::driver` con builder
  che accettano le parti opache del budget sotto bound **sealed**
  (`IntoReadParts`/`IntoWriteParts`) ed espongono `budget()` ai
  driver. `OperationBudget` e le lease restano tipi pubblici ma
  opachi, non ri-esportati dalla facade. `PipelineBundle` e' un
  tipo **opaco** (niente `{ budget, permit }`) e porta lui le
  `into_read_parts` / `into_scan_parts` / `into_convert_parts` /
  `into_write_parts`, cosi' budget e permit non sono incrociabili a
  mano. `SourceFootprint` si ottiene solo da
  `PipelineContext::observe_input(permit) ->
  Result<..>`, unico canale (niente
  `observe_measurement`/`record_footprint`).

**Scope dei budget (subsezione)** — sostituita.
- Prima: `Dataset` incapsulava un `read_budget` vivo con
  contatori cumulativi condivisi fra tutti i reader del Dataset.
- Dopo: `Dataset` inerte, conserva solo metadata +
  `SourceFootprint`. Ogni scansione richiede un nuovo
  `PipelineBudget` fornito dal consumer. Solo `convert` condivide
  un unico context fra reader e writer.

**ADR-IO 7 — file di spool** — sostituita due volte.
- Prima: `same-filesystem` obbligatorio; sweep con `kill(pid, 0)`.
- Poi: directory 0700/DACL con `PLENORA_SPILL_DIR`, cleanup su lock
  esclusivo e ownership, rimozione ricorsiva symlink-safe.
- **Ora (S2, attuato)**: file **senza nome**
  (`tempfile::tempfile_in`). Nessun path apribile da altri, nessun
  orfano da spazzare, nessuna finestra TOCTOU, nessun symlink da
  seguire: le proprieta' che le due versioni precedenti cercavano
  di ottenere con permessi e lock discendono dal fatto che il file
  non esiste nel namespace. `PLENORA_SPILL_DIR` resta per scegliere
  il volume e fallisce chiuso se inutilizzabile.

**Redazione errore DTO** — sostituita.
- Prima: un unico invariante "conformita' v1" con nota che
  message cambia.
- Dopo: **due invarianti distinti**: A "struttura wire invariata"
  (campi/ordine/tipi) e B "message intenzionalmente diverso". Test
  snapshot separati. Il campo `field` resta nome (`String`) nel
  wire, derivato dall'`Option<ContractIdentifier>` del contesto
  strutturato dell'errore, non da una variante del messaggio.

**`FormatDescriptor` e mapping `read_mode`** — sostituita.
- Prima: `read_mode` legacy derivato via
  `ReadMode::from_native(NativeReadMode)` con fallback
  arbitrario (`StreamingRandom` → `StreamingSequential`).
- Dopo: `read_mode` legacy **dichiarato esplicitamente** in
  `const_new` da ogni driver, preservando byte-per-byte il
  valore attuale (`StreamingSequential`, `StreamingColumnar` per
  GeoParquet, `Materializing` per FileGDB). Nessuna derivazione.

**Trattamento di L6** — sostituita.
- Prima: due opzioni (L6-A includi / L6-B escludi con
  `hostile_input_hardened: false`), raccomandazione L6-B.
- Dopo: **ratificato L6-A** come step S12 obbligatorio. Flag
  `hostile_input_hardened: true`. Nessuna alternativa.

**Ordine d'implementazione** — S12 aggiunto obbligatorio.
- Prima: 11 step (S0-S11) con L6 fuori Lotto.
- Dopo: 12 step (S0-S12), S12 = L6 pre-scansione, dipendenze
  S4+S6, in parallelo a S9.

**Stima aggiornata** — ricalcolata.
- Prima: 33-49 gg-persona (base) + 40-59 (con
  conformance/matrici/evidence) + note su L6-A opzionale.
- Dopo: **46-68 gg-persona** unico totale (include tutto:
  S0-S12 + conformance + matrici + evidence). Calendario:
  1 persona 12-16 sett., 2 persone 7-10 sett., 3 persone 5-8
  sett.

**Chiusura formale** — punto L6 semplificato.
- Prima: distinzione tra L6-A e L6-B nel criterio 2.
- Dopo: criterio 2 unico: L6 chiuso con codice + test + fuzz +
  flag `hostile_input_hardened: true`.

**Editorial**:
- `IoLimits` → `PipelineLimits` (unico nome normativo).
- `MemoryLease` → `InternalMemoryLease` in tutte le sezioni
  residue (Concorrenza, Scope, Lifecycle memoria).
- `M5` come tag di step confuso: sostituito con `S9` dove
  riferito agli errori strutturati (R4).
- Rimosso campo `id` duplicato in `FormatDescriptor`.
- Aggiornato riferimento finale `docs/PROPOSAL-L6-...` da
  "ortogonale, non incluso" a "ratificato come L6-A, S12
  obbligatorio".
- Aggiornata riga tabella decisioni "L6-B" a "L6 (ratificato A,
  S12)".

## Cosa non farò senza ratifica

- Non tocco codice, `release/*.json`, evidence base, proposal CLI/SDK.
- Non apro commit.
- Non produco altri documenti oltre a questo pacchetto.

## Riferimenti

- `docs/REVIEW-2026-08-15.md` — review originale.
- `docs/ROADMAP-1.1.0.md` — roadmap che ha dichiarato PR-1, PR-2,
  PR-3 (ora rinominati L0.4, L0.2, L0.7).
- `docs/adr/ADR-IO-7-streaming-vs-operation-atomicity.md` — draft
  ADR, ratificato dall'opzione A in questo pacchetto.
- `docs/PROPOSAL-CLI-SDK-facade.md` — sospeso finche' il Lotto 0
  non chiude.
- `docs/PROPOSAL-L6-progressive-wkt-geojson.md` — ratificato come
  opzione L6-A e incluso nel Lotto 0 come step S12 obbligatorio.
