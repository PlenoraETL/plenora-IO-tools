# Piano 4.1.0 — allineamento con database-tools, residui e dipendenze

## Che cos'è questo documento, e che cosa non è

È il piano di lavoro della 4.1.0: i residui che la 4.0.0 ha lasciato aperti, il
costo delle dipendenze e della manutenzione, e gli strumenti di qualifica da
rendere sostenibili. Nasce dalle decisioni concordate il 15 settembre 2026,
subito dopo la pubblicazione della 4.0.0, e le integra nella documentazione
versionata: prima viveva in `.s9-checkpoint/pianificazione/`, ignorata da Git
per non modificare l'albero che la campagna stava misurando.

La richiesta successiva dell'utente aggiunge il confronto con
`plenora-database-tools`, precisando che interessa la **maturità del codice**.
Il piano distingue quindi gli interventi ingegneristici M1–M5 dalle possibili
estensioni d'interoperabilità A1–A7. Una pipeline file → database → file è un
caso d'uso utile per provare i confini; non giustifica da sola una riscrittura
dello SDK o l'aggiunta di nuove superfici. Questo aggiornamento analizza
sorgenti e controlli: non implementa l'allineamento e non qualifica la pipeline.

Non aggiunge condizioni alla 4.0.0, che è pubblicata. Non promette l'assenza di
fork o di duplicazioni: promette che ogni voce si chiuda con **una soluzione
provata oppure una motivazione precisa**. Un elenco di rimozioni sarebbe una
promessa; un elenco di esiti è un piano.

Il predecessore è [docs/PIANO-4.0.0.md](PIANO-4.0.0.md), che resta in questo
docset — vedi «Un documento che non può scadere» più sotto.

## Allineamento con database-tools — maturità e interoperabilità

### Baseline verificata il 15–16 settembre 2026

Il confronto usa copie separate dei repository, senza aggiornare checkout di
lavoro esterni né il pin adottato da IO-tools. I riferimenti sono immutabili:

| Componente | Revisione confrontata | Versione e perimetro |
|---|---|---|
| IO-tools | `7fe1311` | Workspace `4.0.0`, con Q2a e R6; nessuna modifica al prodotto rispetto alla release `v4.0.0` |
| database-tools | [`00d7613402e87a449a36e0ff7d91e7710999b822`][db-base] | Workspace `4.2.0`; sorgenti Rust, CLI, SDK Python e gate. La release GitHub [`py-v4.2.0`][db-release] risulta pubblicata il 14 settembre; i suoi asset non sono stati scaricati o qualificati in questa analisi |
| Contratti condivisi | `453c8d1ff2eb260840e6cedc033a2b76b58a0b9e` e `5d151078142e7ed49d831659ee8be92a4975f80b` | Rispettivamente pin IO e pin database; confronto del contenuto, non dei soli identificatori |

Il 16 settembre la revisione remota database è stata riconfermata invariata.
Le modifiche di sviluppo successive alla baseline IO restano fuori da questa
fotografia: ogni intervento ricontrolla il codice corrente prima di agire.

### Valutazione della maturità del codice

**Non emerge una superiorità uniforme di database-tools.** La valutazione è
per proprietà osservabile, non per quantità di funzionalità o numero di
versione. Database offre esempi utili di modello tipizzato e controllo delle
duplicazioni; IO ha già difese più esplicite su alcuni confini del prodotto.
La 4.1.0 deve adottare i miglioramenti che mancano, conservando quelli già
presenti. Non deve «raggiungere la 4.2» copiandone l'architettura.

| Proprietà | Database-tools: evidenza | IO-tools: confronto e giudizio |
|---|---|---|
| Responsabilità e modello unico | Separazione core, renderer SQL, engine, provider e binding; [`check_relational_ir.py`][db-ir-guard] impedisce la doppia definizione del modello query. Controllo eseguito verde sul clone | Model/core/driver/CLI sono già distinti. Il principio utile è impedire duplicazioni del significato fra libreria, CLI e SDK, non importare un IR SQL. Verificare i percorsi concreti prima di creare un nuovo layer |
| Stati rappresentati dai tipi | [`Observation<T>`][db-observation] distingue `NotMeasured` da `Observed(T)`; l'adattatore JSON resta separato dalla reflection pubblica. La [guardia dei metadati][db-metadata-guard] è stata eseguita verde | IO distingue già CRS risolto/irrisolto/assente e conserva `scansione_completa`. Il limite noto è trasportare quest'ultima distinzione sul filo, C3. Il modello database è utile dove una combinazione di flag/valori opzionali ammette stati contraddittori; non prova che occorra riscrivere tutti i tipi IO |
| Panici e aritmetica | Manifest con `unsafe_code = forbid`, Clippy pedantic/nursery e overflow checks in release. Il workflow Rust esaminato non impone il gruppo di lint anti-panic esplicito di IO | IO aggiunge `unwrap_used`, `expect_used`, `panic`, `unreachable`, `todo`, `unimplemented` negati su librerie **e binari** distribuiti, e barriera ai decoder di terzi. Su questa proprietà IO è già più sorvegliato; Q2 prova che ciò non elimina i difetti upstream |
| Risorse e lifecycle | Budget condivisi e lease in `resource.rs`, cancellazione con deadline in `cancellation.rs`. Tuttavia il caricatore CLI IPC accumula i batch in `VecDeque` prima del consumo e `_to_ipc_bytes` accumula l'intero stream Python | IO ha preflight con `InputPermit`, budget prima della materializzazione e spool con quota. L'esistenza di un tipo `ResourceBudget` non dimostra che ogni ingresso lo rispetti: seguire acquisizione, rilascio e cancellazione lungo il percorso. Non adottare le materializzazioni del riferimento come modello di robustezza |
| Coerenza fra dichiarazione e comportamento | Il catalogo annuncia file/stream; il percorso CLI esaminato usa lettore/scrittore file. È un rilievo statico da riprodurre, non una qualifica negativa dell'intero componente | Le due deviazioni IO C1/C2 mostrano che anche qui i descrittori non bastano. La maturità cresce quando un test attraversa l'operazione dichiarata e confronta l'esito reale, non quando aumenta il numero di capability |
| Test e copertura | Test Rust separati: controllo eseguito verde su 193 sorgenti di prodotto. Budget di coverage distinti: righe Rust prodotto 39%, binding nativo 20%, SDK Python 69% e branch Python 55% | IO ha separazione verificata delle prove, regressioni ostili, sonde delle guardie e soglia righe libreria 80%. Sono **soglie configurate**, non percentuali misurate in questa analisi; i denominatori differiscono. Non ne segue una classifica 80 contro 39 né una certificazione di qualità |
| Manutenibilità locale | `plenora-database-cli/src/main.rs`: 3.360 righe fisiche; SDK Python `__init__.py`: 1.413. Il budget di dimensione è uguale alla misura corrente | IO ha `plenora-io-tools/src/lib.rs`: 2.362 righe, `driver.rs`: 2.236, `budget.rs`: 2.028. Entrambi hanno concentrazioni da esaminare. Le dimensioni indicano dove leggere, non dimostrano da sole un difetto; lo split deve separare responsabilità, non soltanto abbassare un contatore |
| Controlli mantenuti ed eseguiti | Stato generato dai sorgenti; self-test di corrispondenza sweep/CI. Per lo SHA esaminato GitHub riporta 18 corse concluse con successo, fra push, release e schedule | IO ha già registri, docset e riconciliazione dei passi, ma R2–R5 restano debiti espliciti. Sono verificati gli stati delle corse database, non rianalizzati tutti i log/artefatti: un workflow verde non dimostra che ogni ramo o provider sia stato esercitato |

Fonti delle proprietà di verifica: [workflow Rust database][db-ci],
[budget coverage database][db-coverage], [self-test CI][db-ci-tests] e
[corsa Rust sullo SHA confrontato][db-ci-run]. Per IO:
`.github/workflows/ci.yml`, `scripts/check_test_layout.py`,
`scripts/check_coverage_exclusions.py`, `scripts/check_permit_boundary.py` e
`assurance/registries/code-size-budget.json`.

Sono stati eseguiti sul clone database **quattro controlli statici**:
`check_relational_ir.py`, `check_typed_metadata.py`, `check_test_layout.py` e
`code_size.py --check`, tutti verdi. Il contatore database misura 100.890 righe
fisiche / 83.625 di codice, includendo Rust e package Python nel suo perimetro;
il contatore IO registra 45.374 righe nel proprio perimetro Rust. Non si
confrontano quei totali come se avessero lo stesso denominatore. Non sono
stati eseguiti build, suite Rust/Python complete, benchmark o prove live di
database-tools. Una guardia che cerca definizioni o marker nel sorgente prova
quell'invariante strutturale; non sostituisce una prova del comportamento.

### Interventi di maturità per la 4.1.0

| ID | Intervento | Criterio di chiusura |
|---|---|---|
| M1 | Rivedere le responsabilità dei tre moduli IO sopra nominati | Mappa delle responsabilità e dei dipendenti; separazioni solo dove riducono accoppiamento o duplicazioni. Export pubblici preservati, prove sui chiamanti esterni e stessi esiti/effetti. È ammesso mantenere un modulo con motivazione; nessun obiettivo cosmetico di righe per file |
| M2 | Verificare che conoscenza, assenza e valore siano distinguibili nel modello | Censimento mirato dei metadati geometrici e dei descrittori; fixture distinguono non misurato, misurato vuoto e valore presente. Stati impossibili impediti o rifiutati al confine. C2/C3 restano la sede delle modifiche sul filo; nessuna rottura dei tipi pubblici per imitare `Observation<T>` |
| M3 | Provare l'efficacia delle guardie sui percorsi pubblici | Per ogni lacuna individuata: ingresso concreto, percorso fino alla guardia, controprova valida, errore atteso e conseguenza evitata. Priorità a lettura/scrittura IPC e ai percorsi di publish, con budget esaurito, cancellazione durante attesa e errore tardivo. Riutilizzare le regressioni esistenti; nessun aumento del solo conteggio dei test |
| M4 | Legare capability, documentazione e prove senza duplicare le fonti | Censimento delle proprietà dichiarate da catalogo/CLI/SDK e del test che le esercita; distinzione tra compilato, eseguito, saltato e non applicabile. Stato derivato dai registri esistenti e verifica della corrispondenza con CI/checkpoint. R3–R5 governano identità delle corse e riuso; nessun secondo sistema di assurance |
| M5 | Valutare le dipendenze come parte della manutenibilità del prodotto | L1–L4, D1–D7 e F1–F7 producono esiti motivati con API/feature, regressioni e grafo effettivo. Riduzione del lavoro da mantenere dimostrata per delta rimossi o catene semplificate; nessuna equivalenza fra meno crate, meno byte e maggiore qualità |

### M1 — mappa delle responsabilità e dei dipendenti

Analisi eseguita il 16 settembre 2026 sui sorgenti, senza modifiche al codice.
Mappa registrata nel commit `89b70ce`. L'analisi iniziale è completata;
la decisione su `lib.rs` e `driver.rs` resta aperta fino alle misure di
accoppiamento descritte sotto. Per `budget.rs` l'esito proposto è mantenerlo.
Le dimensioni sono state **rimisurate**, non riprese dal confronto: 2.362 righe
in `plenora-io-tools/src/lib.rs`, 2.236 in `plenora-io-core/src/driver.rs`,
2.028 in `plenora-io-model/src/budget.rs`.

Un primo fatto va detto perché cambia la lettura: **questi tre non sono i tre
moduli più grandi.** `driver-shp/src/lib.rs` ne ha 3.858, `driver-filegdb`
3.477, `driver-geoparquet` 2.873. La differenza non è la dimensione ma la
natura: un driver concentra un formato, cioè una responsabilità che nessuno
chiede di separare; questi tre stanno su percorsi trasversali, e lì la
concentrazione può nascondere responsabilità diverse messe insieme. È il motivo
per cui la dimensione indica dove leggere e non che cosa correggere.

#### Chi dipende da che cosa

| modulo | altri crate dipendenti nel workspace | superficie dichiarata |
|---|---|---|
| `plenora-io-core/src/driver.rs` | **13** — ogni driver, più `plenora-bench` e `plenora-io-tools` | i quattro trait sono il contratto interno fra core e driver |
| `plenora-io-model/src/budget.rs` | **14** — come sopra, più `plenora-io-core` | `PipelineBudget`, permessi e lease attraversano tutto |
| `plenora-io-tools/src/lib.rs` | **nessuno** | gli ingressi dichiarati in `operazioni` chiamano le implementazioni qui |

L'asimmetria indica l'ampiezza dei consumatori da considerare nelle verifiche,
non quanti crate debbano essere modificati. Una separazione interna che
preservi percorsi e firme può evitare modifiche ai tredici o quattordici
dipendenti. Per `lib.rs`, zero altri crate nel workspace non esclude il
binario dello stesso pacchetto, i test e i consumatori esterni.

#### `plenora-io-tools/src/lib.rs` — nove responsabilità, zero export contrattati

| responsabilità | righe |
|---|---|
| preambolo e costanti | 218 |
| buste e identità | 244 |
| struttura `Cli` | 165 |
| parsing degli argomenti | 369 |
| documento capability | 169 |
| comandi `catalog`, `inspect`, `layers` | 305 |
| comandi `write`, `read`, `convert` | 537 |
| dispatch e `run` | 201 |
| hook di panico | 150 |

Gli ingressi Rust dichiarati sono definiti in un altro file.
`contracts/superficie-rust.json` nomina sei export, e sono tutti in
`operazioni.rs` — `plenora_io_tools::operazioni::{catalog, inspect, layers,
read, write, convert}`, 328 righe che avvolgono i `cmd_*` di `lib.rs`. I `pub
fn cmd_read` e simili sono pubblici per Rust ma non contrattati, e il gate
`check_superficie_rust.py` prova i sei export contro l'archivio distribuito, non
questi.

Una separazione può quindi mantenere gli ingressi dichiarati in `operazioni`,
ma non è per questo sicura: i wrapper dipendono dai `cmd_*` e possono cambiare
comportamento anche a firme invariate. Il manifesto esclude esplicitamente
gli export non elencati dalla compatibilità promessa; questo non elimina la
necessità di verificare gli esiti dei percorsi dichiarati e della CLI.
La tabella suggerisce possibili separazioni, da valutare misurando lo stato
condiviso. Tre moduli figli (`busta`, `operazioni`, `radici`) mostrano che
l'estrazione è già la pratica del crate.

Resta da verificare, prima di proporla: che i `cmd_*` non condividano stato
mutabile con il parsing oltre a `Cli`, e che l'hook di panico non dipenda
dall'ordine di inizializzazione. Nessuna delle due è stata misurata qui.

#### `plenora-io-core/src/driver.rs` — la concentrazione è in un punto solo

| responsabilità | righe |
|---|---|
| `Source` e `Sink` | 187 |
| `ReadOptions` e `WriteOptions` | 244 |
| cancellazione e barriera arrow | 119 |
| i quattro trait (`FormatDriver`, `OpenDatasetHandle`, `LayerReader`, `FormatWriter`) | 181 |
| preflight e limiti di scrittura | 116 |
| **validazione della scrittura** | **1.208** |
| righe rifiutate: diagnostica | 122 |

Più della metà del file sta in una sezione, e dentro quella se ne distinguono
tre gruppi di responsabilità, il cui accoppiamento resta da misurare:

- **pianificazione della perdita** (~350 righe): `planned_write_loss`,
  `stato_per_il_piano`, `RappresentazioneDelCrs`, `assess_write_contract` —
  calcola che cosa la scrittura perderà, prima di scrivere;
- **macchinario della scrittura** (~340 righe): `LimitedWriter`,
  `WriteBatchResources`, `GeometryValidation` — il writer con le risorse
  limitate;
- **validazione per riga e per batch** (~450 righe): `validate_geometry_batch_at`,
  `nullability_violations`, `inspect_geometry_row` e gli errori di rifiuto.

I quattro trait sono il contratto fra core e driver, e ogni driver li importa:
cambiarne percorsi o firme può richiedere modifiche ai consumatori. I tre gruppi della validazione,
invece, sono raggiunti attraverso `with_write_validation`, una sola funzione
d'ingresso. Un'eventuale separazione deve preservare quella firma e il
comportamento; la convenienza dipende dalle chiamate fra i gruppi.

Il crate ha già sei moduli figli (`capabilities`, `descriptor`, `loss`,
`publish`, `request`, `registry`) e `driver/` ne contiene altri tre
(`batch_worker`, `reader_adapters`, `spool`): l'estrazione è la pratica
corrente, e questi tre gruppi sono candidati coerenti con essa.

#### `plenora-io-model/src/budget.rs` — dieci responsabilità, e nessuna isolata

| responsabilità | righe |
|---|---|
| preambolo | 235 |
| identità della sorgente: `SourceEntry`, `SourceDigest` | 327 |
| limiti dichiarati: `PipelineLimits` | 159 |
| pool e prenotazioni: `ResourcePool` | 100 |
| osservazione dell'input: `ObservedInput`, `InputPermit`, footprint | 214 |
| contesto della pipeline | 355 |
| budget e bundle | 154 |
| contatori e budget d'operazione | 133 |
| lease: memoria, spill, concorrenza | 206 |
| parti di lettura | 145 |

