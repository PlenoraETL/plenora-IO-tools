# Piano 4.0.0 — adozione dei contratti pubblici

**Documento di pianificazione.** Non descrive ciò che il codice fa oggi — quello
lo dicono [docs/PRODUCT.md](PRODUCT.md) e [docs/ENGINEERING.md](ENGINEERING.md) —
ma lo **scostamento** fra ciò che il codice fa e ciò che
[`plenora-contracts`](https://github.com/PlenoraETL/plenora-contracts) richiede,
e l'ordine in cui chiuderlo.

Nessuna API, specifica condivisa o versione di prodotto è modificata da questo
documento. Tag, artefatti ed evidenze della 2.0.0 e della 3.0.0 restano intatti.

## Le revisioni su cui il piano è costruito

Un piano che non nomina le revisioni che ha letto invecchia senza che nessuno se
ne accorga. Queste sono le sei consultate.

| repository | revisione | ruolo |
|---|---|---|
| `plenora-IO-tools` | `29c78a79f325b9b1e2ce6daebe2798b34f474b0e` | `main` al momento dell'analisi |
| `plenora-IO-tools` | `28bf62ccba49d47bda0797b9892e373c49ebea61` | revisione qualificata della 3.0.0, da cui esce l'artefatto misurato |
| `plenora-contracts` | `73ba27dd456fab420d18b2a52013c7eea2040215` | `main`, riferimento normativo letto |
| `plenora-contracts` | `5d151078142e7ed49d831659ee8be92a4975f80b` | pin adottato da database-tools, precedente da imitare |
| `plenora-database-tools` | `c82bbfa3dfc432f2293c87b596cfa09d007b3839` | `main`, riferimento ingegneristico — commit «adopt public contracts for 4.0.0 (#84)» |
| `plenora-database-tools` | `2f35ca0c3a276c1c99b7562220f90b7a47fb3ae2` | copia locale, **indietro** rispetto al remoto: non usata, non aggiornata |

La copia locale di database-tools era indietro di undici commit e non è stata né
aggiornata né sovrascritta. L'analisi è stata fatta su due checkout separati e
temporanei dei remoti. Una copia locale che qualcuno sta usando non è materiale
di consultazione.

### Il comportamento attuale è misurato, non letto

La colonna «comportamento attuale» della matrice non viene dai sorgenti ma
dall'**artefatto pubblicato**: `plenora-io-3.0.0-linux-x86_64-base.tar.gz`,
riscaricato da GitHub Releases e interrogato come lo interrogherebbe un
consumatore. È il confine che `ADOPTION.md` richiede — «launch the released
binary for CLI checks» — e la differenza non è accademica: leggere il dispatcher
avrebbe detto quali comandi esistono, non che cosa risponde il binario che
qualcuno ha scaricato.

## Che cosa dicono i due riferimenti, e come si dividono il campo

`plenora-contracts` governa **i comportamenti pubblici**: identità delle
operazioni, scoperta, contratti di ingresso e uscita, assi d'errore, forma della
busta CLI, metadati Arrow scambiati. Il suo `GOVERNANCE.md` fissa il test di
confine: un requisito appartiene lì solo se un consumatore può osservarlo senza
ispezionare l'interno del componente. Esclude esplicitamente il layout dei
crate, i tratti Rust, gli algoritmi, i driver e la selezione delle dipendenze.

`plenora-database-tools` è il riferimento **ingegneristico**: non ha autorità su
di noi, ma ha già percorso questa strada e i suoi gate sono la forma concreta di
standard che qui non esistono ancora. Le sue regole in `AGENTS.md` sono policy —
una capability resta falsa finché non c'è una prova, i documenti non ripetono
fatti che vivono nel codice, un gate che nessuno esegue non è un gate.

Le due cose non si confondono: adottare un gate di database-tools è una scelta
nostra di ingegneria; adottare `plenora-cli-v2` è un obbligo verso chi ci
consuma.

---

## La matrice

Sei colonne: requisito, fonte, comportamento attuale, scostamento, intervento,
prova di accettazione. Le prove di accettazione sono scritte perché qualcuno
possa eseguirle senza chiedere che cosa intendessero.

### A — Adozione dei contratti comuni

| # | Requisito | Fonte | Comportamento attuale | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| A1 | Identificatore di componente `plenora-<domain>-tools` | SURF-001 | `release/cli-protocol-v2.json` dichiara `"component": "plenora-IO-tools"` | maiuscole non ammesse dalla forma | fissare `plenora-io-tools` come identificatore pubblico unico, in un solo punto del codice | il campo `component` di ogni busta JSON vale `plenora-io-tools`; una sonda confronta la costante con la forma `^plenora-[a-z]+-tools$` |
| A2 | Pin immutabile del repository dei contratti | ADOPTION §1.2 | assente | nessun pin: nulla lega il prodotto a una revisione dei contratti | `contracts/adoption-source.json` sul modello di database-tools, con `contracts_source.revision` a 40 esadecimali | il gate rifiuta un checkout dei contratti la cui `HEAD` non coincide col pin, come fa `check_public_contracts.py` di database-tools |
| A3 | Manifesto di adozione validato dallo schema v4 | ADOPTION §1.5, `adoption-manifest-v4.schema.json` | assente | nessuna dichiarazione di conformità pubblicabile | manifesto con `artifacts`, `contracts`, `deviations`; ogni artefatto porta `version` e `digest` `sha256:<64 hex>` | il manifesto valida contro lo schema v4 al pin; i digest coincidono con quelli congelati in `assurance/current-state.json` |
| A4 | Identità immutabile dell'artefatto | ADOPTION §2 | i sei digest sono già congelati nella candidate e verificati due volte | **nessuno** — la macchina di rilascio produce già esattamente ciò che il manifesto richiede | riusare i digest esistenti invece di ricalcolarli | i digest del manifesto sono gli stessi di `aperto.candidate_release.artefatti` |
| A5 | Prova black-box: il verificatore invoca un binario, non ispetta lo stato privato | ADOPTION §3 | i gate attuali girano nell'albero di build e leggono strutture interne | i test misurano il codice, non un artefatto invocato dall'esterno | un verificatore che riceve **il percorso di un binario** e lo interroga dal confine di processo, senza sapere da dove venga | il verificatore passa sul binario appena costruito in CI, e fallisce se una risposta viola un contratto |
| A5b | Identità dell'artefatto qualificato | ADOPTION §2 | i digest sono congelati e verificati, ma nessun controllo lega il **binario interrogato** a un digest | il verificatore da solo non dice quale artefatto ha interrogato | in **qualifica** il binario si estrae dall'archivio identificato dal digest, e il verificatore riceve quel percorso | l'estrazione ricalcola il digest dell'archivio e lo confronta con `aperto.candidate_release.artefatti` **prima** di invocare; un digest diverso ferma la qualifica |
| A6 | Deviazioni dichiarate con regola, osservabilità e tracking | ADOPTION §5 | assente | una conformità parziale non dichiarata si legge come completa | ogni riga di questa matrice non chiusa alla 4.0.0 diventa una `deviation` con il proprio identificatore | ogni deviazione cita un identificatore esistente (`SURF-001`, `CLI-2.4`, …) e dichiara `detectable_before_invocation` |

### B — API pubbliche: CLI

Questa è l'area con lo scostamento maggiore, ed è anche quella dove le misure
sono più nette. Le righe qui sotto vengono dalle risposte del binario 3.0.0.

| # | Requisito | Fonte | Comportamento attuale (misurato) | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| B1 | `--help` presente | CLI-2.0 §3 | `--help` esce **2** con `CLI_USAGE: uso: plenora-io <catalog\|inspect\|layers\|read\|convert>` | la superficie di scoperta richiesta non esiste: l'aiuto è un errore | implementare `--help` che descrive **solo** i comandi compilati nel binario | `--help` esce 0 e non nomina comandi assenti da quella build |
| B2 | `--version --format json` nella busta comune | CLI-2.0 §3 | `--version` rende `{"status":"ok","version":"3.0.0"}`; con `--format json` esce **2**, «non prende argomenti» | busta non conforme e flag rifiutato | `--format json` accettato ovunque; `result` porta `component_version` e la versione di protocollo CLI | la risposta valida contro `cli-envelope-v2.schema.json` e `result.component_version` vale la versione del workspace |
| B3 | `capabilities --format json` | CLI-2.0 §3, CAP-003…CAP-009 | il comando **non esiste** | manca la scoperta delle operazioni: nessun orchestratore può selezionarci | comando `capabilities` che descrive **il binario che risponde**, feature di compilazione comprese | la risposta valida contro `capabilities-v2.schema.json`; la build `base` e la build `filegdb` producono documenti **diversi** |
| B4 | Una sola versione di protocollo nell'artefatto | profilo io-tools, «cutover» | `catalog` rende `protocol_version: 2`; **ogni percorso d'errore misurato** — uso, `io`, `unsupported` — rende `protocol_version: 1` | v1 e v2 **coesistono** nello stesso binario, che il profilo vieta. Non è il solo errore d'uso: è tutta la superficie d'errore | portare ogni percorso alla busta v2; `--legacy-protocol-v1-unsafe` va rimosso o isolato dietro un artefatto separato | nessuna risposta del binario conforme porta `protocol_version: 1`, successo **ed errore** |
| B5 | Niente su stderr in modo JSON | CLI-2.0 §4 | gli errori escono **su stderr**, stdout vuoto | selezione dello stream non conforme; un consumatore che legge stdout non vede nulla | busta d'errore su **stdout**, un documento e una newline | per ogni comando, in fallimento: stdout è un documento JSON, stderr è vuoto, exit ≠ 0 |
| B6 | Campi d'identità nella busta | CLI-2.0 §5 | busta di successo: `contract`, `determinism`, `drivers`, `protocol_version`, `status` | mancano `component`, `component_version`, `command` | aggiungerli a ogni busta, successo ed errore | ogni busta valida contro `cli-envelope-v2.schema.json`, che li richiede |
| B7 | Dati dell'operazione dentro `result` | CLI-2.0 §5 | `drivers` e `determinism` stanno **al primo livello** | campi di primo livello aggiuntivi, vietati | spostare il corpo dentro `result` senza cambiarne la forma interna | le chiavi di primo livello sono esattamente quelle della busta; tutto il resto è sotto `result` |
| B8 | Identificatore di contratto del catalogo | catalogo io-tools | `contract: plenora-io-catalog-v2` | il catalogo comune fissa `plenora-io-catalog-v1` per `io.catalog@1` | allineare l'identificatore, oppure dichiarare la deviazione e la ragione | l'identificatore emesso coincide con quello del catalogo comune al pin |
| B9 | `--format json` come selettore esplicito | CLI-2.0 §2 | `catalog --format json` esce **2**: il flag non è accettato | la modalità macchina non è selezionabile come il contratto prescrive | `--format json` su tutti i comandi; formato umano esplicito e mai implicito | ogni entrypoint del binding CLI è invocabile **letteralmente** come scritto in `bindings/cli-v1.json` |
| B10 | Comando `write` | catalogo io-tools, `io.write` | il binario espone `catalog, inspect, layers, read, convert` | `io.write` è **richiesto** e non esiste come comando | esporre `write --input INPUT.arrow --output SINK --format json` | l'entrypoint del binding risponde e dichiara esito di pubblicazione e fedeltà |
| B12 | `io.read` **consegna** dati Arrow | catalogo io-tools, ARROW-011, binding `read SOURCE --output OUTPUT.arrow` | `cmd_read` apre il reader, **drena i batch contandoli e li scarta**, e rende `rows_read`, `batches`, `truncated`, `fidelity`, `layer`. Nessun `--output`, nessun byte Arrow prodotto | non è una consegna incompleta: è un'operazione **di conteggio**. Il catalogo dichiara per `io.read` i content type `application/vnd.apache.arrow.stream` e `.file` e l'interchange `plenora-arrow-interchange-v1`; oggi non ne esce nessuno | migrare `read` da validatore a operazione di consegna: `--output`, scrittura Arrow IPC, e il risultato JSON che descrive ciò che è stato consegnato | un consumatore legge il file Arrow prodotto senza il nostro codice, e ne ritrova schema, righe e metadati; la busta JSON dichiara il content type effettivamente prodotto |
| B13 | Streaming senza materializzazione completa | ARROW-011 | il lettore è già a batch e lo spool è documentato in ENGINEERING | da verificare **al confine**, non nell'implementazione | dichiarare nel descrittore se la materializzazione è limitata, e provarlo | il consumatore elabora il primo batch prima che l'ultimo sia stato prodotto, oppure il descrittore dichiara la materializzazione limitata |
| B11 | Proiezione dei codici d'uscita | CLI-2.0 §8 | la proiezione è agganciata a `IoErrorCode`, **non** alla categoria del contratto: vedi la tabella qui sotto | scostamento **incompatibile**: solo `cancelled → 130` coincide | riscrivere la proiezione sulla `category`, che è l'asse autoritativo | una prova tabellare copre ogni categoria e il suo codice atteso, e fallisce se la chiave torna a essere il codice interno |

### B-bis — I contratti di confine di proprietà nostra

Il profilo non chiede solo che le sei operazioni esistano: chiede che IO-tools
**pubblichi** gli schemi delle dodici forme di ingresso e uscita, con esempi di
conformità, prima che un artefatto reclami il profilo. Sono schemi nostri — il
repository comune ne fissa gli identificatori pubblici e il significato
incrociato, non l'implementazione. Mancava ogni riga operativa.

| # | Requisito | Fonte | Comportamento attuale | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| BB1 | Dodici schemi immutabili per le sei coppie | profilo io-tools, «Component-owned wire contracts» | nessuno dei dodici identificatori esiste come schema pubblicato; il CLI emette `plenora-io-catalog-v2` senza uno schema che lo definisca | dodici schemi da scrivere: `…-catalog-query-v1`/`…-catalog-v1`, `…-inspect-input-v1`/`…-inspect-v1`, `…-layers-input-v1`/`…-layers-v1`, `…-read-input-v1`/`…-read-result-v1`, `…-write-input-v1`/`…-write-result-v1`, `…-convert-input-v1`/`…-convert-v1` | pubblicarli sotto `contracts/`, versionati e immutabili | ogni busta emessa dal binario valida contro lo schema del proprio `contract`; uno schema modificato in modo che cambi la validazione richiede un identificatore nuovo |
| BB2 | Esempi di conformità validi e invalidi | profilo io-tools | assenti | senza esempi, uno schema è una dichiarazione che nessuno prova | un esempio valido e uno invalido per ciascuna delle dodici forme | il gate valida i «validi» e **rifiuta** gli «invalidi»; un invalido che passa è rosso |
| BB3 | `plenora-io-error-details-v1` | profilo io-tools, ERR-013 | `details` non ha una forma dichiarata; il vocabolario delle perdite esiste ma non come schema di `details` | quando un errore IO porta `details`, il valore **deve** conformarsi a uno schema nostro pubblicato | definire lo schema e i suoi esempi limitati; omettere `details` resta valido quando i quattro assi bastano | un errore con `details` valida contro lo schema; un `details` che viola i limiti di ERR-012 è rifiutato dal produttore, non solo dal validatore |
| BB4 | Limiti semantici di ERR-011 e ERR-012 | ERRORS-1.0 §6 | il v2 ha un proprio sistema di budget molto più dettagliato — 64 KiB totali, 12 KiB per sezione, tetti per voce | i due sistemi non sono confrontati: i nostri limiti potrebbero essere più stretti o più larghi di `524 288` byte per l'errore e `262 144` per `details`, e di profondità 8 / 128 proprietà / 2 048 nodi | confrontare i due insiemi di limiti e dichiarare quale governa | una prova costruisce il caso peggiore dichiarato e misura i byte JSON effettivi contro **entrambi** i tetti |
| BB5 | Verifica dei metadati Arrow al confine | ARROW-001…ARROW-012 | il vocabolario è implementato per intero nel codice; nessuna prova lo verifica **sui byte prodotti da un'invocazione pubblica** | l'implementazione non è la prova: ARROW-001 (versione di contratto nello schema), ARROW-003/004 (identità dei campi preservata), ARROW-006 (metadati non contraddittori), ARROW-007 (CRS risolto, dichiarato-non-risolto e assente distinti) vanno letti dall'artefatto Arrow consegnato | una prova che legge il file prodotto da `io.read` e verifica ciascun identificatore | la prova fallisce se `plenora.contract.version` manca, se un `plenora.field_id` cambia in un round-trip che non lo doveva cambiare, o se un CRS non risolto viene presentato come risolto |

Le righe BB1–BB3 hanno una dipendenza che vale la pena dire: gli schemi si
scrivono **dopo** aver deciso la forma della busta (B6, B7) e **prima** di
dichiarare l'adozione (A3). Scriverli ora significherebbe versionare una forma
che sta per cambiare.

#### La proiezione dei codici d'uscita, misurata

Avevo scritto che questa riga era conforme. Lo era su un campione di uno:
l'errore d'uso esce **2**, e `invalid_configuration` proietta a 2 anche nel
contratto. Provando percorsi d'errore veri, la coincidenza sparisce.

Misurato sul binario 3.0.0 pubblicato:

| invocazione | exit | `category` emessa | exit richiesto da CLI-2.0 §8 |
|---|---:|---|---:|
| `inspect /nessun/file/qui.shp` | 1 | `io` | **5** |
| `inspect README.md` | 4 | `unsupported` | **3** |
| `convert /nessun/file.shp …` | 1 | `io` | **5** |
| `layers /nessun/file.gpkg` | 1 | `io` | **5** |

La causa sta in `crates/plenora-io-cli/src/main.rs:145`: la proiezione è un
`match` su `IoErrorCode` — il codice **interno** — e non sulla `category`, che
è l'asse dichiarato autoritativo dal contratto. I codici prodotti sono
`1, 2, 3, 4, 5, 6, 7, 8, 130`; il contratto ammette `0, 2, 3, 4, 5, 6, 70, 130`.
Il **7** e l'**8** non esistono nel contratto, e **70** non è mai prodotto.

Solo `cancelled → 130` coincide, ed è l'unico ramo già agganciato alla categoria
invece che al codice. Non è una coincidenza fortunata: è la prova che la forma
giusta era già nota e applicata in un punto solo.

#### Il repository sa già che questo lavoro manca

`docs/contracts/handoff-plenora-error.json` e la matrice `plenora-io-handoff-v1`
generata dal CLI dichiarano la destinazione `plenora-error-v1` con
`"stato": "mappatura preparata, conformita' NON dichiarata"` e la nota
«l'adozione e' uno step breaking separato dopo S9, insieme a CLI v2, exit code
e capabilities».

Non è una scoperta di questo piano: è una decisione già presa, registrata e
nominata, di cui questo piano fissa la scadenza. Dirlo cambia la natura del
lavoro — non si rimedia a una svista, si esegue uno step che era stato
deliberatamente rinviato.

Quel documento porta anche una domanda aperta, che diventa **D8**.

### C — API pubbliche: Rust

| # | Requisito | Fonte | Comportamento attuale | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| C1 | Superficie Rust **richiesta** dal profilo | profilo io-tools, «Public surfaces» | `release/cli-protocol-v2.json` dichiara `rust_api.status: internal_unstable`, `semver_guarantee: false`: nessun export è documentato come pubblico e nessuna compatibilità è promessa | manca ciò che il profilo chiede — export documentati, regole di compatibilità, prova da un consumatore esterno | dichiarare la superficie: elenco degli export, contratto di compatibilità, mappatura verso le operazioni | un crate consumatore importa **solo** gli export documentati e compila; il test è rosso se uno di essi diventa privato |
| C1b | Il canale di distribuzione è una scelta separata | SURFACE-BINDINGS §2, ADOPTION §2 | tutti e sedici i crate hanno `publish = false` | `publish = false` riguarda **crates.io**, non l'esistenza di una superficie pubblica: un workspace sorgente è già consumabile per path o per dipendenza git | scegliere il canale — crates.io, sorgente versionato, dipendenza git su tag — **dopo** aver deciso la superficie, non insieme | il manifesto di adozione identifica l'artefatto crate per versione e digest, qualunque sia il canale |
| C2 | Mappatura operazione → export pubblico | SURFACE-BINDINGS §2 | assente | nessuno può sapere quale simbolo serve `io.read` | mappatura versionata, sul modello di `rust_surface_bindings()` di database-tools | un crate consumatore importa **solo** gli export documentati e compila |
| C3 | Verifica da un consumatore esterno | SURFACE-BINDINGS §2 | i test vivono dentro i crate | un test interno non prova che l'API sia usabile da fuori | test di integrazione che importa il crate come dipendenza | il test fallisce se un export documentato diventa privato |
| C4 | Equivalenza fra superfici | SURF-017, CLI-2.0 §10 | non verificabile: una delle due superfici non esiste ancora | — | verifica di equivalenza fra CLI e Rust sulla stessa operazione | stessi input rifiutati, stessi assi d'errore, stesso significato del risultato |

### D — API pubbliche: Python

| # | Requisito | Fonte | Comportamento attuale | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| D1 | Lo SDK Python **non** è richiesto dal profilo | profilo io-tools, catalogo `python_sdk: not_applicable` | `sdk/python` esiste, distribuzione `plenora-io`, import `plenora_io`, `Private :: Do Not Upload`, Python 3.11–3.13 | **nessuno**: spedirlo è legittimo e non crea obblighi di conformità | dichiararlo `not_applicable` nel manifesto e non promettere conformità per esso | il manifesto non elenca `plenora-python-sdk-v1` fra i contratti adottati |
| D2 | Se lo SDK rimane, resta coerente col CLI | SURF-009 | lo SDK avvolge il processo CLI | rischio derivato: cambiando la busta CLI, lo SDK si rompe in silenzio | far dipendere lo SDK dalla busta v2 e dai campi d'identità | i test dello SDK falliscono se la busta perde `component_version` |

### E — Standard di commenti e documentazione

| # | Requisito | Fonte | Comportamento attuale | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| E1 | Niente debito anonimo nei commenti | `check_comments.py` di database-tools | **zero** occorrenze di `TODO/FIXME/HACK/XXX` in `crates`, `scripts`, `sdk` | nessuno nei fatti; manca la **guardia** | portare `check_comments.py`, adattando `SKIP_PARTS` a `vendor/` | il gate è verde al primo colpo e rosso su un `TODO` introdotto apposta |
| E2 | Niente cronaca del processo nei commenti | `check_comments.py` | **167 violazioni su 274 file**, da 13 marcatori distinti: vedi la ripartizione qui sotto | adottare il gate così com'è significa 167 riscritture, molte delle quali toglierebbero prosa che spiega **perché** | decidere marcatore per marcatore — D6 | il gate gira su tutto l'albero; ogni violazione o è corretta o il marcatore è escluso con la ragione scritta |
| E3 | I documenti non ripetono fatti che vivono nel codice | `AGENTS.md` di database-tools, regola 3 | parzialmente adottato: il blocco di stato di `docs/RELEASE.md` è **generato** e un gate lo verifica | il resto della prosa non è generato | estendere la generazione dove il fatto esiste già strutturato | rigenerare non produce differenze, come già per `docs/RELEASE.md` |
| E4 | Docset minimo ed esatto | `check_docset.py` (nostro) | cinque canonici più sette operativi, allowlist esatta | **nessuno**: la regola esiste ed è più severa di quella di database-tools | ammettere questo documento e collegarlo, senza toccare gli altri controlli | `check_docset.py` verde con `docs/PIANO-4.0.0.md` in `CANONICI` e linkato da `README.md` |
| E6 | Un cambiamento editoriale non deve pretendere una rimisura completa | osservato scrivendo questo piano | aggiungere un documento canonico cambia `docset.markdown_canonici` in `assurance/current-state.json`, e le sonde del contratto di release diventano rosse — dentro L1 | un documento in più costa una corsa di checkpoint. Il conteggio è **derivato** dall'allowlist e **duplicato** nello stato: due posti per lo stesso fatto | **derivare** il conteggio dall'allowlist invece di copiarlo nello stato. Togliere la sonda da L1 non elimina la duplicazione: la nasconde, e i due valori tornerebbero a divergere senza che nessuno lo veda | `docset.markdown_canonici` non esiste più come valore scritto: si legge da `CANONICI`. Aggiungere un documento non tocca `assurance/current-state.json`, e i controlli del docset bastano |
| E5 | Un `AGENTS.md` che dica ciò che non è negoziabile | `AGENTS.md` di database-tools | assente | i vincoli vivono sparsi fra `README.md`, i gate e i messaggi di commit | valutarne uno nostro, **non** copiato: le regole devono essere le nostre | ogni regola scritta è citabile e almeno un gate la presidia |

#### I marcatori di «cronaca obsoleta», misurati

I tredici marcatori di `check_comments.py` di database-tools, applicati ai 274
file commentabili di IO-tools — `vendor/`, `docs/` e `target/` esclusi:

| occorrenze | marcatore | un esempio nostro |
|---:|---|---|
| 102 | `prima (stesura\|versione\|implementazione\|esecuzione\|campagna)` | «la prima stesura di questo passo la trattava come un difetto» |
| 26 | `tranche` | «registrata nella CIA della tranche 5» |
| 14 | `(versione\|forma\|stesura\|commento\|contratto) precedente` | «La stesura precedente usciva subito su una pagina non a dizionario» |
| 6 | `da allora` | «Da allora la forma sciolta è diventata un opt-in esplicito» |
| 3 | `era stat[oa] (aggiunt[oa]\|rimoss[oa])` | «nemmeno dopo che il file era stato rimosso» |
| 3 | `qui c'era` | «Qui c'era un `continue`, e sopra il perché» |
| 3 | `nello stesso commit` | — |
| 3 | `era rimast` | — |
| 2 | `prima era` | «prima era rifiutato con "nome di elemento XML non valido"» |
| 2 | `roadmap` | entrambe **dentro i gate**, che citano documenti eliminati |
| 1 ciascuno | `pre-fix`, `diceva il contrario`, `… diceva` | — |

Il debito anonimo è **zero**: nessun `TODO`, `FIXME`, `HACK` o `XXX` in 274
file. Quella metà del gate passerebbe oggi senza toccare niente.

L'altra metà no, e il numero da solo non dice se sia un difetto. Due terzi delle
occorrenze vengono da un marcatore solo, e molte di quelle frasi spiegano
**perché** il codice è come è: «la prima stesura la trattava come un difetto, e
sbagliava» è la motivazione di una scelta, non la cronaca di un commit.
Adottare il gate senza distinguere cancellerebbe proprio il contenuto che
`check_comments.py` dichiara di voler preservare — «Motivazioni, invarianti,
limiti e compatibilita correnti restano invece contenuto utile».

La decisione è **D6**, e ora ha un numero sotto.

### F — Separazione dei test e modularità

| # | Requisito | Fonte | Comportamento attuale (misurato) | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| F1 | Nessun modulo `cfg(test)` inline nei sorgenti di prodotto | `check_test_layout.py` di database-tools | **44 moduli inline** su 55 sorgenti Rust; un solo modulo esterno, un solo file dedicato | lo scostamento più grande per numero di file toccati | spostare ciascun modulo in un file dedicato; restano moduli figli, quindi vedono i privati e non allargano l'API | `check_test_layout.py` verde; `cargo test` esegue lo **stesso** numero di test di prima |
| F2 | Misura del solo codice di prodotto | `code_size.py` di database-tools | non misurato | senza denominatore, «ridurre» non è verificabile | portare `code_size.py` e fissare un tetto | misurate oggi: **47 930** righe di prodotto e **36 730** di test dentro i file di prodotto, cioè il **43,4 %** |
| F3 | Il tetto è un budget, non una fotografia | `code_size_budget.json` | assente | un tetto fissato sul valore corrente non impedisce nulla | budget esplicito con la ragione del numero | il gate è rosso se il prodotto cresce oltre il budget senza che il budget sia stato cambiato in un commit visibile |
| F4 | Modularità: il confine fra `plenora-io-model` e il resto | `release/cli-protocol-v2.json`, nota R15.4.1 | il protocollo dichiara già l'estrazione dei tipi di confine da `plenora-io-model` come prevista | intento dichiarato, non eseguito | eseguire l'estrazione **prima** di esporre la superficie Rust: è ciò che rende esponibile un confine stretto | i tipi pubblici della superficie Rust vengono da `plenora-io-model`, non dai driver |

### G — Riduzione delle dipendenze

| # | Requisito | Fonte | Comportamento attuale | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| G1 | Il grafo compilato è l'autorità, non il lockfile | memoria di progetto, `cargo tree` | **266** nel lock; **173** compilati (`plenora-io-cli`, feature predefinite, `--edges normal`), **175** con `gdal-backend`, **186** includendo le build-dependencies | il lock sovrastima di **93 pacchetti**: il 35 % di ciò che dichiara non entra in nessun artefatto spedito | partire da 173, non da 266: ridurre il lock non riduce ciò che spediamo | il censimento nomina feature e target; due feature diverse danno due numeri diversi |
| G2 | Advisory e licenze presidiate da un gate che gira | `AGENTS.md` di database-tools, regola 6 | **presidiate**: `cargo audit --deny warnings` gira in CI (`ci.yml:314`) con una lista di deroghe governata da `audit_ignores.py` e dal suo test; `check-licenze-artefatto.py` verifica le licenze **per artefatto** nella distribuzione, Linux e Windows, con referto | nessuno sugli advisory. L'assenza di `deny.toml` non è una lacuna: è un altro strumento per un lavoro in parte già fatto | identificare che cosa `cargo deny` aggiungerebbe che oggi manca — ad esempio una allowlist di licenze a livello di **dipendenza** e non di artefatto, un divieto su crate nominati, una politica sulle versioni duplicate, una allowlist delle sorgenti | se un controllo aggiuntivo serve, entra in un workflow che gira; se non serve, la riga si chiude dichiarando che il presidio esiste già |
| G3 | Ridurre senza toccare ciò che è qualificato | questo piano | i sei artefatti della 3.0.0 sono congelati | rimuovere una dipendenza cambia i byte: è lavoro da 4.0.0, mai retroattivo | verifiche **proporzionate**: test mirati durante il lavoro, L1 alla chiusura dell'intervento, L2 solo quando si qualifica una candidate | nessuna riduzione tocca `assurance/evidence/` né i digest della 2.0.0 e della 3.0.0; il costo di verifica è scelto in base a ciò che l'intervento tocca, non per abitudine |

### H — Seguito dei fork

| # | Requisito | Fonte | Comportamento attuale | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| H1 | Ogni fork dichiara i propri delta | `scripts/*-fork-lock.json` | `dxf 0.6.1` (59 file, 4 delta funzionali), `gdal 0.19.0` (63 file, 3), `shapefile 0.9.0` (29 file, 3) | **nessuno**: i lock sono già la forma giusta | mantenerli | il digest dell'albero versionato coincide col lock |
| H2 | Sapere se upstream ha assorbito un delta | pratica già in uso (`1d6454b`: «tre delta avanti, uno che upstream ha assorbito») | i tre fork sono **allineati** all'upstream corrente — `dxf 0.6.1`, `gdal 0.19.0`, `shapefile 0.9.0`, verificato su crates.io | nessuna deriva oggi. Manca il **controllo**: l'allineamento è il frutto di una revisione manuale, e la prossima potrebbe non esserci | un **monitoraggio periodico** che confronti la versione del lock con quella pubblicata upstream e riporti la distanza | il monitoraggio nomina versione locale e versione upstream; una nuova versione upstream produce una **segnalazione**, non una CI rossa. Una pubblicazione di terzi non è un difetto nostro, e rendere rossa la CI ordinaria per un evento che non controlliamo trasforma un avviso in un blocco |
| H3 | Un delta che upstream ha assorbito si ritira | pratica già in uso | applicata alla 0.9.0 di `shapefile` | nessuno oggi; il rischio è dimenticarlo al prossimo giro | ogni aggiornamento di fork rivede i delta uno per uno | il messaggio di commit dice per ciascun delta se resta, e perché |

### H-bis — Ciò che va cambiato nel repository comune

Una riga sola, e non è nostra da applicare: vive in `plenora-contracts` e segue
il processo di modifica dei contratti, non il nostro ciclo di rilascio.

| # | Requisito | Fonte | Comportamento attuale | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| HB1 | Il profilo nomina la prima release conforme | `profiles/io-tools.md`, «First conforming release and cutover» | il profilo dice «The first release claiming this profile is IO-tools `2.0.0` or a later `2.x` release» e che «The existing `1.x` line remains historical» | la 2.0.0 e la 3.0.0 sono uscite **senza** reclamare il profilo, e la linea storica non è più la 1.x. Il riferimento è legato a un cutover che non è avvenuto | proporre a `plenora-contracts` l'aggiornamento del riferimento alla **4.0.0**, con la dichiarazione che le linee 2.x e 3.x restano storiche e non reclamano il profilo | il profilo aggiornato nomina la release che reclama davvero il profilo; il nostro pin punta a una revisione che contiene quella correzione |

Va proposta **prima** di fissare il pin (A2): fissare una revisione che dice la
cosa sbagliata, e correggerla dopo, significherebbe cambiare pin a metà
adozione. È anche la ragione per cui questa riga non appartiene alla priorità 1
ma la precede.

### I — Qualifica finale

| # | Requisito | Fonte | Comportamento attuale | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| I1 | Il gate del contratto di release copre i nuovi obblighi | `scripts/check_release_contract.py` | 37 invarianti, 35 verificati, 0 bloccanti, 2 differiti | i requisiti di questo piano non sono invarianti: nessuno li presidia | un invariante `contratti.profilo-pubblico` bloccante finché l'adozione non è verificata | il gate è **rosso** finché il black-box del profilo non passa |
| I2 | Il black-box gira in CI su ogni push | `AGENTS.md` di database-tools, regola 6; il loro job `profilo pubblico v4` | assente | un gate che nessuno esegue non è un gate | job CI che fa il checkout immutabile dei contratti al pin e lancia il verificatore sul binario costruito | il job è rosso se il pin e il checkout divergono |
| I3 | La qualifica cross-component resta differita o si chiude | invariante `sistema.qualifica-cross-component` | differita e **non verificata** per decisione del titolare | la catena IO → data → database non è provata in nessuna direzione | l'adozione dei contratti comuni è la **precondizione**, non la prova | quando `release/system-rc-gate.json` passa a `satisfied`, l'invariante torna verificato da sé |
| I4 | La 4.0.0 è una major, e lo è per ragioni dichiarate | COMPATIBILITY, «Incompatible changes» | la 3.0.0 è pubblicata e intatta | — | ogni riga incompatibile di questa matrice è elencata nelle note di rilascio | le note nominano B4, B5, B7 e ogni altra rottura osservabile |
| I5 | Le evidenze precedenti restano leggibili | `verifica_release_storiche` | 2.0.0 in archivio, 3.0.0 in `aperto.candidate_release` come `pubblicata` | — | non toccare nulla di ciò che è registrato | i digest, i tag e le evidenze della 2.0.0 e della 3.0.0 sono identici a prima |

---

## Priorità

L'ordine non è l'importanza: è la **dipendenza**. Una riga che ne abilita altre
viene prima anche quando è piccola.

**Priorità 0 — la correzione nel repository comune.** HB1. Il profilo dice
ancora che la prima release conforme è una `2.x`. Va proposta prima di fissare
il pin, altrimenti si fissa una revisione che dice la cosa sbagliata e la si
cambia a metà adozione. Non dipende da noi quanto le altre: segue il processo di
modifica dei contratti.

**Priorità 1 — il pin e la misura.** A2, A3, A5, I2. Senza un pin e un
verificatore black-box che gira, ogni riga successiva è un'opinione. Sono anche
le righe più economiche, e rendono misurabili tutte le altre.

A5 e A5b si fanno insieme ma restano distinte: il verificatore riceve **un
percorso** e interroga ciò che trova, senza giudicarne la provenienza; è
l'estrazione a legare quel percorso a un digest, e solo in qualifica. In CI si
interroga il binario appena costruito, perché lì la domanda è «il codice di
questo commit rispetta i contratti»; in qualifica si interroga quello estratto
dall'archivio, perché lì la domanda è «l'artefatto che spediamo li rispetta».
Due domande, due fasi, un verificatore solo.

**Priorità 2 — la busta CLI e i codici d'uscita.** B1, B2, B5, B6, B7, B9,
B11 e la rimozione della coesistenza B4. Sono la rottura incompatibile che giustifica la major, ed è
meglio farle **insieme**: ciascuna cambia la stessa struttura, e distribuirle su
più cicli significherebbe romperla più volte. B11 è entrata qui dopo la
misura: la proiezione dei codici è agganciata al codice interno invece che
alla categoria, e cambiarla è incompatibile quanto cambiare la busta.

**Priorità 3 — la scoperta.** B3 e A1. Il documento capability è ciò che rende
selezionabile il componente da un orchestratore; l'identificatore va corretto
prima, perché il documento lo contiene.

**Priorità 4 — la separazione dei test.** F1 e F2. Quarantaquattro moduli da
spostare sono lavoro meccanico e voluminoso: conviene farlo quando la superficie
pubblica è ferma, non mentre cambia. F4, l'estrazione dei tipi di confine, viene
qui perché è la precondizione di C1.

**Priorità 5 — la superficie Rust.** C1, C2, C3, C4, dopo la decisione D1. È il
punto con più incertezza e più costo, e non blocca nulla di ciò che precede.

**Priorità 6 — le due operazioni sui dati.** B10 e B12. `io.write` manca del
tutto; `io.read` esiste ma **non consegna**: conta i batch e li scarta. Sono la
stessa famiglia di lavoro — muovere dati Arrow attraverso il confine pubblico —
e la seconda è più grande della prima, perché trasforma un validatore in
un'operazione di consegna. B13 segue da B12. Ha senso solo dopo che la busta e
la scoperta sono stabili.

**Priorità 7 — i contratti di confine.** BB1–BB5. Dodici schemi, i loro esempi,
lo schema di `details` e le prove sui metadati Arrow consegnati. Vengono **dopo**
la busta e le due operazioni sui dati, e **prima** di dichiarare l'adozione:
scriverli mentre la forma cambia significherebbe versionare qualcosa che sta per
cambiare, e dichiarare l'adozione senza di essi è ciò che il profilo vieta.

**Priorità 8 — standard, dipendenze, fork.** E1–E6, G1–G3, H2, H3. Migliorano il
repository senza essere precondizioni di nessuna riga di conformità. G2 potrebbe
chiudersi senza lavoro: il presidio su advisory e licenze esiste già, e resta
solo da dire se manca qualcosa.

**Priorità 9 — la qualifica.** I1, I4. Si chiudono per ultime perché misurano
tutto il resto.

---

## Le decisioni ancora necessarie

Sono quelle che non posso prendere leggendo il codice, e su cui il piano si
ferma.

**D1 — esponiamo una superficie Rust pubblica?** Il profilo la richiede. Oggi
il protocollo dichiara l'API Rust `internal_unstable`, senza export documentati
né promessa di compatibilità. Le strade sono due: dichiarare la superficie —
elenco degli export, regole di compatibilità, mappatura verso le operazioni,
prova da un consumatore esterno — oppure dichiarare una deviazione su `SURF-016`
e vivere con una conformità parziale, che è onesta ma limita chi può comporci.

**D1b — su quale canale la distribuiamo?** È una decisione **separata**, e
tenerle unite è un errore che avevo fatto: `publish = false` riguarda crates.io,
non l'esistenza di una superficie pubblica. Un workspace sorgente versionato è
già consumabile — per path, per dipendenza git su tag, o per un archivio con
digest — e il manifesto di adozione chiede di identificare l'artefatto crate per
versione e digest, non di trovarlo su un registro. Si può quindi esporre una
superficie Rust pubblica e stabile restando `publish = false`. Il canale si
sceglie dopo, e non condiziona D1.

**D2 — che cosa succede al protocollo v1.** Il profilo vieta la coesistenza. Il
flag `--legacy-protocol-v1-unsafe` esiste per non rompere chi lo usa. Le scelte
sono rimuoverlo alla 4.0.0, oppure spedire un artefatto separato che lo
conserva. Serve sapere se qualcuno lo sta usando.

**D3 — la portata di `io.write`.** Il catalogo comune la vuole `required` e la
dichiara con effetto `local`, controlli di deadline e cancellazione, e ingresso
Arrow. Oggi il prodotto sa scrivere — `convert` lo fa — ma non espone la
scrittura come operazione autonoma su ingresso Arrow. Va deciso se la 4.0.0
espone `io.write` completa o la dichiara deviante.

**D4 — la superficie runtime.** Il profilo la dice richiesta «per ogni
operazione di I/O selezionata per l'orchestrazione». Oggi non esiste alcun
binding runtime. Se nessuna operazione è selezionata per l'orchestrazione, il
requisito non si applica e va dichiarato `not_applicable`; se qualcuna lo è,
è un blocco di lavoro che questo piano non dimensiona.

**D5 — quale revisione dei contratti fissare.** Database-tools ha fissato
`5d15107`; il `main` dei contratti è ora `73ba27d`. Fissare la stessa revisione
del componente vicino rende confrontabili le due adozioni; fissare `main` prende
le correzioni successive. Va scelta una sola revisione e scritta.

**D6 — come chiudere le 167 occorrenze di cronaca.** La misura c'è; resta la
scelta del metodo. Non è fra «cancellare il perché» ed «escludere il marcatore»:
c'è una terza strada, ed è la migliore. La motivazione si **riscrive al
presente**, e perde la cronaca senza perdere il contenuto — «la prima stesura la
trattava come un difetto, e sbagliava» diventa «non è un difetto: è il
comportamento dichiarato del formato». Il perché resta, il riferimento al
momento in cui qualcuno ha sbagliato se ne va, e la frase smette di invecchiare.

Resta da decidere solo il **perimetro**: riscrivere tutte e 167 in un intervento
solo, o marcatore per marcatore lungo il ciclo. Due dei tredici marcatori
colpiscono i gate stessi, che citano documenti eliminati, e quelli si chiudono
da sé quando la cronaca viene tolta.

**D8 — un driver di formato è un `provider`?** La domanda è già registrata in
`docs/contracts/handoff-plenora-error.json` con identificatore
`driver-e-un-provider`. Il campo `provider` di `plenora-error-v1` suggerisce un
servizio o un backend remoto; per noi è il formato del file — csv, geoparquet,
shapefile — scelto dal chiamante e senza effetto remoto. Se la destinazione
intende `provider` nel primo senso, il valore appartiene a un `details`
component-owned come `format_id`. Il documento dice che il DTO è l'unico punto
che dovrà cambiare quando la risposta arriva.

**D7 — dove fissare il tetto di dimensione.** F3 chiede un numero, e la scelta
non è fra «un vincolo» e «nessun vincolo»: un tetto fissato al valore corrente —
47 930 righe di prodotto — **impedisce la crescita**, ed è già un vincolo utile
anche se non obbliga a ridurre. Fissarlo più in basso impegna anche a ridurre, e
va scelto sapendo che l'intervento sulla busta CLI e su `io.read` aggiungerà
codice prima di toglierne. Il numero è una decisione, non una misura.

---

## Che cosa questo piano non fa

Non modifica API, specifiche condivise o la versione del prodotto. Non crea
documenti paralleli: l'unica estensione al docset è l'ammissione di questo file
e il collegamento da `README.md`, e ogni altro controllo resta com'era. Non
tocca tag, artefatti o evidenze della 2.0.0 e della 3.0.0, che restano
verificabili come lo erano il giorno in cui sono stati pubblicati.