È il modulo con più responsabilità nominabili e la distribuzione più piatta:
nessuna sezione domina, e ventuno tipi pubblici escono da qui verso quattordici
crate. Le parti si tengono per costruzione — `InputPermit` esiste perché il
footprint sia osservato una volta sola, i lease perché il pool sappia quando
restituire, il contesto perché i contatori abbiano un posto dove vivere — e una
separazione le distribuirebbe senza ridurre l'accoppiamento, perché
resterebbero a chiamarsi fra loro.

**Esito proposto per M1 su questo modulo: mantenerlo, con motivazione.** Il
criterio lo ammette esplicitamente, e la ragione è misurata: qui la dimensione
viene dal numero di concetti che il budget deve tenere insieme, non da
responsabilità estranee finite nello stesso file. Un taglio abbasserebbe il
contatore e lascerebbe l'accoppiamento dov'è.

#### Le due misure mancanti — 16 settembre 2026

##### `lib.rs`: stato condiviso e inizializzazione

**Stato globale mutabile: nessuno.** Gli `static` del file sono tutti
`&'static str`, il crate dichiara `#![forbid(unsafe_code)]`, e non compaiono
`OnceLock`, `thread_local`, `Mutex`, `RwLock` né atomici. L'unico atomico del
crate sta in `segnali.rs`, è un `Arc<AtomicBool>` locale a una funzione, e quel
modulo è già separato.

**Inizializzazione: due ordini, entrambi contenuti.** `installa_hook_silenzioso`
installa un hook di panico per l'intero processo, ed è chiamato **solo da
`main.rs`**: è una preoccupazione del binario, non della libreria, e una
separazione di `lib.rs` non la tocca. Dentro `run` c'è un ordine vero — il
gestore dei segnali si arma **prima** del dispatch, e se non si installa il
comando non parte — ma vive per intero dentro quella funzione.

**Stato condiviso fra parsing e comandi: `Cli`, e nient'altro.** È una struttura
di dati — posizionali, flag, tre mappe di opzioni, `PipelineLimits` — con un
solo elemento vivo, il `CancellationToken` che `parse_legato` vi inietta. I
comandi la ricevono per riferimento e non scrivono nulla che il parsing rilegga.

Il grafo interno, contando i riferimenti che attraversano i confini delle
sezioni, è **direzionale**: `dispatch → comandi → parsing → buste`, con una sola
freccia all'indietro (`parsing → run`, un riferimento in un commento). La
sezione «buste e identità» è la base che tutti usano; «capability» ha **un solo**
riferimento in entrata dai comandi.

**Proposta: separare due sezioni, non tutto.**

- `capability` (169 righe, un riferimento in entrata): è il documento che i gate
  del contratto leggono, e isolarlo rende esplicito il suo perimetro;
- `parsing` e `Cli` (534 righe): è dove vive la conformità a CLI-2.0, e oggi un
  comando può raggiungerne gli aiutanti senza che nulla lo segnali.

Il beneficio concreto è quello, e va detto per quello che è: **la separazione
non riduce un accoppiamento alto — l'accoppiamento è già basso e a senso unico.
Rende una dipendenza inversa visibile**, perché diventerebbe un `use` scritto in
cima a un file invece di una chiamata nello stesso modulo. Non la **impedisce**:
un `use` si aggiunge. Impedirla richiederebbe un vincolo verificato — un gate
che rifiuti quella direzione — e non è oggetto di questa voce. Restano insieme comandi, dispatch e buste,
che si chiamano davvero fra loro.

Che gli export contrattati stiano in `operazioni.rs` dice una cosa sola: i sei
percorsi pubblici restano quelli, perché sono definiti là e non qui. **Non
garantisce né le firme né il comportamento**: i wrapper chiamano i `cmd_*`, e
un'estrazione che ne cambiasse una firma li romperebbe alla compilazione, mentre
una che ne cambiasse il comportamento lo lascerebbe passare intatto ai chiamanti
esterni. Il primo caso lo prende il compilatore, il secondo solo le prove.

##### `driver.rs`: chiamate e tipi condivisi fra i gruppi di validazione

Misura dei riferimenti che attraversano i tre confini proposti, contando anche
le occorrenze nei commenti — il conteggio sovrastima l'accoppiamento invece di
nasconderlo:

| da → a | riferimenti |
|---|---|
| B macchinario → C validazione per riga | **1** (`validate_geometry_batch_at`) |
| C → A, A → B, A → C, B → A, C → B | **0** |

I tre riferimenti che la prima passata segnava come «C → A» sono falsi
positivi, verificati uno per uno: `actual_dimensions.nome()` è un metodo di un
altro tipo, e «categoria» compare in un commento.

I tre gruppi dipendono invece dai tipi condivisi del resto del file — descrittori,
contratti, `saturating_u64` — che resterebbero dove sono: A ne usa 23
riferimenti, B 51, C 41.

**Proposta: separare i tre gruppi.** Il beneficio è preciso, e non è il
contatore di righe. I tredici crate che dipendono da questo modulo importano **i
quattro trait**; non importano nulla delle 1.208 righe di validazione. Oggi le
due cose stanno nello stesso file, e chi legge il contratto fra core e driver ne
legge 2.236. Separando, il contratto resta in un file che lo contiene e basta, e
l'unica chiamata che attraversa — `B → C` — diventa un `use` visibile.

**Dove verificare le regressioni, che non è la stessa cosa di dove intervenire.**
I tredici crate dipendenti non vanno modificati: i trait non cambiano firma.
Sono il posto dove una separazione sbagliata si vedrebbe, cioè dove eseguire le
prove — le suite dei driver, che esercitano scrittura, perdita dichiarata e
rifiuto di righe attraverso quei trait.

##### `budget.rs`

Confermato il mantenimento, con la motivazione già registrata: dieci
responsabilità, distribuzione piatta, ventuno tipi pubblici, e parti che si
tengono per costruzione. Nessuna nuova misura lo contraddice.

#### M1 — completata

| | |
|---|---|
| analisi e mappa | `89b70ce`, `1c53fb3`, `25b2174` |
| estrazione in `tools` | `9017e27`, corretta da `c1ecdb7` |
| estrazione in `core` | `76bed7e` |

`plenora-io-tools/src/lib.rs` passa da 2.362 righe a 1.779, con `cli.rs` (307) e
`capability.rs` (325). `plenora-io-core/src/driver.rs` passa da 2.236 a 1.110,
con `perdita_pianificata.rs` (370), `scrittura_limitata.rs` (372) e
`validazione_geometrie.rs` (474). `budget.rs` resta com'è.

L'API esterna non cambia: i sei export contrattati stanno in `operazioni.rs`,
non toccato, e la superficie del crate non guadagna né perde un elemento. Ciò
che si è allargato è l'accesso **dentro** il crate, dove costanti e aiutanti
sono passati da privati a `pub(crate)`.

**Verificato dalle prove**: 47 suite e 1.078 prove sull'intero workspace, zero
fallimenti; `equivalenza_superfici` confronta le sei operazioni Rust con la CLI.
fmt, clippy con `-D warnings`, disposizione delle prove, commenti, registro
delle dimensioni.

**Controllato nel diff**, dove le prove non arrivano: che in
`scrittura_limitata` nessun `drop` sia stato aggiunto o tolto, nessun ritorno
anticipato spostato, nessuno scope allargato o ristretto, e che la sequenza
staging → commit → fallimento sia rimasta nell'ordine di prima.

##### La controprova su `FLAG_FORMATO`

L'estrazione in `tools` ha prodotto un cambiamento di comportamento silenzioso:
`FLAG_FORMATO => {` senza la costante importata è un **binding** che cattura
tutto, non un confronto, e il compilatore lo dice soltanto con un avviso.

Che la regressione fosse coperta non è stato dedotto: reintrodotto il difetto,
**otto prove di `consegna_arrow` diventano rosse**. Il pattern usa ora
`crate::FLAG_FORMATO`, così un nome che non risolve è un errore e non un
catch-all.

##### Copertura dell'errore tardivo: una prova, non due

Il resoconto dell'estrazione in `core` ne citava due, e **una delle due non lo
dimostra**. Il nome di una prova e l'assenza di residui dicono che il rifiuto
non lascia sporcizia; non dicono **quando** il fallimento avvenga, e un
fallimento che precede la scrittura non esercita il percorso tardivo.

- `sigint_annulla_la_conversione_e_non_lascia_staging` **lo dimostra**: attende
  che il file di staging compaia prima di mandare il segnale, e fallisce
  esplicitamente se il figlio esce prima — «la conversione non ha raggiunto la
  creazione del writer». Il fallimento è quindi a scrittura iniziata.
- `una_scrittura_rifiutata_non_lascia_destinazione_ne_payload` **non lo
  dimostra**: rifiuta a `driver.create`, cioè alla validazione del piano, prima
  che la scrittura cominci. Il test stesso lo dice: «alcuni driver accettano il
  piano e rifiutano alla scrittura: il rifiuto è comunque coperto, ma qui non
  c'è errore da ispezionare».
- `una_deadline_di_un_millisecondo_ferma_la_conversione_prima_del_publish` non
  stabilisce il momento: verifica che la destinazione resti vuota, ma un
  millisecondo può scadere prima che il writer esista.

Resta quindi **una** prova che copre il fallimento a scrittura iniziata, e per
un solo modo di fallire — il segnale. Budget esaurito e errore del backend a
metà scrittura non hanno una prova che ne fissi il momento: è una lacuna
concreta, e appartiene a M3, che è la voce sull'efficacia delle guardie.

#### Che cosa questa analisi non dice

Le due misure che mancavano sono state fatte, e i confini proposti reggono al
criterio che le motivava: i tre gruppi di `driver.rs` si scambiano **una**
chiamata, e in `lib.rs` non c'è stato condiviso oltre a `Cli`.

Resta però la distanza fra «i confini sono netti» e «una separazione preserva il
comportamento». Un'estrazione può cambiare l'ordine in cui le cose accadono, o
il momento in cui un `Drop` rilascia una risorsa, senza che nessun conteggio di
riferimenti lo mostri — e `LimitedWriter` e i lease del budget sono
precisamente il genere di codice dove questo conta. Neanche i sei wrapper di
`operazioni.rs` lo garantiscono: passano attraverso i `cmd_*`, quindi un
cambiamento di comportamento arriverebbe intatto ai chiamanti esterni.

Quello che una separazione dovrà quindi portare con sé non è un altro conteggio:
è l'esecuzione delle suite dei tredici crate dipendenti, con gli stessi esiti e
gli stessi effetti osservabili — pubblicazione atomica compresa. E resterà una
parte affidata alla lettura del diff: che l'ordine delle operazioni e il punto in
cui una risorsa viene rilasciata siano quelli di prima è una proprietà che le
prove coprono dove hanno un caso, e che altrove si verifica guardando.

Non è stato eseguito alcun refactoring, nessun export è cambiato, e nessuna
prova è stata aggiunta o tolta.

### M2 — censimento dei tre stati

Censimento del 16 settembre 2026 sui sorgenti. Nessuna modifica al codice:
questo blocco riporta come gli stati sono rappresentati, dove si producono, come
li leggono i consumatori e quali prove li fissano.

#### La distinzione che governa il censimento

Non tutti i campi devono ammettere tre stati, e confonderli sarebbe il difetto
opposto a quello che si cerca. **Un descrittore dichiara, un contratto misura**:
per una dichiarazione statica «non misurato» non esiste, perché nessuno stava
misurando. `FormatDescriptor` ha diciassette campi opzionali — `write_mode`,
`write_determinism`, `spec_version_supported`, `write_capabilities` — e in tutti
`None` significa «questo formato non ha quella capacità», non «non l'ho
osservata». Sono due stati per costruzione, e vanno lasciati a due.

La domanda dei tre stati vale dove c'è una **misura**: il contratto geometrico e
le diagnostiche di riga.

#### `geometry_types` più `scansione_completa` — tre stati, e il terzo è recente

| stato | rappresentazione |
|---|---|
| valore presente | `geometry_types` non vuoto |
| misurato vuoto | `geometry_types` vuoto **e** `scansione_completa = true` |
| **scansione non completa** | `geometry_types` vuoto **e** `scansione_completa = false` |

Il campo lo dice da sé: «vuoto **da solo** non dice quale dei due stati sia». È
la coppia a decidere, e `false` è il valore prudente — chi non ha percorso la
sorgente non può affermare un'assenza.

La terza riga va letta per quello che il nome dice, e non di più.
`scansione_completa = false` significa **scansione non completa**, che non
equivale a «non misurato»: può esserci stata una misura parziale, e
`geometry_types` può perfino non essere vuoto. Il campo dichiara che
l'enumerazione non è esaustiva, non che non sia avvenuta. Ai fini della
decisione a valle le due cose coincidono — un elenco non esaustivo non si può
dichiarare come esatto — ma coincidono nell'uso, non nel significato.

**Dove si produce**: la passata di inferenza di GeoJSON lo valorizza a fine
file; `set_exact_geometry_types` lo pone a `true`.
**Chi lo consuma**: `validate_write` accetta il secondo stato — un'assenza
accertata non ha niente da dichiarare — e rifiuta il terzo.
**Prove**: `conformance_tests.rs` esercita entrambi gli stati su **ogni** driver
scrivibile, con `scansione_completa` come unica differenza fra i due piani.

**Il limite è sul filo, non nel modello**: `l_assenza_accertata_non_sopravvive_al_filo`
fissa il prezzo — la stessa sorgente si consegna al primo giro e viene rifiutata
al secondo, perché `ARROW-VOCABULARY-1.0` si dichiara chiuso e
`types_declaration` ammette i soli `exact`, `mixed`, `unresolved`. È C3, e
dipende dalla decisione 0006: non è una correzione che questo blocco possa fare.

#### `CrsResolution` — tre stati in un enum, senza campi impossibili

`Resolved(ResolvedCrs)`, `DeclaredButUnresolved(RawCrs)`, `Missing`.

**Sono tre stati utili, ma non sono la terna di questo censimento.** La terna è
«non misurato / misurato vuoto / valore presente»; questa è «risolto /
dichiarato ma non risolto / assente». Le due si sovrappongono solo in parte:
`Missing` dice che una dichiarazione non c'è, non se qualcuno sia andato a
cercarla. Il censimento la include perché è la modellazione dell'incertezza che
il contratto geometrico porta accanto ai tipi, non perché risponda alla stessa
domanda.

Il tipo impedisce lo stato impossibile per costruzione: un CRS non risolto non può
essere letto come un `ResolvedCrs` valido, perché non c'è un `ResolvedCrs` da
leggere. La documentazione del tipo lo dice: «evita di rappresentare `unknown`
come se fosse un `ResolvedCrs` valido».

`DeclaredButUnresolved` è lo stato che molte modellazioni perdono: la sorgente
**ha dichiarato** qualcosa che non sappiamo risolvere, e il dichiarato si
conserva invece di essere appiattito su `Missing`.

**Prove**: `unresolved_geodata_is_preserved_in_typed_error`,
`projjson_without_identifier_is_a_typed_unresolved_crs`,
`missing_geodata_requires_explicit_assumption`.

Su `Missing` resta una domanda che il censimento non chiude con una prova: due
soli driver lo producono — `driver-geoparquet` e `driver-ipc` — e in entrambi i
casi è «la sorgente non dichiara un CRS», non «non ho guardato». Che non esista
un percorso in cui `Missing` significhi il secondo non è stato **dimostrato**:
è stato osservato sui due punti di produzione.

#### `KnownOrUnknownCount` — il modello fatto per intero

`Known { value }` contro `Unknown`, e con `value: 0` i tre stati ci sono tutti:
non so quante righe, so che sono zero, so che sono `n`. È usato dai quattro
conteggi di `RowDiagnosticWriteOutcome`, dove la differenza fra «zero righe
rifiutate» e «non so quante ne siano state rifiutate» è precisamente ciò che un
consumatore deve poter distinguere prima di dichiarare un esito.

Accanto c'è `WriteDiagnosticStateCounts`, che porta gli stessi quattro nomi come
`u64` nudi: è l'aggregato **dopo** che l'incertezza è stata risolta, e i due
tipi non vanno confusi.

#### `srid: Option<i32>` — due stati, e la verifica ha corretto una mia misura

`None` significa «nessun SRID nativo distinto o aggiuntivo rispetto al CRS
risolto». Nove driver su dieci lo valorizzano; l'unico che non lo tocca è
`driver-ipc`, e la ragione è che il vocabolario Arrow non ha una chiave dove
metterlo — `None` lì è «il formato non ha posto per dirlo», che è ancora una
dichiarazione e non una misura mancata.

Una prima misura diceva **zero** driver, ed era sbagliata: il criterio cercava
`= Some(...)` e `driver-gpkg` scrive `geometry.srid = i32::try_from(srs_id).ok()`.
Va detto perché il censimento vale quanto il modo in cui è stato fatto, e un
criterio che cerca una forma sintattica invece di un effetto trova quello che la
forma ammette.

#### `native_metadata: BTreeMap<String, String>` — mappa vuota, due letture

È l'unico campo dove la domanda resta aperta senza un compagno che la chiuda.
Una mappa vuota può significare «la sorgente non porta metadati nativi» o «questo
driver non ne raccoglie», e nulla nel tipo le separa. Sei driver la popolano —
`csv`, `dxf`, `filegdb`, `geoparquet`, `gpkg` e altri — con chiavi namespaced
come `gpkg.geometry_type_name` e `filegdb.ogr_geometry_type`.

**Perché non propongo di correggerlo.** La conseguenza osservabile non c'è: il
campo non decide nulla a valle. La perdita di metadati nativi non è dedotta dal
confronto fra due mappe — sarebbe quella la lettura che l'ambiguità
falserebbe — ma **riportata dai driver** come categoria di perdita, che
`FidelityReasonCode::NativeMetadataLoss` poi classifica. Un driver che non
raccoglie metadati non dichiara nemmeno di averli persi.

Aggiungere un compagno tipo `scansione_completa` costerebbe un campo pubblico in
un tipo che attraversa dieci driver, per una distinzione che oggi nessun
consumatore osserva. Se un consumatore futuro dovesse dedurre l'assenza dalla
mappa vuota, allora servirebbe — e questa riga è il posto dove ritrovare la
ragione.

#### Esito

Nessuna ambiguità con conseguenza osservabile è stata trovata **nei percorsi
esaminati**. La terna del censimento è modellata per intero in due posti: la
coppia `geometry_types` / `scansione_completa`, e `KnownOrUnknownCount` nelle
diagnostiche di riga. `CrsResolution` porta una terna diversa e altrettanto
esplicita, che il censimento registra senza confonderla con questa.

Nessuna correzione proposta, e quindi nessuna modifica ai tipi pubblici. Ciò che
resta aperto è il limite del **filo**, che è C3 e dipende dalla decisione 0006.

### M3 — le due lacune sul fallimento tardivo

M1 aveva registrato che una sola prova fissava il momento del fallimento, e per
un solo modo di fallire. Le due lacune nominate erano il budget esaurito durante
la scrittura e l'errore dopo una scrittura parziale.

#### Che cosa esisteva, e che cosa no

`--max-output-bytes` è un flag della CLI e **nessuna prova lo esercitava**: la
ricerca fra le suite non ne trova un uso. I limiti che avevano prove —
`max_rows`, i tetti su WKT e componenti — agiscono tutti **in lettura**, prima
che un writer esista.

#### Il budget d'uscita si applica a scrittura finita, ed è misurabile

`publish_file_atomic_limited` legge la dimensione del file di staging —
`temp.as_file().metadata()?.len()` — e rifiuta prima del rename. La scrittura è
quindi completa quando il limite scatta, e la busta lo attesta senza bisogno di
guardare dentro il processo: il messaggio nomina i byte prodotti, e quel numero
coincide con `bytes_written` della stessa conversione senza limite. Un rifiuto
preventivo non potrebbe conoscerlo.

Su 2.000 righe: uscita piena 229.601 byte, tetto a 100.000, e la busta dice
«output da 229601 byte oltre il limite di 100000». Destinazione assente,
directory senza residui.

#### Il rilievo: `phase` non rispetta ERR-003 su questo percorso

ERR-003 dice che `phase` «identifies the last externally meaningful phase known
to have started». Il rifiuto del budget d'uscita dichiara `phase: validate`,
mentre la lettura è avvenuta, la scrittura è finita e si è alla pubblicazione.

Non è una semplificazione generale del prodotto: il rifiuto per destinazione
preesistente dichiara `phase: commit`, ed è corretto. La causa è puntuale —
`PlenoraIoError::limite_redatto` fissa `ErrorPhase::Validate` — e quel
costruttore serve anche limiti che scattano davvero in validazione, dove la fase
è giusta.

**Ingresso concreto**: una conversione CSV→GeoJSON di 2.000 righe con
`--max-output-bytes` sotto la dimensione prodotta.
**Conseguenza osservabile**: un orchestratore che legge `phase` per sapere
quanto lontano sia arrivata l'operazione legge `validate`, cioè «non ha
cominciato», mentre l'intero output è stato prodotto e scartato. Gli altri tre
assi restano corretti — `resource_limit`, `retry: never`, `remote_effect: none`
— e la destinazione non esiste davvero: l'errore non è nell'effetto, è in quanto
lavoro dichiara di aver fatto.

**Correzione applicata.** `PlenoraIoError::limite_alla_pubblicazione_redatto`
dichiara `ErrorPhase::Commit`, e i tre controlli di `publish.rs` la usano.
`limite_redatto` resta invariato per gli altri ottantasei chiamanti, che
rifiutano davvero in validazione: cambiarlo per tutti avrebbe spostato la fase
di ottantasei per correggerne tre. La regressione pubblica pretende ora
`phase: commit`.

È un valore osservabile che cambia, ed è il motivo per cui la correzione si fa:
porta la busta a dire ciò che ERR-003 le chiede di dire. Lo stesso rilievo vale
per il guasto dei dati, che dichiarava pure `phase: validate`; lì avevo scritto
che il momento non era osservabile e che quindi non c'era una fase giusta da
dedurre. **Era sbagliato**: ERR-003 riguarda la fase nota al prodotto, non
quella che il test riesce a vedere. L'assenza di osservabilità pubblica limita
la prova, non determina il valore. La tracciatura è più sotto.

#### Che cosa `--max-output-bytes` limita davvero

Limita **la pubblicazione**: il confronto avviene sulla dimensione dello staging
appena prima del rename, e ciò che il flag impedisce è che un output più grande
del tetto diventi la destinazione.

**Non** è un tetto ai byte scritti temporaneamente su disco. Lo staging viene
prodotto per intero prima che il limite scatti, e la prova qui sotto lo mostra
invece di nasconderlo: il messaggio d'errore nomina 229.601 byte sotto un tetto
di 100.000, cioè dichiara di aver scritto più del doppio del consentito. Chi
avesse bisogno di un tetto sullo spazio temporaneo non lo troverebbe qui, e
nessuna prova di questo blocco lo promette.

#### Le prove aggiunte

`crates/plenora-io-tools/tests/fallimento_tardivo.rs`, tre prove sul percorso
**pubblico**, cioè invocando il binario:

| prova | che cosa fissa |
|---|---|
| `il_budget_d_uscita_esaurito_non_lascia_destinazione_ne_residui` | i quattro assi con `phase: commit`, la destinazione assente, la directory senza staging, **e** che il messaggio nomini i byte prodotti |
| `un_wkt_malformato_e_rifiutato_dall_inferenza_prima_dello_stream` | gli stessi invarianti con un WKT rotto a riga 15.000, e `phase: prepare` |
| `un_guasto_nel_loop_di_lettura_dichiara_la_fase_di_lettura` | `phase: read` su un guasto che l'inferenza non può intercettare |
| `una_destinazione_preesistente_resta_invariata_dopo_un_fallimento` | che il file di prima sia byte per byte quello di prima, e la fase `commit` |

E una prova **interna**, in `plenora-io-core/src/driver/tests.rs`, perché il
momento del guasto dalla riga di comando non si osserva:

| prova | che cosa fissa |
|---|---|
| `un_guasto_del_backend_dopo_una_scrittura_riuscita_non_pubblica_niente` | il primo batch scrive davvero sullo staging e la sua dimensione lo **attesta**; il secondo fallisce con `phase: Write` e `retry: Never`; lo staging sparisce alla distruzione del writer; la destinazione preesistente resta byte per byte quella di prima |

Che quella prova misuri il momento non è affermato: è stato verificato
rompendola. Facendo fallire il writer **subito** invece che al secondo batch,
diventa rossa sull'asserzione «il primo batch deve riuscire». Una prova sul
fallimento tardivo che resta verde anche quando il fallimento è immediato non
misura il momento, e questa non lo fa.

Sono deterministiche: nessun segnale, nessuna attesa, nessuna corsa contro il
tempo. Il tetto della prima si calcola dalla corsa di riferimento invece di
essere un numero fisso, così non invecchia con il formato.

#### Provato sul percorso pubblico contro verificato internamente

Tutto quanto sopra è **pubblico**: buste, codici d'uscita, filesystem. Nulla
guarda dentro il processo.

Ciò che resta **non attestato dalla CLI** è che la scrittura fosse cominciata
nel caso del guasto dei dati: la superficie pubblica non espone quanto sia stato
fatto, e la prova lo dice invece di dedurlo dal numero di riga. Che il file di
staging esista e venga rimosso è verificato lì per **assenza di residui**, non
per osservazione diretta.

La prova interna copre esattamente quella distanza, e su un percorso diverso:
osserva lo staging mentre esiste, ne misura la dimensione dopo il primo batch, e
lo ritrova assente dopo la distruzione del writer. È evidenza più forte, e per
questo sta dentro il crate: chiederla alla riga di comando avrebbe voluto dire
inventare un'osservazione che quella superficie non offre.

**Le due lacune non sono chiuse allo stesso modo.** Il limite d'uscita rilevato
prima della pubblicazione è coperto, e la correzione della fase lo accompagna.
Il guasto del backend dopo una scrittura parziale è ora coperto **internamente**;
sul percorso pubblico resta senza una prova che ne fissi il momento, e il guasto
sui dati non è un equivalente.

Il rilascio dei lease non è osservabile dalla riga di comando — il processo
termina, e con lui tutto. Le regressioni che lo coprono restano quelle interne
già censite in M1: sedici sui lease, ottantaquattro sui budget.

#### Il residuo sulla fase: tracciato, e corretto in due punti

Avevo giustificato `phase: validate` dicendo che la CLI non espone l'avanzamento.
Non regge: ERR-003 parla della fase **nota al prodotto**. Quindi ho tracciato
dove l'errore nasce davvero, invece di dedurlo.

**Primo fatto, dal codice.** L'errore nasce in `wkt_progressivo`, un analizzatore
puro, e il suo costruttore `wkb_redatto` fissa `ErrorPhase::Validate`. Ma il
parser serve tre passate diverse — l'inferenza, il loop di lettura e i writer che
ricodificano in uscita — e non sa in quale sta girando. Quella fase è perciò un
**default**, non una determinazione: cambiare il costruttore avrebbe spostato la
fase anche agli encoder del lato scrittura, che direbbero `read`.

**Secondo fatto, dal vicinato.** Nello stesso loop, due righe più in là,
`required_cell` dichiara `Read` via `formato_redatto`. Stessa iterazione, stessa
conoscenza dello stato, due fasi diverse: non possono essere entrambe giuste. Non
è una deduzione dal test — è il prodotto che si contraddice da solo.

**Terzo fatto, misurato.** Marcando temporaneamente il sito dell'inferenza con
una fase riconoscibile, la busta della prova ha riportato quel marcatore. Quindi
il guasto di quella prova **non** avviene a metà stream: `infer_wkt_geometry`
gira dentro `open` e visita la colonna geometrica fino a `max_rows`, incontrando
la riga rotta mentre il reader viene allestito. Lo stream non è cominciato e il
writer non esiste.

Ne segue che il nome della prova, `..._a_meta_stream_...`, affermava un momento
che non avviene, ed è stato corretto insieme alla fase: è la stessa lezione del
blocco precedente, cioè che il nome di una prova non stabilisce quando.

**Le due correzioni.** Ciascuna al livello che la fase la sa, con `during()`, che
è l'idioma già in uso in `scrittura_limitata` — costruttore con default, corretto
da chi conosce lo stadio.

| sito | prima | dopo | perché |
|---|---|---|---|
| `infer_wkt_geometry`, dentro `open` | `validate` | **`prepare`** | è lo stadio di `reader_busy` e `projection_unsupported`, che nascono nello stesso `open`; `probe` etichetta la scoperta della sorgente, che viene prima |
| il loop di `spawn_parser` | `validate` | **`read`** | `read_record` è già riuscito, e la riga accanto lo dichiara |

**Che il secondo sito sia raggiungibile è stato provato, non assunto.** Se
l'inferenza vede sempre le stesse celle prima del loop, la correzione nel loop
sarebbe codice morto. Esiste però un'asimmetria: l'inferenza **analizza**
soltanto, il loop analizza e poi **codifica** in WKB. Una `LINESTRING` con
coordinate corte occupa in WKB circa quattro volte il suo testo, quindi con
`--max-wkb-cell-bytes 1000` un WKT da 411 byte passa l'inferenza e i suoi 1613
byte di WKB falliscono nel loop. La prova nasce da lì.

Entrambi i valori sono verificati sul percorso pubblico, e il valore di prima è
stato verificato **rimuovendo la correzione e rieseguendo**: la busta tornava a
dire `validate`.

#### Lo stesso difetto sugli altri percorsi: censito, e corretto

Avevo scritto che riguardava «DXF, GeoJSON, KML e Shapefile». Il censimento per
propagazione lo smentisce in parte, e la differenza conta perché due di quei
quattro non avevano niente da correggere.

| driver | punto | stadio | esito |
|---|---|---|---|
| CSV | `write`, due chiamate | scrittura | `validate` → **`write`** |
| DXF | `write` | scrittura | `validate` → **`write`** |
| DXF | `next_row`, spool su file | lettura | `validate` → **`read`** |
| GeoJSON | `write_feature` ← `write` | scrittura | `validate` → **`write`** |
| XLS | `finish`, due chiamate | finalizzazione | `validate` → **`finalize`** |
| XLS | `encode_geometry_cell` ← `open` | allestimento | `validate` → **`prepare`** |
| KML | `write` | — | **nessun difetto *di fase***: l'errore non è propagato |
| Shapefile | `write` | — | **nessun difetto *di fase***: l'errore non è propagato |
| Shapefile, GPKG | entry point di fuzz | — | non è percorso di pipeline |

KML e Shapefile usano `let Ok(…) else`: spingono un rifiuto di riga e
continuano. La fase di un errore che nessuno legge non è un difetto — e
l'affermazione si ferma qui. **Che scartare sia la scelta giusta non è provato
da questo**: è una domanda sul comportamento, non sulla busta, e per rispondere
servirebbe stabilire se una geometria illeggibile debba diventare una riga
rifiutata o fermare la conversione. Nessuna prova di questo blocco la tocca, e
non apro la revisione qui. CSV e XLS, che non avevo nominato, propagano invece
davvero.

XLS prende `Finalize` e non `Write` perché il suo writer **accumula** i batch e
materializza il foglio in `finish`: è lì che decodifica, ed è lì che siamo.

#### Il ramo è difensivo, e questo cambia dove stanno le prove

Le sei correzioni sono giuste, ma cinque su sei non si raggiungono dalla riga di
comando, e vale la pena dire perché invece di scrivere prove che sembrano
esercitarle.

Davanti a ogni decodifica di scrittura ci sono **due strati**. Il primo è in
lettura: corrompendo uno per uno i cinque WKB di un file Arrow IPC — non
compresso, quindi correggibile byte per byte a lunghezza costante — l'errore
arriva cinque volte su cinque con `phase: read`, che è già la fase giusta. Il
secondo è `with_write_validation`, che `create` avvolge attorno al writer di
ogni driver: ispeziona la geometria e registra una **violazione di riga**
(`conversion.invalid_geometry`) invece di propagare.

`inspect_wkb` e `decode_wkb` prendono gli stessi limiti e percorrono la stessa
struttura — la doc del secondo dice «gli stessi errori del primo» — quindi ciò
che supera l'ispezione supera anche la decodifica. Non è che il ramo sia
improbabile: è che il prodotto è costruito perché non ci si arrivi.

Difensivo non vuol dire esente. Se ci si arriva, la fase dev'essere quella in
corso, e le regressioni stanno al livello da cui il ramo si raggiunge davvero:
dentro i crate, invocando `write_feature` o costruendo lo stato del writer come
fa `create` ma senza la guardia. È la stessa distinzione di M3 fra ciò che è
provato sul percorso pubblico e ciò che è verificato internamente.

**La sola pubblica è l'inferenza XLS**, che si raggiunge stringendo
`--max-wkb-cell-bytes` sotto la lunghezza del testo WKT: `phase: prepare`, con
la corsa di controllo senza tetto che riesce.

#### Una riga corretta non era quella raggiungibile

In `next_row` avevo corretto il `decode_wkb`. Due righe sopra c'è
`inspect_wkb(&bytes, limits)?`, con gli **stessi limiti**, e la decodifica gira
solo se l'ispezione è passata: un tetto superato ferma l'ispezione, e la riga
che avevo corretto non viene mai raggiunta. La correzione utile era l'altra, e
la prova la esercita.

È la stessa lezione di `FLAG_FORMATO` in una forma nuova: lì compilare non
bastava, qui non basta nemmeno che la riga corretta sia quella giusta per
descrizione. La prova deve arrivarci.

#### Le sei prove, e la verifica in entrambi i versi

| prova | dove | fase fissata |
|---|---|---|
| `l_inferenza_xlsx_dichiara_l_allestimento_del_reader` | `tests/fase_degli_errori.rs`, **pubblica** | `prepare` |
| `la_decodifica_difensiva_del_writer_dichiara_la_fase_di_scrittura` | driver-csv, interna | `write` |
| idem | driver-dxf, interna | `write` |
| idem | driver-geojson, interna, via `write_feature` | `write` |
| `la_decodifica_difensiva_del_writer_dichiara_la_finalizzazione` | driver-xls, interna | `finalize` |
| `lo_spool_su_file_dichiara_la_fase_di_lettura_sul_tetto_per_cella` | driver-dxf, interna | `read` |

Ciascuna controlla la fase **e** che gli altri quattro assi non si muovano:
codice, categoria, riprovabilità, effetto remoto. La correzione riguarda un
campo solo, e le prove lo dicono invece di fidarsi.

Che misurino la correzione è verificato nei due versi: mettendo da parte i
quattro file corretti e rieseguendo, tutte e sei diventano rosse; ripristinando,
tutte e sei tornano verdi.

#### Il controllo di clippy che non copriva tutto

Registrato come fatto, perché spiega perché un rilievo vecchio è emerso ora: il
controllo che eseguivo non copriva tutte le feature, e `--all-features` bocciava
quattro `format_push_string` e un `missing_const_for_fn` introdotti nel blocco
precedente. Corretti; il controllo completo — `--workspace --all-targets
--all-features -D warnings` — ora passa, ed è quello del checkpoint.

#### M3 rispetto al proprio perimetro

La voce nominava cinque cose: lettura e scrittura IPC, i percorsi di publish,
budget esaurito, cancellazione durante attesa, errore tardivo. Due erano lacune,
e sono quelle che questo blocco ha chiuso; le altre tre avevano già prove, e il
confronto serve a dirlo con i nomi invece che con un conteggio.

| percorso del perimetro | stato | dove |
|---|---|---|
| budget esaurito | **chiuso in questo blocco** | `fallimento_tardivo.rs`, pubblico |
| errore tardivo | **chiuso in questo blocco** | pubblico per gli invarianti, interno per il momento |
| cancellazione durante attesa | già coperto | `cancellable_send_exits_when_a_full_channel_is_cancelled` e due sorelle |
| lettura e scrittura IPC | già coperto | ventidue prove fra flusso, serializzazione e prevalidazione |
| percorsi di publish | già coperto | cinquantaquattro prove fra pubblicazione, atomicità, staging e destinazione |

La cancellazione durante attesa merita una riga perché il nome non la
annunciava: il canale è pieno, il thread è bloccato in invio, e la cancellazione
lo sblocca. Le due sorelle coprono il token già cancellato prima dell'attesa e
il ricevente che sparisce mentre si attende — tre modi di uscire da un blocco,
non uno.

**Residui: nessuno.** Il solo che restava era la fase del guasto dei dati, ed è
chiuso nella sezione qui sotto.

Il resto del perimetro non ha residui che queste prove lascino scoperti. Non
significa che le guardie siano complete: significa che le cinque che la voce
nominava hanno ciascuna una prova, e che due di quelle prove non c'erano prima.

### M4 — censimento della corrispondenza

Censimento sui registri esistenti, senza secondo sistema di assurance e senza
nuovi gate. La domanda è se ogni proprietà dichiarata abbia una prova, e se
quella prova venga davvero eseguita.

#### Il legame dichiarato → prova esiste, ed è chiuso nei due versi

`contracts/requisiti-pubblici.json` porta **32 requisiti**, ciascuno con `id`,
la regola del contratto che lo impone, e uno stato fra `implementato` e
`non_ancora`. `scripts/check_public_contracts.py` tiene `SONDE`, una mappa da
`id` alla funzione che lo esercita, e la corrispondenza è verificata in
entrambe le direzioni:

- un requisito senza sonda è un errore del gate;
- una sonda senza requisito è «orfana», e pure quella è un errore;
- una prova del gate confronta i due insiemi **contro il registro reale**, non
  contro una copia.

È la proprietà che M4 chiede, e per la superficie pubblica c'è già. Non va
aggiunta: va riconosciuta, e il censimento serve a dire dove.

#### I quattro stati, e chi li distingue

| stato | chi lo dice |
|---|---|
| eseguito e passato | `implementato` + sonda verde → conta fra i **protetti** |
| eseguito e fallito | `implementato` + sonda rossa → **regressione**, e il gate è rosso |
| non applicabile | `non_ancora` + sonda rossa → **mancante**, atteso e dichiarato |
| eseguito oltre il dichiarato | `non_ancora` + sonda verde → **avanzamento**, da registrare |

C'è un quinto stato che il gate tratta a parte, e merita di essere nominato: un
artefatto che va in crash, si blocca o non parte non sta dicendo «questo
requisito non è ancora implementato» — non sta dicendo niente. Classificarlo con
lo stato del registro trasformerebbe un binario rotto in un piano di lavoro.

Il «saltato» ha un registro suo, `sonde-saltate-nella-sdist.json`, che elenca le
prove che nell'archivio sorgente non girano, con la condizione per cui non
girano e dove girano invece.

#### Dove le prove vengono eseguite davvero — e il residuo

| gate | CI | checkpoint L2 |
|---|---|---|
| `check_public_contracts.py` (i 32 requisiti) | sì, job `profilo-pubblico`, su ogni push | **no** |
| `check_public_identity.py` | sì | sì, due passi |
| gli altri gate del checkpoint | sì | sì, 105 passi |

Il verificatore del profilo pubblico **non è un passo del checkpoint**: zero
riferimenti in `s9-checkpoint.sh`, e nessuno dei 105 passi lo nomina. La ragione
è visibile — è un gate black-box che pretende un binario costruito, e il job di
CI infatti lo costruisce prima di invocarlo — ma la conseguenza va detta: **una
corsa di livello 2 può passare senza che i 32 requisiti pubblici siano stati
verificati.**

Per la 4.0.0 quella verifica è stata fatta, su un binario **estratto
dall'archivio** e non su una ricostruzione, con esito 32 su 32, ed è registrata
nell'evidenza. Ma è stata fatta perché qualcuno l'ha eseguita, non perché un
gate la pretendesse.

#### Il punto dove la corrispondenza si affida al verbale

Il manifesto di adozione porta, per ogni artefatto, un elenco `verification` coi
comandi che lo hanno verificato — per l'archivio Linux base il primo è proprio
`check_public_contracts.py`. Quelle stringhe sono **scritte**, non eseguite dal
gate che le valida, e `check_manifesto_adozione.py` lo dichiara apertamente:
verifica che il documento sia onesto nella forma, non che il prodotto sia
conforme, perché «un gate che pretendesse di rispondere a entrambe risponderebbe
male alla seconda».

La distinzione è corretta e va conservata. Ciò che resta scoperto è il terzo
passo: **nessuno verifica che quei comandi siano stati eseguiti su quei byte**.
Un manifesto che elencasse una verifica mai fatta sarebbe formalmente valido.

È esattamente il perimetro di R3–R5 — il legame fra candidate, revisione
qualificata ed evidenza, e il riuso che non si eredita — e questa voce vi
appartiene invece di generare un meccanismo proprio.

#### Catalogo, CLI e SDK

Il **catalogo** e la **CLI** sono coperti dai 32 requisiti, che nascono da
`CAPABILITY-DISCOVERY`, `CLI-2.0`, `ERRORS-1.0` e dalle altre regole nominate in
`regola`. L'**SDK Python** ha le proprie prove, tre job `python-sdk` in CI su
3.11, 3.12 e 3.13, e il gate che confronta i suoi modelli col protocollo — ma
non ha un registro `id → prova` come quello del profilo pubblico: la
corrispondenza lì è per suite, non per requisito.

Non propongo di costruirgliene uno. Sarebbe un secondo sistema di assurance per
una superficie che il profilo dichiara **non richiesta**, e il costo cadrebbe su
ogni requisito futuro. Che la granularità sia diversa è un fatto da registrare,
non una lacuna da chiudere per simmetria.

#### Esito

La corrispondenza dichiarato → prova è **già chiusa e verificata** per la
superficie pubblica, ed è il risultato principale di questo censimento: la voce
si chiude indicando ciò che esiste, non aggiungendo.

**Registrato: M4 è un censimento completato**, con due residui assegnati a
R3–R5. Sono entrambi sull'**esecuzione** e non sull'esistenza, e sono riformulati
lì come requisiti di chiusura invece di restare osservazioni:

1. rendere **obbligatoria** la verifica dei 32 requisiti pubblici nella
   qualificazione finale dell'artefatto distribuito;
2. **collegare** l'esito di quella verifica al digest dell'artefatto
   effettivamente provato, e verificare la corrispondenza prima dell'adozione.
   Un elenco di comandi nel manifesto non basta.

Si riusano registro, sonde ed evidenze che già esistono. Non si aggiunge un
secondo sistema per l'SDK: la sua granularità diversa resta un fatto registrato.

Il residuo concreto, detto in una riga sola, è **collegare le verifiche eseguite
ai byte dell'artefatto**.

La corsa 32/32 fatta a mano vale per il binario che ha verificato, e non chiude
il collegamento: resta un'esecuzione decisa da chi la esegue, non pretesa da un
gate, e il suo esito non è ancorato al digest di quell'artefatto. Registrarla
come prova della capacità sarebbe scambiare una verifica riuscita per il legame
che ancora manca.

### R3–R4 — i due residui di M4, implementati

M4 aveva lasciato due cose, entrambe sull'esecuzione: la verifica dei 32
requisiti pubblici non era pretesa da nessun gate sull'artefatto distribuito, e
i comandi di `verification` del manifesto erano stringhe scritte, non
collegate ai byte. Il residuo, in una riga, era **collegare le verifiche
eseguite ai byte dell'artefatto**.

#### Il legame si costruisce, non si dichiara

`check_public_contracts.py` prende un ingresso nuovo, `--artefatto`, e la
differenza con `--cli` non è di comodità: cambia che cosa viene provato. Il
binario non lo indica chi lancia il comando, lo si **estrae dall'archivio**, e
il digest che finisce nell'attestazione è quello dei byte su cui le sonde hanno
girato. «Questi requisiti sono verificati su questi byte» diventa così una
conseguenza di come la verifica è stata eseguita.

Le due opzioni si escludono, per la stessa ragione per cui esistono entrambe:
`--cli` interroga un binario senza dire da dove venga, ed è ciò che serve in CI;
ammetterle insieme vorrebbe dire poter attestare un archivio interrogandone un
altro.

Tre scelte che vale la pena dire, perché ciascuna chiude una via al verde
facile:

- l'attestazione è **sempre esigente**. Una corsa non esigente direbbe
  «conforme» con requisiti non soddisfatti, e la conformità parziale non
  qualifica.
- si scrive **anche quando la verifica fallisce**. Scriverla solo in caso di
  successo lascerebbe una corsa rossa senza traccia, e la via più breve al verde
  sarebbe rieseguire finché non passa.
- un archivio con **due** binari della CLI è un errore, non una scelta: due
  candidati vorrebbero dire che non si sa quale sia stato interrogato, ed è
  proprio l'ambiguità che l'attestazione esiste per togliere.

#### La sesta condizione

L'insieme delle condizioni di autorizzazione passa da cinque a sei.
`profilo-pubblico-attestato` pretende, per **ogni** artefatto `cli` del
manifesto, un'attestazione il cui digest coincida con quello distribuito, con
conformità dichiarata, esigenza dichiarata, pin dei contratti uguale a quello
del manifesto e nessuna regressione, nessun avanzamento non dichiarato, nessuna
invocazione guasta.

È rossa anche in tre casi che non sono «manca un'attestazione», e ciascuno era
una via al verde:

| caso | perché è rosso |
|---|---|
| il manifesto non c'è | senza, non si sa quali byte si stiano distribuendo |
| nessun artefatto `cli` | un insieme vuoto soddisfa ogni «per ogni» |
| un'attestazione che non corrisponde a niente | è il residuo di una distribuzione precedente |

E un file illeggibile non viene trattato come assente: il messaggio direbbe
«manca» di un file che c'è, e chi cerca il guasto lo cercherebbe altrove.

**Non c'è una via differita, ed è deliberato.** Un binario Windows non si esegue
in una corsa Linux: la sua attestazione si produce dove quel binario gira e si
versiona insieme alle altre. Ammettere un rinvio qui sarebbe riammettere
esattamente ciò che mancava. È un costo operativo, e va detto che è nuovo.

#### Che cosa lo prova

Venticinque regressioni nuove: sedici sulla condizione, nove sull'ingresso del
verificatore, più le esistenti che contano le condizioni e che ora ne pretendono
sei. Ciascuna guasta **una** proprietà e lascia le altre sane, così il rosso
nomina la causa invece di essere la somma di più cose rotte insieme.

Oltre a queste, la catena è stata percorsa intera sui byte veri: archivio
costruito col binario dentro, attestazione prodotta, condizione verde; poi il
digest del manifesto cambiato lasciando l'attestazione dov'era, e la condizione
rossa con il messaggio che nomina entrambi i digest.

#### Che cosa resta aperto

La corsa 32/32 fatta a mano per la 4.0.0 resta ciò che era: una verifica
riuscita su un binario, non il legame. Vale per quell'artefatto e non lo
attesta nel senso che la condizione ora pretende.

E `check_public_contracts.py` continua a non essere un passo del **checkpoint**.
La voce chiedeva che fosse obbligatorio nella qualificazione finale
dell'artefatto distribuito, ed è lì che ora lo è; il checkpoint gira su un
albero, non su un archivio, e portarcelo sarebbe una voce diversa da queste due.

### Residui aperti — elenco unico

Uno solo, e sostituisce quello del blocco precedente. Ogni voce dice **come si
chiude** e **da che cosa dipende**; le voci senza dipendenze si possono prendere
in qualunque ordine.

#### Stato dei gate

`check_release_contract.py` esce **0** su un checkout pulito: i tre invarianti
che erano rossi sono chiusi. Con `--release` esce **1**, ed è giusto che lo
faccia: le quattro condizioni non soddisfatte sono lo stato reale di una
candidate che non esiste ancora.

| condizione non soddisfatta | perché |
|---|---|
| `candidate-coerente` | nessuna candidate è preparata |
| `profilo-pubblico-attestato` | nessun artefatto è stato costruito né attestato |
| `profondita-fuzz-rimisurata` | quattro misure dichiarate scadute |
| `confine-asan-rimisurato` | il confine ASan dichiarato scaduto |
| `campagna-fuzz-completa` | nessun verbale di campagna: lo smoke non è stato eseguito su questo albero |

#### Il rischio da nominare adesso, non alla vigilia

**Il finding GeoParquet può impedire la campagna completa, e quindi la
qualificazione del candidato.**

Il classificatore gestisce l'*esito* di quel crash; non toglie il difetto che lo
produce. Se il fuzzer lo ritrova sul candidato finale — e non c'è ragione di
credere che non lo ritrovi, visto che il difetto è a monte e ancora aperto anche
nella 60.0.0 — `geoparquet_reader` si ferma, il verbale lo registra fra i
fermati, e `campagna-fuzz-completa` è rossa.

**Non c'è deroga implicita per i crash compatibili con il noto**, ed è
deliberato. Un rinvio che coprisse anche questo caso trasformerebbe la
registrazione in un permesso: la campagna resterebbe interrotta e nessuno
dovrebbe più farci niente.

Le vie che chiudono il rischio sono tre, e nessuna è «ignorarlo»:

| via | che cosa richiede |
|---|---|
| la correzione upstream è pubblicata e il pin la include | dipende da Q2b, cioè dall'invio, e dai tempi di `arrow-rs` |
| il difetto è aggirato nel nostro percorso, prima che il decoder lo incontri | è una modifica al prodotto, e va decisa: non è una prevalidazione scelta per comodità, è un cambiamento di ciò che il driver accetta |
| la deroga è **esplicita**, dichiarata e datata | costa una decisione scritta, come le due deviazioni della 4.0.0, e dice che cosa la release smette di promettere |

Le prime due possono richiedere tempo; la terza è una scelta, non un
meccanismo. Il punto di scriverlo qui è che nessuna delle tre si improvvisa la
sera prima del rilascio.

Questo è anche il motivo per cui la voce Q2b non è chiusa: il suo criterio non è
«il caso minimo esiste», è il riferimento alla issue. Finché non c'è, la prima
via non è nemmeno cominciata.

Le ultime tre si chiudono **solo** sul candidato finale. Non sono debito: sono
il rinvio che funziona, e il fatto che `--release` le nomini è la prova che il
rinvio non si trasforma in dimenticanza.

#### Lavoro aperto

| # | voce | criterio di chiusura | dipende da |
|---|---|---|---|
| 1 | ~~**Q2b** — segnalazione upstream ad `arrow-rs`~~ | **chiusa**: inviata il 2026-09-17, [`apache/arrow-rs#11121`](https://github.com/apache/arrow-rs/issues/11121), con `Cargo.toml` e `src/main.rs` del caso minimo nel corpo. Chiudeva con il riferimento alla issue, e il riferimento c'è. **Inviare non è correggere**: il difetto a monte resta aperto, e la prima via per `campagna-fuzz-completa` dipende ora dai tempi di `arrow-rs` | — |
| 2 | ~~**Q2c** — finding noti del fuzz~~ | **chiusa**: `classifica_finding_fuzz.py` riconosce un crash **compatibile** con una firma nota, conserva ogni input col suo referto, e il verbale della corsa impedisce che un'interruzione nota passi per campagna completa | — |
| 3 | ~~**Q1** — contraddizione del workflow «Release checkout qualification»~~ | **chiusa**: il gate distingue l'albero congelato dal commit di registrazione; sei prove, e la prima riproduce il guasto | — |
| 4 | ~~**R1** — regressione su `MAX_BLOCCHI`~~ | **chiusa**: tre casi in `prevalida_arrow::tetto_dei_messaggi`; costo misurato — 128 MiB generati in 60 ms e scanditi in 3,5 s — e fixture generata, non versionata | — |
| 5 | ~~**R2** — campagne senza `sleep` a scadenza~~ | **chiusa**: le tre proprietà c'erano, mancava la quarta — `collect` rimuoveva il container portandosi via il log. Sette prove con un Docker finto. Limite residuo: due corse nello stesso secondo condividerebbero il nome del file, ed è detto nella prova invece che nascosto | — |
| 6 | ~~**R3–R4**~~ | **chiuse**: il legame misura-evidenza era già imposto e ora è verificato per intero; la regola del riuso è scritta per i sei tipi, col gate che pretende l'esistenza dei sorveglianti | — |
| 7 | ~~**R5** — corse concorrenti e push dopo controlli falliti~~ | **chiusa**: `start` rifiuta di sovrascrivere una corsa e di partire su un albero sporco, l'esito viene da `inspect`, e i sorgenti sono un **clone isolato** della revisione incisa | — |
| 8 | ~~**D1**~~ | **rinviata al ciclo successivo**: il beneficio è una versione duplicata in meno, il costo è un delta nuovo da mantenere nel fork — `enum_primitive` serve numerose enum e quattro generatori | — |
| 9 | ~~**M5** e le fasi D, F, U, C~~ | **fuori dalla 4.1.0**, per scelta di perimetro dichiarata. D5, D6 e F1 non sono rinviate per comodità: il lock dice che non si chiudono da qui | — |

#### Da R3–R4: anche il verbale fuzz è riferito a ciò che ha eseguito

La stessa domanda che R3–R4 pongono alle evidenze vale per il verbale della
campagna, e all'inizio non ci valeva: registrava chi avesse finito, non su quale
albero. Un verbale **completo** di una revisione precedente avrebbe qualificato
quella corrente.

Due campi lo legano, e ciascuno chiude una via diversa:

- la **revisione**. Una campagna vale per il codice su cui è girata, ed è la
  stessa ragione per cui le misure di profondità portano l'impronta del
  perimetro. Qui la revisione basta: la qualifica pretende già un albero pulito
  allo SHA atteso, quindi due SHA uguali sono due alberi uguali. È il riuso del
  collegamento che esiste, non un meccanismo nuovo.
- i **bersagli dichiarati**, cioè `cargo fuzz list`. Lo smoke sa girare su un
  sottoinsieme, e un target in quarantena esce comunque dai finiti: in entrambi
  i casi «nessuno si è fermato» sarebbe vero e direbbe pochissimo. La qualifica
  pretende che i finiti siano **tutti** quelli dichiarati.

Cinque regressioni, fra cui il verbale di un'altra revisione e la corsa su un
sottoinsieme.

#### La decisione che precede il congelamento

Le priorità che il piano dichiara — M1–M3, poi M4 con R3–R5 — sono chiuse, e
con esse R1, R2 e R6. Resta aperta Q2b, che dipende da un invio, e resta la
voce 8: M5 con le fasi D, F, U, C.

**Quella voce non si può rimandare a dopo il congelamento**, ed è il motivo per
cui la decisione viene prima. D1–D7, F1–F7 e le altre sono aggiornamenti di
dipendenze: ciascuno tocca `Cargo.lock`, e la regola appena scritta in
`riuso-delle-evidenze.json` dice che cosa comporta — censimento della closure,
profondità del fuzz, confine ASan, copertura. Farne uno dopo il congelamento
invaliderebbe le misure che il congelamento serve a fissare.

**La via scelta è la seconda**: prima si definisce e si completa il
sottoinsieme di dipendenze destinato alla 4.1.0, poi si congela e si rimisura.
Alternare è la via peggiore — congelare, aggiornare una dipendenza e rimisurare
costa due campagne per un risultato solo.

#### Il sottoinsieme proposto, e il criterio che lo seleziona

Il criterio è uno: **entra ciò che si chiude dentro questo repository, con una
regressione che dica se è andata bene.** Resta fuori ciò che dipende da una
risposta di terzi, perché un ciclo non si tiene aperto aspettando qualcuno.

**La prima selezione era sbagliata in tre punti su quattro**, e vale dirlo
perché l'errore è istruttivo: avevo applicato il criterio leggendo i titoli
delle voci invece del lock. «Un chiamante solo» e «dentro il nostro
`Cargo.toml`» erano affermazioni sul piano, non sul grafo. Questa tabella nasce
da `cargo tree -i`, e dice chi trattiene davvero ciascuna versione.

| voce | dentro | che cosa dice il lock |
|---|---|---|
| D1 `num-traits` 0.1 | **sì** | `num-traits 0.1.43` ← `enum_primitive 0.1.1` ← **`dxf 0.6.1`, il nostro fork**. È l'unica catena, e passa da codice che governiamo |
| D5 `thiserror` 1 → 2 | **no** | `thiserror 1.0.69` ← **`wkt 0.14.0`**, non da noi. Non dichiariamo `thiserror`: aggiornarlo richiede che `wkt` si muova, o un fork nostro |
| D6 `syn` 2 → 3 | **no** | `syn 2` ← `strum_macros`, `thiserror-impl` e `zerocopy-derive`, tutte esterne. Nessuna la governa questo repository |
| F1 ZIP → Zopfli | **no** | la feature `deflate` di `zip` accende `deflate-zopfli`, e `deflate` la richiede **`calamine`**, non solo il nostro `Cargo.toml`. Spegnerla da qui non basta, ed è ciò che la voce stessa avvertiva |
| D2, D3, D4, D7 | no | trattenute da `kml`/`calamine`, da Arrow, da mezza closure, da `atomicwrites` |
| U1–U3 | no | riconciliazione dei fork e monitoraggio: U3 dice esplicitamente che una release di terzi non deve rendere rossa una revisione qualificata |
| C1–C4 | no | ciascuna chiede una decisione concordata sul contratto comune, cioè un'altra parte |
| **M5** | **rinviata al ciclo successivo** | valuta le dipendenze come manutenibilità *sugli esiti* di L, D e F. Con una voce su quindici eseguita, la valutazione direbbe del campione e non del prodotto |

**Resta dentro D1, e nient'altro.** Tre delle quattro che avevo scelto chiedono
che si muova un progetto di terzi o che si apra un fork nuovo, e nessuna delle
due cose sta dentro il criterio.

#### Il perimetro della 4.1.0 è chiuso: anche D1 è fuori

**D1 è rinviata esplicitamente al ciclo successivo**, e la ragione è un conto
che avevo fatto male.

Il beneficio è dimostrato ma stretto: togliere dalla closure una versione
duplicata, `num-traits 0.1`. Il costo l'avevo descritto come «una sola
rimisurazione», e non è quello. **Le misure finali sono dovute comunque**: il
congelamento le pretende con o senza D1, quindi non sono un costo di D1. Ciò
che D1 costa davvero è un'altra cosa — implementazione, regressioni, e **un
delta nuovo da mantenere nel fork**: `enum_primitive` serve numerose enum e
quattro generatori, quindi bisogna preservare conversioni, gestione dei valori
non validi e superficie, e poi aggiornare la provenienza del fork.

Non emerge un beneficio funzionale o di manutenzione che giustifichi di
prolungare il ciclo per questo. Una versione duplicata in meno è un risultato;
un delta in più da riconciliare a ogni release upstream è un impegno, e i due
non si compensano.

**M5 resta fuori dalla 4.1.0**, per la stessa scelta di perimetro: valuta le
dipendenze come manutenibilità sugli esiti di L, D e F, e con zero voci D ed F
eseguite direbbe del campione e non del prodotto.

**Il perimetro è chiuso.** La 4.1.0 consegna M1–M4, R1–R6, Q1 e Q2c; non
promette la valutazione di manutenibilità delle dipendenze né alcun
aggiornamento delle fasi D, F, U, C. Q2b resta visibile come azione pendente:
il contributo è pronto, l'invio no, e il criterio è il riferimento alla issue.

#### Sul candidato finale, per accordo

Cinque rimisurazioni — profondità per i quattro bersagli e confine ASan — più
L2, i soak prolungati e la qualifica completa degli artefatti. Il criterio di
chiusura è che `check_release_contract.py --release` non nomini più nessuna
delle tre condizioni di cui sopra.

Dipendono da una cosa sola: che l'albero non cambi più nel perimetro di quei
bersagli dopo la misura. È il motivo per cui le rimisurazioni vanno **ultime**,
e per cui il rinvio porta l'impronta dell'albero corrente: se il codice cambia
ancora, la dichiarazione diventa stantia e il gate torna rosso da solo.

**Priorità: chiuse.** M1–M4 e R1–R6 sono completate; restano Q2b, che dipende
da un invio, e la decisione sulle fasi delle dipendenze. Le righe che seguono
restano com'erano scritte, perché dicono perché quell'ordine fu scelto.

**Come era formulata: M1–M3, dopo L1–L2 ora chiuse; M4 con R3–R5.** Una voce
già coperta si chiude indicando la prova esistente, senza aggiungere un nuovo
gate per simmetria con database-tools. Le lacune osservate nel riferimento
sono un motivo per verificare la stessa classe in IO, non un'autorizzazione a
modificare l'altro repository. M5 raccoglie gli interventi sulle dipendenze già
previsti e non ne duplica l'implementazione.

**La base comune esiste già.** Entrambi adottano CLI v2, capability discovery
v2, errori v1, diagnostica di riga e Arrow interchange v1. L'allineamento deve
provare il passaggio dei dati sulle superfici effettive. Non richiede di
portare in IO-tools engine SQL, ORM, transazioni di database, pooling o driver
database: queste responsabilità restano nel componente che le possiede.

### Interoperabilità: differenze osservate e conseguenze

| Asse | Evidenza nei sorgenti | Conseguenza per la 4.1.0 |
|---|---|---|
| Autorità dei contratti | Il [pin database][db-adoption] è **anteriore** al pin IO. Fra i due il catalogo database passa da `provisional` a `normative`, e il profilo IO ratifica il cutover 4.0.0; il vocabolario Arrow non cambia | Non retrocedere il pin IO per far coincidere gli SHA. Riconciliare l'adozione sul lato database e verificare i requisiti applicabili a ciascun componente |
| IPC sulla CLI | IO legge e scrive `file` e `stream` (`crates/driver-ipc/src/lib.rs`). Il [catalogo database][db-catalog] dichiara entrambi, ma `canonical_write` arriva a [`IpcFileBatchStream::open`][db-ipc], che usa solo `FileReader`; `canonical_read` scrive con `FileWriter` in [`main.rs`][db-cli] | Lo scambio via IPC **file** è il primo percorso candidato da provare. Lo stream sulla CLI database è un disallineamento statico fra dichiarazione e percorso: va riprodotto e trattato nel repository responsabile, non compensato togliendo supporto a IO |
| Rust nello stesso processo | IO fissa Arrow `59.3.0`; il [manifest database][db-cargo] fissa `59.2.0` | Il passaggio diretto di `RecordBatch` richiede risoluzione congiunta e prova di compilazione. La differenza dei pin, da sola, non dimostra incompatibilità dei byte IPC; nessun downgrade automatico di IO |
| Geometrie e metadati | Namespace `plenora.geometry.*` e versione schema `1` comuni. IO conserva localmente `scansione_completa`; entrambi hanno sul filo solo `exact`, `mixed`, `unresolved`. Il [lettore database][db-fields] analizza SRID e field id come `u32`; IO usa SRID `i32` e conserva identità dichiarate anche oltre la posizione fisica della colonna | Servono vettori comuni di accettazione/rifiuto, inclusi domini numerici e perdita della distinzione «scansione completa, nessuna geometria». Il riferimento resta il [vocabolario condiviso][arrow-vocabulary], che dichiara SRID signed 32-bit |
| SDK Python | IO espone `Client.read(source, output)` e `write(...)` attraverso il processo CLI. Il [binding database][db-python] è PyO3, con `Table`, `RecordBatch`, iterable e IPC per il bulk. L'[adattatore Python][db-arrow-python] normalizza alcuni tipi Arrow e accumula l'IPC in `BytesIO` prima di `copy_from` | Progettare un adattatore Arrow opzionale in IO mantenendo le API esistenti. Un iterable accettato non prova memoria costante; il formato stream non prova esecuzione streaming. PyO3 e «zero-copy» non sono requisiti automatici |
| Errori, retry ed esiti | Il [modello database][db-errors] contiene anche `concurrent_modification` e `quarantine`; IO non li emette. Entrambi distinguono categoria, fase, effetto remoto e retry; la scrittura locale IO e una transazione database hanno esiti propri | Confrontare la proiezione pubblica con gli schemi comuni. Un adattatore deve conservare origine, diagnostica e indicazioni di recupero; non deve ritentare una scrittura con effetto sconosciuto né equiparare pubblicazione locale e commit remoto |
| Manutenzione e controlli | Database genera `docs/STATO.md` con [`render_state.py`][db-state] e confronta sweep offline/CI; IO dispone già di cataloghi derivati, registri e checkpoint | Riusare il principio di una fonte unica e della corrispondenza fra controlli dichiarati ed eseguiti. Evitare una seconda infrastruttura di qualifica; R2–R5 conservano il loro perimetro |

Questi sono rilievi da lettura dei sorgenti. Non sono risultati di una corsa
fra i due prodotti e non attestano il comportamento degli asset pubblicati.
In particolare, un limite osservato nel percorso CLI non si estende
automaticamente all'API Rust o al binding Python dello stesso componente.

### Possibili interventi di interoperabilità

Le voci seguenti sono **proposte aperte**, da delimitare dopo il confronto di
maturità. A1–A3 e A5 identificano le prove dei confini utili anche a M2–M4;
A4 e A7 aggiungono rispettivamente una superficie SDK e una qualifica fra
componenti, e non diventano obblighi della 4.1.0 soltanto per somiglianza con
database-tools. Le modifiche al componente vicino e ai contratti condivisi
hanno un seguito nel rispettivo repository; non sono già disponibili.

| ID | Intervento e responsabilità | Criterio di chiusura |
|---|---|---|
| A1 | Riconciliare profili, pin e confini pubblici — IO, database e contratti | Matrice per operazione/superficie con schema, content type, effetto e controlli; differenze normative distinte da scelte interne. Pin IO non retrocesso; ogni modifica comune concordata e versionata. C1–C4 restano riferimenti unici per le deviazioni già note |
| A2 | Provare lo scambio Arrow in entrambe le direzioni — IO e database | Fixture prodotte da un componente e consumate dall'altro sulle revisioni fissate, per `file` e `stream` separatamente. Verifica di valori, schema, nullability, ordine, identità dei campi e metadati; riproduttore del limite CLI database e relativo seguito. Lo scambio file non chiude gli archi `direct` che dichiarano stream |
| A3 | Allineare metadati, tipi e fedeltà — bordi Arrow dei due componenti | Vettori condivisi e casi di round-trip per geometrie/CRS, tipi tabellari e metadati nativi; perdita esplicita o rifiuto prima della scrittura se il sink non rappresenta il dato. Nessuna normalizzazione silenziosa; C2 e C3 governano tipi ignoti e assenza accertata |
| A4 | Rendere componibile lo SDK Python IO con il bulk database — IO | Adattatore opzionale per oggetti/batch PyArrow e IPC, con ownership e chiusura esplicite, senza cambiare `Client.read`/`write` né imporre PyArrow all'installazione base. Prove dalla wheel installata, incluso input vuoto con schema e fallimento a metà. Confini di materializzazione e costi dichiarati; migrazione nativa PyO3 solo con decisione separata |
| A5 | Verificare errori, cancellazione e atomicità nella pipeline — entrambi | Errori strutturati con componente d'origine e diagnostica conservati; timeout, cancellazione, limite e dato incompatibile distinti. Prove di interruzione a metà, cleanup, nessuna pubblicazione locale parziale e nessun retry automatico su effetto remoto `unknown`; il commit database non viene promesso come transazione distribuita col file |
| A6 | Decidere il percorso Rust diretto e il costo delle dipendenze comuni — entrambi | Crate d'integrazione minimo con i due core e tentativo documentato di risoluzione/compilazione dei pin Arrow. Allineamento coordinato se compatibile, oppure IPC come confine esplicito; nessuna promessa di `RecordBatch` condiviso senza prova e nessun aggiornamento massivo dei lock |
| A7 | Qualificare una pipeline di riferimento — IO con database fissato | Esempio riproducibile file → PostgreSQL/PostGIS → file, più prove nei due sensi sui tipi supportati. Evidenza nomina SHA, versioni/digest degli artefatti, provider, formato, mapping policy ed esiti. Gate offline sulle fixture durante lo sviluppo; prova live mirata sulla candidate, senza attribuire il risultato agli altri provider |

Se si estende la 4.1.0 all'interoperabilità effettiva, il percorso minimo
parte da **A1, A2 e A3**, poi dalle prove d'interruzione A5. L'adattatore Python
A4 richiede un beneficio concreto rispetto alle API esistenti; A6 è una
decisione distinta, perché l'IPC può essere il confine praticabile anche
quando i due core non condividono i tipi Rust. A7 raccoglie la prova
d'integrazione e non sostituisce la qualifica dei singoli prodotti.

La matrice dei casi A2–A3 deve almeno comprendere:

- schema senza righe, batch multipli, null e ordine delle colonne; identità
  preservate attraverso rinomina/proiezione;
- interi signed/unsigned ai limiti, decimali con precisione/scala, timestamp
  con unità e timezone, binary/string e varianti large, dictionary e tipi
  nested: ogni coppia sorgente/sink esplicita supporto, conversione o rifiuto;
- WKB/EWKB, dimensioni XY/XYZ/XYM/XYZM, geometrie vuote e null, dichiarazioni
  exact/mixed/unresolved; CRS risolto, dichiarato non risolto, assente e
  contraddittorio; SRID negativo e field id fuori dal dominio `u32`;
- metadati sconosciuti e provider-specifici, anche sui campi nested. Un giro
  dichiarato lossless li conserva; un formato che non li rappresenta rende
  osservabile la perdita prima di promettere fedeltà.

Per A7 si propone PostgreSQL/PostGIS come primo provider; il perimetro iniziale
proposto è Arrow IPC per l'identità e GeoParquet/GeoPackage per il giro geospaziale,
nei tipi che entrambi i lati dichiarano. Altri formati e provider entrano
solo con prove proprie. I dataset ostili e quelli fuori capacità devono
fallire nel punto previsto, non essere «aggiustati» per far passare il giro.

La 4.1.0 deve dichiarare **quali percorsi sono realmente qualificati**. Se una
voce dipende da una modifica del componente vicino o da una major dei
contratti, si registra il blocco e il seguito preciso: non si pubblica un
claim di interoperabilità completa. Una nuova chiave in uno schema chiuso,
una diversa semantica di consegna o una migrazione dello SDK non diventano
compatibili soltanto perché servono all'allineamento.

[db-base]: https://github.com/PlenoraETL/plenora-database-tools/tree/00d7613402e87a449a36e0ff7d91e7710999b822
[db-release]: https://github.com/PlenoraETL/plenora-database-tools/releases/tag/py-v4.2.0
[db-adoption]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/contracts/adoption-source.json
[db-catalog]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/crates/plenora-database-core/src/public_contract.rs
[db-ipc]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/crates/plenora-database-cli/src/ipc_input.rs
[db-cli]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/crates/plenora-database-cli/src/main.rs
[db-cargo]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/Cargo.toml
[db-fields]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/crates/plenora-database-core/src/field_contract.rs
[db-python]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/crates/plenora-database-py/README.md
[db-arrow-python]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/crates/plenora-database-py/python/plenora_database/_arrow_io.py
[db-errors]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/crates/plenora-database-core/src/error.rs
[db-state]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/scripts/render_state.py
[arrow-vocabulary]: https://github.com/PlenoraETL/plenora-contracts/blob/453c8d1ff2eb260840e6cedc033a2b76b58a0b9e/specs/data/ARROW-VOCABULARY-1.0.md
[db-ir-guard]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/scripts/check_relational_ir.py
[db-observation]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/crates/plenora-database-engine/src/metadata/mod.rs
[db-metadata-guard]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/scripts/check_typed_metadata.py
[db-ci]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/.github/workflows/rust-ci.yml
[db-coverage]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/scripts/coverage_budget.json
[db-ci-tests]: https://github.com/PlenoraETL/plenora-database-tools/blob/00d7613402e87a449a36e0ff7d91e7710999b822/scripts/test_ci_workflows.py
[db-ci-run]: https://github.com/PlenoraETL/plenora-database-tools/actions/runs/34798703096

## Riconciliazione con lo stato effettivo

Il piano è stato scritto mentre il rilascio era in corso. Alcune sue righe
descrivevano cose ancora da verificare, e adesso hanno una risposta. Questa
sezione le riconcilia una per una, perché un piano che continua a chiedere ciò
che è già stato fatto fa ripetere il lavoro.

| Riga del piano concordato | Stato effettivo al 15 settembre 2026 |
|---|---|
| «La conferma della CI finale resta da verificare» | **verificata.** CI verde su `8cf013f`, l'ultimo commit del verbale; verde anche su `4d4298c` e `47f8667` |
| «Il censimento non è presente nella radice» | **recuperato e intatto**, vedi la sezione seguente |
| «Il volume target conserva circa 144 GiB e il disco virtuale non è stato compattato» | **superata.** Il resoconto di pulizia registra la compattazione: `docker_data.vhdx` da 516.045.144.064 a 19.636.682.752 byte |
| «Un solo container: `plenora-err`» | **rimosso** dalla pulizia. Zero container; sei immagini distinte, con `plenora-io-dev:latest` che punta alla stessa immagine di `plenora-io-dev-198` |
| P0 «Confermare pubblicazione e verifica della 4.0.0» | **chiusa.** Tag `v4.0.0` su `6beb410b9984f527f3e89bba420764ac9e24e436`, ventitré asset riscaricati dalla release identici a quelli della bozza, stato registrato, CI confermata |

La Fase 0 è quindi chiusa nelle sue quattro voci. Resta vero il suo avvertimento
operativo: **non ripetere cancellazioni sulla base di un inventario precedente**,
perché l'ambiente è stato rimisurato dopo il rilascio e le cifre della prima
tranche sono storiche.

## Il censimento delle dipendenze: dov'è, e perché non era dove il piano diceva

Il piano lo cita «nella radice IO-tools». Non c'era, e la ragione è nota: il
15 settembre alle 07:47 UTC è comparso in radice **durante** la corsa di livello
2 su `6beb410`, e l'ha fatta cadere con tre passi rossi — `sonde_docset`,
`check_docset` e soprattutto `albero_invariato`, che dice che l'albero è cambiato
sotto la misura. È stato **spostato, non cancellato**, in
`C:\Users\marco\Desktop\censimento-librerie.md`, e la corsa è stata rifatta da
capo perché l'invarianza dell'albero non si recupera a posteriori.

Il documento è integro e **non gli manca nulla di dichiarato**: 121.133 byte,
misurato contro `6beb410b9984f527f3e89bba420764ac9e24e436`, con
`cargo tree --offline --locked` su profili base e filegdb per Linux e Windows
x86_64 — «8/8 interrogazioni riuscite», e alla voce «Verifiche online non
riuscite» dice «Nessuna».

Da questo blocco è nel docset come
[docs/CENSIMENTO-LIBRERIE-2026-09-15.md](CENSIMENTO-LIBRERIE-2026-09-15.md),
con la data nel nome perché è **una fotografia, non uno stato corrente**: il
grafo cambia al primo aggiornamento, e un documento canonico senza data
inviterebbe a leggerlo come se non fosse invecchiato. Si ritira quando gli
interventi L1–L4 sono chiusi e il grafo è stato rimisurato; il suo successore
porterà la propria data.

## Vincoli dei gate incontrati in questo blocco

Il mandato di questo primo blocco era «solo documentazione». Due vincoli lo
attraversano, e sono segnalati invece di aggirati.

**L'allowlist del docset è codice, non dati.** `CANONICI` è una lista Python in
`scripts/check_docset.py`, e l'allowlist è **esatta**: nessun Markdown tracciato
può stare fuori da essa. Ammettere questo piano e il censimento richiede quindi
una modifica a `scripts/`, che non è documentazione. È l'unica modifica di
questo blocco fuori da `docs/` e dal `README.md`, era prevista dal piano
concordato — «l'ammissione necessaria nel docset» — e consiste nell'aggiungere
due voci a una lista. Non tocca alcuna logica di verifica.

**Il gate del contratto non è un controllo documentale.**
`scripts/check_release_contract.py`, anche fuori dalla modalità release, esegue
l'harness dei test con `cargo`. Dopo la pulizia post rilascio il volume target è
vuoto, quindi una sua corsa ricompila il workspace da zero: su una modifica di
sola documentazione costa ore e non misura nulla che la documentazione abbia
cambiato. In questo blocco è stato avviato per abitudine e **fermato**, senza
lasciare modifiche. Non è un obbligo automatico alla chiusura di ogni blocco:
se comporta una verifica completa o lunga, appartiene alla qualifica finale.
Durante lo sviluppo si eseguono i controlli e le sonde pertinenti alla modifica.
Per la documentazione sono `check_docset.py`, le sue trentanove sonde e
`check_comments.py`.

**Le campagne lunghe non si lanciano durante lo sviluppo.** Checkpoint completi
di livello 1 e 2, soak prolungati, campagne complete di copertura e qualifica
degli artefatti si eseguono **sulla candidate finale**, prima del rilascio.
Durante lo sviluppo valgono prove mirate alle modifiche e controlli
proporzionati; chiudere un blocco non innesca una qualifica completa.
Dopo la qualifica finale, una campagna si ripete solo se una modifica ne
invalida le prove o emerge un problema concreto, dichiarando **quale misura è
invalidata e perché**. Se un gate pretendesse diversamente, il vincolo va
segnalato: non aggirato e non risolto avviando altre ore di test.

## Due rilievi aperti dalla pubblicazione

Non erano nel piano concordato perché sono emersi dopo. Nessuno dei due mette in
discussione la 4.0.0 pubblicata; entrambi vanno trattati nella 4.1.0.

### Q1 — il workflow del tag contraddiceva il modello a due revisioni: **chiusa**

La push di `v4.0.0` ha innescato «Release checkout qualification» sulla revisione
congelata, e il job `rust` è rosso con:

```
`candidate_release.tag_creato` vale «False» ... git trova il tag «v4.0.0»
```

Non è un difetto del prodotto né del rilascio. Il tag punta alla revisione
**congelata**, e in quella revisione lo stato non può sapere di un tag creato
dopo: `tag_creato` viene scritto in `2484019`, e `commit_di_assurance` nel
successivo, perché un commit non può nominare se stesso. Il workflow qualifica
il checkout del tag e vi applica un invariante che a quel checkout è falso per
costruzione.

**Chiusa distinguendo i due alberi, non allentando l'invariante.**

Il confronto fra `tag_creato` e git resta intero dove è nato: sul ramo di
sviluppo lo stato dichiarava `tag_creato: false` mentre `v1.0.1` esisteva, ed è
quella copia scritta a mano che il controllo esiste per prendere.

Sull'albero **congelato** la stessa domanda cambia oggetto, perché non ha una
risposta che quel commit possa dare. Lì la pretesa diventa: `tag_creato`
**dev'essere falso**, e un `true` sarebbe un commit che afferma di conoscere un
tag creato dopo di sé. La registrazione del tag si verifica dov'è scritta, cioè
sul commit di assurance, dove il confronto gira nella sua forma piena.

La distinzione sta nel gate, in `_candidate_legata_alle_fonti`, e non nel
workflow: è una proprietà del modello a due revisioni, non di quella corsa, e
ripeterla nel workflow vorrebbe dire due luoghi che possono divergere. Il
workflow la **nomina**, e una prova pretende che continui a farlo.

Sei regressioni, e la prima riproduce il guasto: HEAD sulla revisione
congelata, il tag che esiste e punta lì, lo stato che dice `false`. Verificato
che fallisca senza la correzione.

### Q2 — panico upstream in parquet raggiunto da GeoParquet malformato

Lo smoke fuzz della corsa «Release checkout qualification» 34988124226, sulla
revisione pubblicata, ha trovato un finding nuovo su `geoparquet_reader`:

```
thread panicked at parquet-59.3.0/src/encodings/decoding/byte_stream_split_decoder.rs:61:38
index out of bounds: the len is 2 but the index is 2
```

L'input è conservato: 3.966 byte, magic `PAR1`,
`sha256=18cb2a7e0e394484…`, in
`IO-tools-evidenze-4.0.0\finding-geoparquet-34988124226\`.

**La causa è individuata.** In `parquet-59.3.0`,
`ByteStreamSplitDecoder::get` ricava due quantità da due fonti diverse — lo
`stride` dai byte **effettivi** della pagina, il conteggio del ciclo da
`total_num_values`, cioè dal **dichiarato** — e `join_streams_const` indicizza
`sub_src[i + j * stride]` senza confrontare l'indice con la lunghezza. Il campo
incoerente è `num_values` nell'header della data page.

Non è una descrizione dedotta dal panico: è dimostrata costruendola. Un
riproduttore scrive un GeoParquet **valido** con una colonna `DOUBLE`
`BYTE_STREAM_SPLIT`, verifica che si legga, poi altera **un solo byte** — lo
zigzag di 63 occupa lo stesso spazio di quello di 8, quindi nessun offset si
muove — e prova tutti i siti in cui quel varint compare. Solo l'offset 13, nel
header di pagina, produce l'indicizzazione fuori limite; l'offset 17, un altro
campo dello stesso header, non produce nulla. Il primo tentativo sostituiva il
varint con uno più lungo, spostava il resto e otteneva `EOF: Invalid page
header`: struttura rotta, non incoerenza semantica, e nessuna dimostrazione.

**Il prodotto non crolla, e non serve una 4.0.1.** Sul binario estratto
dall'archivio pubblicato: `read` esce 3 con `FORMAT_ERROR`, categoria
`data_mapping`, fase `read`, `retry: never`. Zero segnali, zero timeout. La
barriera `catch_unwind` sta in `plenora-io-core/src/driver.rs`, cioè nella
**libreria**: un consumatore Rust che chiama il driver direttamente riceve lo
stesso errore tipizzato, verificato separatamente dalla CLI.

Tre comandi su quattro **non** provano la barriera, e va detto: `inspect` e
`layers` leggono footer e schema senza decodificare pagine; `convert` si ferma
in fase `validate`, e il controfattuale lo dimostra — sullo stesso percorso con
un file valido esce `ok`, quindi sa leggere, e sul seme non ci arriva. L'unico
comando che esercita il decoder è `read`.

**Il finding resta valido.** L'`abort()` che `libfuzzer-sys` chiama prima
dell'unwinding impedisce al target di osservare il recupero; non rende
inesistente il difetto a monte.

#### Q2a — regressione locale: **chiusa**

`crates/driver-geoparquet/tests/byte_stream_split.rs` pretende i quattro assi
dell'errore e che il panico non sia propagato;
`byte_stream_split_sintetico.rs` è il riproduttore che identifica il campo. La
fixture sta in `crates/driver-geoparquet/tests/fixtures/`, **fuori** da
`fuzz/seeds/`, `fuzz/corpus/` e `fuzz/artifacts/`, che `scripts/fuzz-replay.sh`
riesegue tutte e tre.

Che le ultime due siano ignorate da Git non le esclude dal replay: dice solo che
una copia appena clonata parte senza. In locale persistono e il replay le legge —
`fuzz/artifacts/geoparquet_reader/` contiene oggi tre input del 2026-08-17, che
il replay del livello 2 su `6beb410` ha rieseguito verdi.

#### Q2b — segnalazione upstream: **chiusa, inviata il 2026-09-17**

Il contributo è in `upstream/arrow-rs-byte-stream-split/`: un progetto Cargo
autonomo, fuori dal workspace, che dipende solo da `parquet`, `arrow-array` e
`arrow-schema`. Chi riceve la segnalazione lo compila senza avere questo
repository. Accanto sta `SEGNALAZIONE.md`, il testo da mandare, in inglese
perché il destinatario è un progetto di terzi.

**Un fatto nuovo, ed è quello che rende la segnalazione utile: la 60.0.0 è
ancora interessata.** Il nostro pin è la 59.3.0, e il difetto si sarebbe potuto
chiudere aggiornandolo; non è così. Verificato portando il riproduttore alla
60.0.0 e rieseguendolo: stesso panico, stessa riga 61 del decoder, stesso
messaggio. Il riproduttore punta perciò alla versione corrente, e il documento
dice che la 59.3.0 riproduce identica.

**La causa, letta sul codice e non dedotta.** `set_data` tiene affiancate due
grandezze di origine diversa — `encoded_bytes`, i byte **effettivi** della
pagina, e `total_num_values`, il conteggio **dichiarato** nell'header — e non le
riconcilia. `get` prende poi lo stride dai primi e il limite del ciclo dal
secondo, e `join_streams_const` indicizza senza confronto. Vale la pena notare
che il decoder a larghezza **variabile** un controllo in `set_data` ce l'ha:
qui sembra un'omissione, non una scelta.

**Che cosa il riproduttore fa, e in quale ordine.** Scrive un Parquet valido, lo
rilegge — il controfattuale viene prima, o l'alterazione non spiegherebbe nulla
— altera **un solo byte** e rilegge. Prova **tutti** gli offset in cui quel
varint compare invece di scegliere quello giusto: l'offset 13 scatta, il 17, un
altro campo dello stesso header, non produce niente. Lascia i tre file sul disco
perché chi legge possa aprirli con altri strumenti.

**Criterio di chiusura**: il riferimento alla issue. Finché non c'è, la voce
resta aperta — il contributo è pronto, non inviato, e le due cose non si
confondono.

**Chiusa il 2026-09-17**: [`apache/arrow-rs#11121`][arrow-11121], aperta con
l'identità `PlenoraETL`, la stessa dei sei contributi precedenti. Il corpo porta
`Cargo.toml` e `src/main.rs` per intero, in modo che il caso si compili senza
avere questo repository; l'unico adattamento rispetto ai file versionati è la
traduzione di due righe di commento che nominavano il nostro workspace, perché
correggerle qui avrebbe toccato `upstream/`, che sta fuori dall'allowlist del
congelamento.

**Che cosa questa chiusura non dice.** Non dice che il difetto sia corretto:
inviare e correggere sono due cose diverse, ed è la stessa distinzione che il
censimento dei contributi fa fra «preparato», «inviato» e «accettato e
rilasciato». La prima via per `campagna-fuzz-completa` — la correzione upstream
nel pin — è ora *cominciata*, non percorsa, e dipende dai tempi di `arrow-rs`.
La verifica dovuta resta: se il riproduttore smette di riprodurre, la voce va
tolta invece di lasciarla invecchiare.

[arrow-11121]: https://github.com/apache/arrow-rs/issues/11121

#### Q2c — gestire i finding noti del fuzz: **chiusa**

Il problema, scritto prima del meccanismo e lasciato com'era:

> gestire finding noti **senza** disabilitare il bersaglio, **senza** nascondere
> difetti nuovi, e **senza** dichiarare completa una campagna che si è
> interrotta.

##### Perché non la quarantena, e perché non una lista di digest

La quarantena del progetto è per **bersaglio**: mettere `geoparquet_reader` in
`fuzz/quarantine.txt` smetterebbe di esplorarlo, e il file stesso riserva quella
via ai finding dove «uno smoke che fallisce sempre non è un gate, è rumore». Un
finding solo non giustifica di smettere di cercarne altri.

Una lista di digest non basta, e la prova era già nel repository: il fuzzer
rigenera lo stesso difetto da un input diverso. Gli artefatti locali di
`geoparquet_reader` lo mostrano — due misurano 3.966 byte, la stessa dimensione
del seme del finding, con digest tutti diversi dal suo. Stessa famiglia, quattro
digest, e una lista li riconoscerebbe uno per volta.

##### Che cosa si riconosce: la famiglia

`scripts/classifica_finding_fuzz.py` legge l'uscita della corsa e ne ricava una
firma di **due** parti, contro il registro
`assurance/registries/finding-noti-fuzz.json`:

| parte | perché così |
|---|---|
| il modulo, col nome della crate **spogliato della versione** | `parquet-59.3.0` e `parquet-60.0.0` sono la stessa crate, e un difetto aperto a monte non smette di esserlo quando il pin sale |
| la **forma** del messaggio, con le cifre ridotte a `N` | «the len is 2 but the index is 2» e «the len is 56…» sono lo stesso difetto su due ingressi |

Il numero di riga **non** è nella firma: si sposta fra versioni, e legarlo
renderebbe stantia una voce che descrive un difetto ancora aperto. Resta
registrato come dato, perché chi rilegge voglia vedere dove si era manifestato.

Il percorso perde anche il prefisso del registro di cargo: altrimenti la firma
dipenderebbe da **dove gira il fuzzer**, e la stessa corsa su un'altra macchina
non riconoscerebbe nulla.

##### Che cosa garantisce, e che cosa no

**La garanzia va detta per quello che è, e la prima stesura la diceva troppo
forte.** Modulo e forma del messaggio riconoscono un crash **compatibile** con
una firma registrata. Non dimostrano che sia lo stesso difetto: un conteggio
incoerente e un offset calcolato male finiscono entrambi su `index out of
bounds` nello stesso modulo, e la firma non li separa. «Noto» vuol dire «già
visto qualcosa che si presenta così», non «già capito».

Da qui discende il prezzo della garanzia più debole, e sono due cose concrete.

**Ogni input si conserva**, con il referto che sostiene la classificazione, in
`assurance/evidence/finding-fuzz/`. Se fosse identità certa si potrebbe scartare
il duplicato; essendo compatibilità, l'input è l'unica cosa che permette di
riesaminare la classificazione più tardi. Vale anche per i crash nuovi, e il
referto scrive a chiare lettere che «noto» è una compatibilità.

**La firma resta stretta** — modulo *e* forma del messaggio, congiunti, per il
bersaglio dichiarato — non perché basti a identificare la causa, ma perché
allargarla aumenterebbe i crash nella stessa cesta senza aumentare di nulla ciò
che si sa di loro. Un altro messaggio nello stesso modulo è fuori; lo stesso
messaggio in un altro modulo è fuori; lo stesso crash su un altro bersaglio è
fuori. Ciascuna delle tre ha la sua prova, e un crash che il classificatore non
riesce a leggere è rosso: una corsa fallita senza panico riconoscibile è un
guasto, non un finding noto.

**Non disabilita il bersaglio.** Il target continua a girare, con tutto il suo
corpus. Ciò che cambia è la lettura del crash, non l'esecuzione.

##### L'interruzione nota, e dove si perdeva

È la garanzia solida delle tre, ed è quella che costa di più. libFuzzer si ferma
al primo crash: un bersaglio fermato **non ha esplorato il tempo restante**. I
tre stati hanno perciò tre codici — 0 nessun crash, **3** compatibile con un
noto, 1 nuovo — e il 3 non è 0 apposta.

Ma una riga stampata non basta, e qui c'era una falla: la CI e il passo
`fuzz_smoke` del checkpoint leggono l'**esito** dello smoke, non ciò che
stampa. Con lo smoke a 0, una corsa interrotta sarebbe stata registrata come
passo riuscito, e l'evidenza di livello 2 avrebbe affermato una campagna che non
c'è stata. La nota dell'invariante `fuzz.quarantena` lo dice da sé: la
completezza dell'esecuzione è verificata «dai passi `fuzz_replay` e
`fuzz_smoke`».

Lo smoke lascia perciò un **verbale**, `assurance/evidence/fuzz-smoke-ultima.json`,
con chi ha finito, chi si è fermato a un crash compatibile con un noto e chi è
fallito su uno nuovo — e i falliti non sono contati fra i finiti, o il verbale
sarebbe più generoso della corsa. Il verbale si scrive **prima** dell'uscita
rossa: una corsa che ha trovato un finding nuovo è comunque una corsa avvenuta,
e cancellarne la traccia lascerebbe la qualificazione a rileggere quella di
prima.

La nona condizione di autorizzazione, `campagna-fuzz-completa`, rilegge quel
verbale e rifiuta se qualcuno si è fermato. È la stessa forma del rinvio delle
rimisurazioni: **verde in sviluppo, rossa in qualificazione**. E l'assenza del
verbale è rossa anch'essa — non è una campagna riuscita, è l'assenza di una
campagna.

##### Che cosa costa una voce

Sette campi obbligatori, e due meritano di essere nominati: `non_promette`, cioè
che cosa la campagna smette di affermare finché la voce c'è, e
`quando_si_toglie`, cioè la condizione che la chiude. Una voce senza questi due
non è una registrazione, è un permesso a tempo indeterminato — e ciascuno dei
sette ha una prova che lo pretende.

La voce di oggi è una sola, `arrow-rs-byte-stream-split-oob`, e si toglie quando
la correzione upstream è pubblicata e il pin la include. La prova che sia ancora
dovuta è il riproduttore di Q2b: se smette di riprodurre, la voce va tolta
invece di essere ereditata.

##### Dove gira

Le diciotto regressioni stanno in CI accanto a quelle della quarantena, e nel
checkpoint come `sonde_finding_noti`: le due vie — per bersaglio e per famiglia
di crash — si leggono insieme. Una delle prove verifica che lo smoke invochi il
classificatore e distingua il codice 3, perché se quel collegamento sparisse un
finding noto tornerebbe a far fallire la corsa e l'unica via sarebbe di nuovo la
quarantena per bersaglio.

## Compatibilità e criteri generali

La 4.1.0 conserva API pubbliche, protocollo CLI, semantica degli errori, formati
accettati, opzioni offerte, metadati, atomicità e proprietà di fedeltà promesse
dalla 4.0.0. Una versione esterna più recente non è automaticamente compatibile.

Se chiudere un residuo richiede una rottura, si prepara una soluzione
compatibile oppure quel punto passa a una major successiva. Non si impone la
versione più alta a una dipendenza transitiva contro i vincoli dei suoi
chiamanti. Non si eliminano controlli di sicurezza né si riscrivono parser per
ottenere un conteggio di librerie inferiore.

Ogni voce si chiude con uno dei tre esiti: **risolta con prova**, **ancora
necessaria con motivazione**, **dipendenza upstream o incompatibilità
identificata con seguito preciso**.

## Fase 1 — residui delle prove e della procedura

| ID | Intervento | Criterio di chiusura |
|---|---|---|
| R1 | Regressione dedicata a `MAX_BLOCCHI` nella prevalidazione Arrow | **Chiusa**: `tetto_dei_messaggi` in `driver-common/src/prevalida_arrow.rs`, tre casi. Sotto e al tetto la guardia tace; uno oltre parla, e si verifica **quale**. Costo misurato: il messaggio più piccolo senza nuove dipendenze è quello di schema, 128 byte, quindi 128 MiB generati in 60 ms e scanditi in 3,5 s. La fixture è generata a ogni corsa, non versionata. Che il caso «al tetto» discrimini è verificato spostando la guardia di uno: diventa rossa da sola |
| R2 | Togliere alle campagne la dipendenza da `sleep` a scadenza e dalla cattura di `docker exec` | **Chiusa.** Tre delle quattro proprietà erano già in `fuzz-container.sh`: il container sopravvive al client (`run -d`), l'exit code viene da `docker inspect` e mai dal log, e il container non si rimuove prima di aver acquisito l'esito — un'interruzione senza exit code non è né verde né rossa. La quarta mancava: `collect` stampava venti righe di log e poi rimuoveva. Ora lo salva **intero** su disco prima, e se non riesce **non rimuove**. La durata la verifica `soak_misurato.py`, chiuso con R6. Sette regressioni con un Docker finto, fra cui l'ordine salva-poi-rimuove e il rifiuto di rimuovere senza log. *Correzione successiva*: `stop` faceva `rm --force` — fermava e distruggeva insieme, senza acquisire l'esito né salvare il log, disfacendo a due righe di distanza tutto ciò che `collect` conservava. Ora ferma e lascia raccoglibile; buttare via si chiede per nome, con `scarta`. La prima prova esercitava solo il ramo «già fermo» e non vedeva il difetto, che stava in quello vivo: estesa. *Limite residuo*: due corse avviate nello stesso secondo condividerebbero il nome del file |
| R3 | Imporre il legame fra candidate, revisione qualificata ed evidenza corrente | **Chiusa.** *Requisito*: fixture che rifiutano una misura assente o di un'altra revisione, distinzione fra registrazione entro allowlist, evidenza storica e riuso, nessun collegamento affidato al solo verbale. *Prova*: `_misura_legata_all_evidenza` pretende che l'evidenza esista, che il suo esito sia un livello 2 superato, che lo SHA risolva e compaia nel nome del file, e che la revisione qualificata coincida; `cambiamenti_dopo_il_congelamento` separa l'allowlist dal resto; `profilo-pubblico-attestato` e `campagna-fuzz-completa` tolgono dal verbale umano i due collegamenti che vi erano rimasti. *Limite residuo*: il legame è per **revisione**, non per contenuto — due alberi con lo stesso SHA sono lo stesso albero, e la qualifica pretende già un checkout pulito, ma un'evidenza prodotta altrove e copiata nel posto giusto non è distinguibile da una prodotta qui |
| R4 | Rendere esplicito il riuso delle evidenze | **Chiusa.** *Requisito*: una regola per tipo di modifica — prodotto, test, documenti, toolchain, feature, lock — con prova della validità e della provenienza, e la revisione realmente misurata conservata. *Prova*: `assurance/registries/riuso-delle-evidenze.json` dice per ciascuno dei sei che cosa invalida, che cosa resta valido e **chi se ne accorge**; `check_riuso_evidenze.py` pretende i sei come insieme chiuso, ogni campo pieno, e che ogni sorvegliante nominato esista sul disco. Nove regressioni. La revisione misurata resta in `ultima_misura.sha`, confrontata con la qualifica. *Limite residuo*, scritto anche nel gate: verifica che la regola sia **scritta** e che i sorveglianti **esistano**, non che se ne accorgano — quello lo provano le loro regressioni |
| R5 | Evitare corse concorrenti duplicate e push dopo controlli già falliti | **Chiusa.** *Requisito*: una misura per SHA e perimetro, esiti controllati prima dei passi dipendenti, monitor legato allo SHA e alla corsa esatta, nessuna diagnosi sulla corsa precedente. *Prova*: `comando_start` rifiuta se un container esiste già — viva o finita e non ancora letta — invece di sovrascriverlo; l'esito viene da `docker inspect` e mai dal log; lo SHA si incide nell'etichetta **all'avvio**, perché dedurlo dopo risponderebbe di un altro albero, e `status` lo dice in **entrambi** i rami — anche a corsa finita, che è il momento in cui si conclude qualcosa. *Correzioni successive, e sono le più profonde*: l'etichetta fissava lo SHA, il mount no. Il container montava il checkout vivo, quindi una modifica durante la campagna entrava nelle compilazioni successive mentre l'etichetta conservava lo SHA iniziale — l'attribuzione diventava falsa proprio dove sembrava più solida. La corsa gira ora su un **clone isolato** della revisione, e un albero di lavoro sporco non avvia. Poi tre difetti nell'isolamento stesso: il clone si riusava senza verificarlo — bastava che esistesse `.git`, e un sorgente toccato dentro veniva compilato con l'etichetta che continuava a nominare la revisione — la pulizia leggeva l'etichetta **dopo** aver rimosso il container, quindi non trovava niente e il clone restava, e il percorso del clone non passava dalla conversione per Docker che `radice_repo` faceva già. Ventisei regressioni con un Docker finto e un repository git vero. *Limiti residui*: nome di container fisso, quindi due campagne su perimetri diversi non convivono senza scegliere `PLENORA_FUZZ_CONTAINER`; e la conversione del percorso ha un ramo Windows che le prove, girando su Linux, non esercitano |
| R6 | Versionare il misuratore del soak con le prove dei casi già osservati | **Chiusa**: `scripts/soak_misurato.py`, regressioni in `scripts/test_soak_misurato.py` collegate a CI e checkpoint; CPU diagnostica; originali e giudizio corretto in `scripts/fixtures/soak/` |

I due requisiti che M4 consegna stanno in R3 e R4 e non in una voce nuova,
perché sono la stessa domanda già posta lì: che cosa lega ciò che si dichiara a
ciò che è stato davvero misurato. Il registro dei requisiti, le sonde e le
evidenze esistono già e vanno riusati; ciò che manca è che l'esecuzione sia
imposta e che il suo esito sia ancorato ai byte.

R4 non significa ereditare automaticamente una qualifica completa dopo una
modifica ai test. Si definisce e si prova prima quali evidenze restano valide,
quali cambiano e come il gate le riconosce.

I tre difetti che R6 deve pinnare sono stati osservati durante la qualifica
della 4.0.0, e vanno scritti perché riguardano la fiducia nella misura:

1. confrontare i due soli orologi interni non rileva il congelamento della
   **VM**, perché si fermano insieme — dopo sedici ore di ibernazione dell'host
   il loro scarto valeva `0 s` su una corsa inservibile;
2. un'uscita diversa da zero non è un finding se la campagna non è mai partita:
   è successo con `rustup` assente dal PATH;
3. la soglia sull'uso della CPU boccia una campagna valida quando il fuzzer non
   è legato alla CPU, e non era un requisito del soak.

Il confronto del corpus fra mount Windows e volume Linux usa **lo stesso
bersaglio e gli stessi input**: l'I/O resta una causa possibile e non
quantificata del tempo non contabilizzato, non una spiegazione dimostrata.

R6 conserva i byte del misuratore e del referto originali, con digest verificati
dalle sonde, e un estratto separato del giudizio corretto già registrato in
`assurance/evidence/checkpoint-6beb410.json`. Il referto originale continua a
dire `durata_dimostrata: false`. La nuova analisi degli stessi numeri accetta
la durata senza imporre saturazione CPU; non costituisce una nuova campagna.

Il misuratore pretende un riepilogo conclusivo con durata sufficiente oltre
all'intervallo degli orologi: la preparazione non completa un soak troppo breve.
Riconosce i segnali di finding anche senza `Done`; un exit nonzero senza tali
segnali resta da classificare. La concordanza degli orologi significa **nessuna
discontinuità rilevata**, e uno scarto non identifica da solo la causa.
R2 resta aperto per il ciclo di vita della campagna e R3 per il legame con la
candidate. Uso e limiti sono descritti in [ENGINEERING.md](ENGINEERING.md#misurare-il-soak).

## Fase 2 — aggiornamenti delle librerie esterne

Le ultime stabili si rimisurano prima di intervenire: i numeri qui sotto sono la
fotografia del censimento, non obiettivi mobili della candidate.

| ID | Intervento | Punto di partenza | Criterio di chiusura |
|---|---|---|---|
| L1 | Aggiornare `rust_xlsxwriter` | 0.99.0 → **0.99.1 adottata** | **Chiusa**, commit `7d3c461`: giro XLSX confrontato sulle due versioni, schema/metadati/fedeltà/valori invariati; grafo confrontato riga per riga nei quattro profili/target |
| L2 | Aggiornare `jsonschema` | 0.55.1 → **0.56.0 adottata** | **Chiusa**, commit `d2d35a7`: breaking upstream fuori dalla superficie usata; verdetti GeoParquet invariati; feature e grafo confrontati, resolver remoti ancora esclusi |
| L3 | Esaminare aggiornamenti transitivi e advisory | Versioni e catene nel censimento | Patch compatibili distinte dai salti richiesti dagli upstream; nessun aggiornamento massivo incontrollato; audit e deroghe riallineati |
| L4 | Rimuovere il solo pin diretto inutilizzato `arrow-select` | Nessun crate lo eredita; Parquet lo introduce transitivamente | Dichiarazione e registri coerenti, grafo invariato. **Non** si presenta come libreria eliminata dal binario |

L1 e L2 sono identificatori di **questa tabella**, non livelli di checkpoint.
Interventi con rischi diversi restano separati; le registrazioni dello stesso
blocco si riuniscono prima della verifica.

**Chiusura L1, 16 settembre 2026.** Il commit
[`7d3c461`](https://github.com/PlenoraETL/plenora-IO-tools/commit/7d3c461ae8157fa16541b855284f94f0f7cd64d2)
contiene soltanto `Cargo.toml` e `Cargo.lock`; il suo messaggio conserva il
resoconto delle verifiche. I quattro grafi base/`gdal-backend` × Linux/Windows
mantengono 241/242/243/244 pacchetti e differiscono soltanto nella versione di
`rust_xlsxwriter`; nel lock cambiano versione e checksum, nessuna transitiva.
Sono registrate 54 prove del driver, tre suite d'integrazione XLSX, fmt e
clippy con `-D warnings`, più due giri osservabili con referti identici.
Il confronto valido usa davvero 0.99.0 e 0.99.1: il primo tentativo, in cui
il pin esatto aveva impedito il ritorno alla versione precedente, è stato
scartato e rifatto. Nessuna incompatibilità osservata, nessun checkpoint
completo o campagna. Questo aggiornamento del piano registra tali prove,
non dichiara di averle rieseguite.

**Chiusura L2, 16 settembre 2026.** Il commit
[`d2d35a7`](https://github.com/PlenoraETL/plenora-IO-tools/commit/d2d35a7b21b99476b269e5eb0acfbff9e4d3b4d2)
registra il passaggio a 0.56.0 di `jsonschema`, `jsonschema-regex`,
`jsonschema-value` e `referencing`. I quattro grafi mantengono
241/242/243/244 pacchetti: il confronto riga per riga cambia solo quei quattro
crate, nessun'altra transitiva. Il breaking dichiarato riguarda
`canonical::CanonicalView::Raw`, non usato dal prodotto. Nel grafo normale
risolto prima e dopo risultano assenti `reqwest`, `ureq`, `hyper`, `tokio`,
`rustls`, `native-tls`, `idna` e `url`; restano disabilitate le feature di
risoluzione remota. I referti su sette file GeoParquet per tre comandi,
inclusi codici d'uscita, categoria, fase e messaggio, coincidono fra le due
versioni. Il lock della baseline è stato verificato per tutti e quattro i
crate. Il verbale registra 145 prove di `driver-geoparquet` (17 sonde dello
schema incluse), 124 di `plenora-io-tools`, fmt e clippy sui crate interessati.
Nessuna incompatibilità osservata nel perimetro provato; nessun checkpoint
completo, soak o campagna di copertura. Qui si registra il verbale del commit,
senza attribuirsi nuove esecuzioni né estendere il risultato agli altri residui.

## Fase 3 — versioni duplicate

La fotografia dei profili base con dipendenze ordinarie e di build mostra sette
nomi duplicati su Linux e otto su Windows. `syn` e `thiserror-impl` riguardano
le macro: una duplicazione non equivale a due copie di codice runtime. Le
versioni presenti soltanto nei lock per altri target o per i test non sono
duplicazioni del binario distribuito.

| ID | Libreria e versioni osservate | Causa e lavoro richiesto |
|---|---|---|
| D1 | `num-traits` 0.1.43 / 0.2.19 | Modernizzare `enum_primitive` nel fork DXF; verificare parser e generatori su valori enum validi e invalidi |
| D2 | `quick-xml` 0.41.0 / 0.42.0 | La vecchia è richiesta sia da `kml` sia da `calamine`: coordinare entrambi gli aggiornamenti preservando le codifiche supportate |
| D3 | `num-bigint` 0.4.8 / 0.5.1 | 0.4 è trattenuta da DXF/`num` e da `jsonschema`/`fraction`, 0.5 da Arrow: trattare entrambe le catene |
| D4 | `getrandom` 0.3.4 / 0.4.3 | Allineare i chiamanti, `ahash` e `tempfile`/`uuid` inclusi, rispettando target e feature; nessuna forzatura dal lock |
| D5 | `thiserror` e `thiserror-impl` 1.0.69 / 2.0.19 | WKT mantiene la linea precedente; aggiornare il chiamante con la semantica degli errori preservata |
| D6 | `syn` 2.0.119 / 3.0.3 | Dipendenze delle macro: seguire l'aggiornamento dei generatori. Misurare il costo di compilazione, non presumere costo runtime |
| D7 | `windows-sys` 0.52.0 / 0.61.2 | `atomicwrites` mantiene 0.52: aggiornamento upstream o sostituzione equivalente del wrapper Windows |

Per ogni eliminazione: confronto del grafo prima e dopo sugli stessi target,
profili e feature, e identificazione dei rami che mantengono ancora una
versione. Il solo conteggio di `Cargo.lock` non è un risultato.

## Fase 4 — riduzione di feature e catene

| ID | Candidata | Intervento e limite |
|---|---|---|
| F1 | ZIP → Zopfli | Verificare la disattivazione del compressore non usato dal writer XLSX, conservando Deflate. Va coordinata la richiesta di feature del workspace, di `calamine` e di `rust_xlsxwriter`: cambiarne una sola non basta |
| F2 | DXF → `image` e miniature BMP | Rendere opzionale la decodifica delle miniature solo con una politica compatibile per DXF validi e malformati. Oggi il parser le decodifica davvero: non si cancella `image` dichiarandola inutilizzata |
| F3 | Publisher Windows → `atomicwrites` | Valutare aggiornamento o sostituzione senza perdere no-clobber, atomicità e supporto directory. Nessun `unsafe` aggiunto al workspace per ridurre un conteggio |
| F4 | DXF → `num` | I due usi diretti di tipi di errore si possono semplificare con `std`, ma `jsonschema`/`fraction` continua a trattenere `num`. Dichiarare l'eventuale beneficio locale senza inventare una riduzione globale |
| F5 | `jsonschema` e validatori non usati dagli schemi incorporati | Valutare modularizzazione upstream. Non si scrive un validatore sostitutivo ad hoc e non si toglie il controllo dei metadati GeoParquet |
| F6 | Backend Deflate | Studiare l'unificazione della selezione `zlib-rs`/`miniz_oxide` con prove sui codec e misure reali. Due crate nel grafo non provano che entrambi i motori siano nel binario |
| F7 | Chiusura nativa GDAL | Valutare se una distribuzione GDAL più limitata sia sostenibile per FileGDB. Non si eliminano singole librerie dal prefisso: una variante richiede nuovi lock, provenance, SBOM e qualifica Linux e Windows |

Vincoli già corretti, da conservare: default KMZ di `kml` disabilitato; codec
immagine DXF limitati al BMP; feature Excel opzionali non abilitate; resolver
HTTP, file, TLS e IDNA di `jsonschema` disabilitati. I codec Parquet offerti dal
catalogo e le funzioni di orologio e UUID realmente usate non sono tagli
gratuiti.

F5, F6 e F7 sono analisi con decisione di convenienza, non autorizzazioni
preventive a introdurre fork o sostituire architetture. Se il costo supera il
beneficio, la valutazione si chiude con motivazione e la dipendenza resta.

## Fase 5 — fork e contratti condivisi

| ID | Intervento | Criterio di chiusura |
|---|---|---|
| U1 | Riconciliare tutti i delta di DXF, GDAL e Shapefile | Elenco completo confrontato **semanticamente** con le release ufficiali, non ricerca di nomi né stato `mergeable` di una PR |
| U2 | Aggiornare le basi e rimuovere le patch assorbite | La release ufficiale contiene il comportamento necessario e le regressioni passano senza la patch. Il fork si rimuove intero solo quando nessun delta necessario resta |
| U3 | Monitorare versioni e risposte upstream periodicamente | Segnalazione separata dalla CI della candidate: una release di terzi non rende rossa una revisione già qualificata |
| C1 | Trattare la deviazione `side_effect` di `io.read` | Soluzione concordata col contratto comune, o comportamento compatibile. La deviazione non è più presente solo quando il requisito è **effettivamente** soddisfatto |
| C2 | Trattare la regola di pubblicazione non espressa dal catalogo | Descrittore e comportamento concordano sul rifiuto dei tipi `unresolved` e sulle garanzie realmente offerte |
| C3 | Conservare sul filo la distinzione «scansione completa, nessuna geometria» | Coordinamento con la decisione 0006 e la PR #6 e col vocabolario successore; prova del secondo giro senza perdita della distinzione; compatibilità esplicita coi lettori precedenti |
| C4 | Riesaminare il ruolo provider e le deviazioni collegate | Nessun campo inventato per chiudere una riga; decisione comune se il significato riguarda più componenti |

Una chiave nuova non è automaticamente compatibile: i vocabolari possono essere
chiusi e le versioni ignote rifiutate. Se C3 non è realizzabile preservando la
compatibilità promessa dalla 4.0.0, la modifica incompatibile resta per una
major successiva.

Il pin corrente dei contratti condivisi è
`453c8d1ff2eb260840e6cedc033a2b76b58a0b9e`. La PR #6 e la decisione 0006 si
riesaminano **sullo stato effettivo**: al controllo del 15 settembre la
[PR #6](https://github.com/PlenoraETL/plenora-contracts/pull/6) è aperta e non
integrata, con head `0ca3d43ce255e3acb8b8481fc90ac0cdf3ada269`. La decisione
0006 non è nel `main` adottato. La proposta non modifica il contratto corrente.

L'interoperabilità effettiva con `plenora-data-tools` e `plenora-database-tools`
era stata differita per decisione dell'utente. La richiesta di allineamento e
la precisazione sulla maturità aprono ora l'analisi M1–M5 e le proposte A1–A7,
non una qualifica già eseguita o l'adozione automatica di un nuovo SDK.
Il supporto dei content type previsti non equivale a interoperabilità provata.
L'estensione effettiva della candidate a una pipeline fra componenti richiede
un perimetro scelto fra quelle proposte; data-tools resta fuori dal confronto.

## Un documento che non può scadere

`docs/PIANO-4.0.0.md` porta scritto che «quando la 4.0.0 è qualificata, questo
documento esce dall'allowlist invece di restare a invecchiare». La 4.0.0 è
qualificata e pubblicata, e la scadenza **non si può eseguire**: le due
deviazioni registrate in `contracts/adozione-4.0.0.json` indicano come proprio
luogo di tracciamento le sue righe B10 e B12c. Cancellarlo romperebbe i
riferimenti di due deviazioni ancora aperte.

Il documento resta quindi nel docset, e la sua scadenza si sposta a quando le
deviazioni C1 e C2 di questo piano sono chiuse — cioè quando il tracciamento non
ha più nulla da indicare. È un esempio del motivo per cui una scadenza scritta
in un documento va riconciliata invece che eseguita.

## Dipendenze: uso effettivo, chi le introduce, condizioni per rimuoverle

Per ogni dipendenza del grafo, la distinzione fra uso effettivo, catena di
introduzione e condizioni di rimozione sta nella tabella estesa del censimento,
[docs/CENSIMENTO-LIBRERIE-2026-09-15.md](CENSIMENTO-LIBRERIE-2026-09-15.md),
sezione «Tutte le dipendenze Rust: funzione, catena e possibilità di rimozione».
Non è ricopiata qui: due elenchi della stessa cosa divergono, e quello con la
data è la misura.

Il criterio che il censimento applica va letto prima delle sue tabelle:
**necessaria significa usata per conservare le capacità attuali**, non
insostituibile in assoluto. Da lì vengono quattro conclusioni che vincolano
questo piano, e ognuna smonta una riduzione che sembrava ovvia:

- `arrow-select` è il solo pin diretto non ereditato da alcun crate, ma resta
  nel grafo transitivo via Parquet: rimuovere la dichiarazione **non** riduce il
  binario;
- `libc` è dichiarata direttamente solo dal benchmark e resta transitiva nel
  prodotto: non è eliminabile dal binario;
- `jsonschema` non è solo test, `zip` non è solo duplicazione di `calamine`,
  `tempfile` non è solo fixture;
- `gdal` e la sua chiusura nativa sono condizionali al profilo FileGDB:
  rimuoverli rinuncia a quella capacità.

Gli archi normali del grafo includono le macro procedurali e non misurano i
simboli presenti nel binario. I lock nativi descrivono il **prefisso di
costruzione**: le librerie effettivamente spedite si leggono nei singoli SBOM,
non si deducono dall'intero lock Conda. Strumenti di CI, programmi di sistema e
dataset non sono librerie applicative e non si contano come tali.

## Sequenza di implementazione e verifiche

1. Piano e censimento sono nel docset. Il confronto di maturità del 15–16
   settembre aggiunge la baseline database e M1–M5, conservando le evidenze
   storiche e separando A1–A7 come proposte d'interoperabilità.
2. L1 e L2 sono chiuse rispettivamente in `7d3c461` e `d2d35a7`; procedere con
   la revisione mirata M1–M3. I residui della procedura non impongono di rinviare
   tutto il lavoro sul prodotto; M4 si coordina con R3–R5 senza costruire
   strumenti doppi.
3. Chiudere Q1, Q2b, Q2c e R1–R5 **prima della candidate**, quando servono gli
   strumenti di qualifica. Q2a e R6 sono chiuse. Nessuna campagna lunga per
   giustificare una revisione del piano o uno split di moduli.
4. Trattare il pin inutilizzato e coordinare duplicazioni, fork e proposte
   upstream; affrontare le catene più grandi dopo una decisione sul beneficio
   misurabile, come richiede M5.
5. Riconciliare contratti, deviazioni e compatibilità. Se viene scelto un
   percorso A, fissarne superfici e revisioni e chiuderlo con prove proprie:
   la qualità interna del codice non dimostra la composizione fra componenti.
6. Chiudere le voci di maturità con esito, prova o motivazione precisa;
   preparare una sola revisione definitiva, eseguire la qualifica prevista,
   produrre gli artefatti sullo stesso SHA e registrare il congelamento entro
   l'allowlist.

Il regime delle misure:

- letture e pianificazione: nessun livello 1 né 2;
- sola documentazione: controlli mirati del docset;
- codice, feature e gate: prove mirate durante lo sviluppo e alla chiusura del
  blocco, inclusi fmt/clippy sul perimetro pertinente; nessun checkpoint
  completo automatico a ogni commit o aggiornamento di dipendenza;
- rimisure solo per i perimetri invalidati, nominando quale misura cade e
  perché; nessuna campagna concorrente duplicata;
- checkpoint completi di livello 1 e 2, soak prolungati, campagne complete di
  copertura e qualifica degli artefatti **solo sulla candidate definitiva prima
  del rilascio**, con base differenziale esplicita, log duraturi e strumenti
  già pronti. Distribuzione e misure devono identificare la revisione provata;
- nessuna aggiunta facoltativa durante il congelamento. La pubblicazione della
  4.1.0 richiede un'autorizzazione propria.

## Riferimenti

- Repository: <https://github.com/PlenoraETL/plenora-IO-tools>.
- Contratti condivisi: <https://github.com/PlenoraETL/plenora-contracts>.
- Riferimento ingegneristico: [database-tools alla revisione confrontata][db-base].
  Analizzato da una copia separata, con fonti immutabili nella sezione iniziale;
  il precedente checkout locale arretrato non è stato aggiornato o modificato.
- Evidenze durature della 4.0.0: `C:\Users\marco\Desktop\IO-tools-evidenze-4.0.0\`,
  con la qualifica di `6beb410`, i confronti di bozza e release e l'input del
  finding Q2. Non sono materiale da eliminare nella pulizia.
- Resoconto della pulizia post rilascio:
  `.s9-checkpoint/pianificazione/PULIZIA-2026-09-15.md`, fuori dal controllo di
  versione come il resto di quella directory.

## Risultato atteso

La 4.1.0 conserva il comportamento pubblico della 4.0.0 e rende verificabili
gli interventi di maturità: responsabilità meno accoppiate dove necessario,
stati del modello non ambigui, guardie provate sul percorso reale e
corrispondenza fra dichiarazioni e controlli eseguiti. Porta inoltre residui
trattati, dipendenze aggiornate o motivate, duplicazioni ridotte dove possibile
e una procedura di qualifica meno costosa senza prove più deboli. Il risultato
non è una parità di funzionalità con database-tools né un punteggio globale
di qualità.

Il resoconto finale confronta versioni, feature, grafo per piattaforma e
profilo, dimensione e composizione degli artefatti quando misurate, tempo di
costruzione e di qualifica quando misurato, e delta residui dei fork. Non
presenta pacchetti non più nel lock come byte risparmiati, né un riuso di
evidenza come una nuova esecuzione.

Se sono realizzati interventi A, il resoconto separa compatibilità dei
contratti, prove offline e pipeline live qualificata, indicando esattamente
quali superfici, formati e provider sono stati esercitati. Le proposte non
scelte e i blocchi nel componente vicino restano espliciti.
