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
| A5 | Prova black-box, non test interni | ADOPTION §3 | i gate attuali girano nell'albero di build | i test misurano il codice, non l'artefatto | un gate che lancia il **binario estratto dall'archivio pubblicato** | il gate fallisce se gli si passa il binario di `target/debug` invece di quello dell'archivio |
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
| B11 | Proiezione dei codici d'uscita | CLI-2.0 §8 | la proiezione è agganciata a `IoErrorCode`, **non** alla categoria del contratto: vedi la tabella qui sotto | scostamento **incompatibile**: solo `cancelled → 130` coincide | riscrivere la proiezione sulla `category`, che è l'asse autoritativo | una prova tabellare copre ogni categoria e il suo codice atteso, e fallisce se la chiave torna a essere il codice interno |

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
| C1 | Superficie Rust **richiesta** dal profilo | profilo io-tools, «Public surfaces» | tutti e sedici i crate hanno `publish = false`; `release/cli-protocol-v2.json` dichiara `rust_api.status: internal_unstable`, `semver_guarantee: false` | il profilo richiede una superficie Rust pubblica; oggi è dichiarata interna e instabile | decidere fra due strade — vedi «Decisioni necessarie», D1 | l'esito della decisione è scritto nel manifesto: superficie esposta, oppure deviazione dichiarata su `SURF-016` |
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
| E6 | Un cambiamento editoriale non deve pretendere una rimisura completa | osservato scrivendo questo piano | aggiungere un documento canonico cambia `docset.markdown_canonici` in `assurance/current-state.json`, e le sonde del contratto di release diventano rosse — dentro L1 | un documento in più costa una corsa di checkpoint. Il conteggio è **derivato** dall'allowlist e **duplicato** nello stato: due posti per lo stesso fatto | valutare se il conteggio vada **letto** dall'allowlist invece che copiato, oppure se la sonda che li confronta debba vivere fuori da L1 | una modifica di solo Markdown richiede i controlli del docset e nient'altro; una modifica di codice continua a richiedere L1 |
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
| G2 | Un `cargo deny` che gira | `deny.toml` di database-tools | `scripts/check_dependency_pins.py` e `audit_ignores.py` esistono; non c'è un `deny.toml` | licenze e advisory non sono presidiate da uno strumento dedicato | valutare `cargo deny` accanto ai gate esistenti, senza duplicarli | il gate entra in un workflow che gira, altrimenti non serve |
| G3 | Ridurre senza toccare ciò che è qualificato | questo piano | i sei artefatti della 3.0.0 sono congelati | rimuovere una dipendenza cambia i byte: è lavoro da 4.0.0, mai retroattivo | ogni rimozione è un commit suo, con il checkpoint suo | nessuna riduzione tocca `assurance/evidence/` né i digest della 2.0.0 e della 3.0.0 |

### H — Seguito dei fork

| # | Requisito | Fonte | Comportamento attuale | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| H1 | Ogni fork dichiara i propri delta | `scripts/*-fork-lock.json` | `dxf 0.6.1` (59 file, 4 delta funzionali), `gdal 0.19.0` (63 file, 3), `shapefile 0.9.0` (29 file, 3) | **nessuno**: i lock sono già la forma giusta | mantenerli | il digest dell'albero versionato coincide col lock |
| H2 | Sapere se upstream ha assorbito un delta | pratica già in uso (`1d6454b`: «tre delta avanti, uno che upstream ha assorbito») | i tre fork sono **allineati** all'upstream corrente — `dxf 0.6.1`, `gdal 0.19.0`, `shapefile 0.9.0`, verificato su crates.io | nessuna deriva oggi. Manca il **controllo**: l'allineamento è il frutto di una revisione manuale, e la prossima potrebbe non esserci | un controllo che confronti la versione del lock con quella pubblicata upstream | il controllo nomina versione locale e versione upstream e dichiara la distanza; è rosso quando upstream avanza |
| H3 | Un delta che upstream ha assorbito si ritira | pratica già in uso | applicata alla 0.9.0 di `shapefile` | nessuno oggi; il rischio è dimenticarlo al prossimo giro | ogni aggiornamento di fork rivede i delta uno per uno | il messaggio di commit dice per ciascun delta se resta, e perché |

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

**Priorità 1 — il pin e la misura.** A2, A3, A5, I2. Senza un pin e un
verificatore black-box che gira, ogni riga successiva è un'opinione. Sono anche
le righe più economiche, e rendono misurabili tutte le altre.

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

**Priorità 6 — `io.write`.** B10. È l'unica operazione richiesta che manca del
tutto, ed è lavoro di prodotto, non di confine: ha senso solo dopo che la busta
e la scoperta sono stabili.

**Priorità 7 — standard, dipendenze, fork.** E1–E5, G1–G3, H2, H3. Migliorano il
repository senza essere precondizioni di nessuna riga di conformità.

**Priorità 8 — la qualifica.** I1, I4. Si chiudono per ultime perché misurano
tutto il resto.

---

## Le decisioni ancora necessarie

Sono quelle che non posso prendere leggendo il codice, e su cui il piano si
ferma.

**D1 — la superficie Rust è pubblica o dichiarata deviante?** Il profilo la
richiede. Oggi ogni crate è `publish = false` e il protocollo dichiara l'API
Rust `internal_unstable`. Le strade sono due e non sono equivalenti: esporre una
superficie Rust pubblica e stabile — con un crate pubblicabile, un contratto di
compatibilità e la mappatura verso gli export — oppure dichiarare una deviazione
su `SURF-016` che dice che l'artefatto non espone la superficie Rust, e vivere
con una conformità parziale. La prima è la più costosa di tutto il piano; la
seconda è onesta ma limita chi può comporci.

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

**D6 — quanto dello standard di database-tools adottare.** I loro gate sui
commenti nascono dalla loro storia: i marcatori di «cronaca obsoleta» colpiscono
frasi che qui potrebbero essere prosa legittima. Prima di adottare `E2` va
**misurato** quante violazioni produrrebbe sul nostro albero, e deciso se il
criterio è nostro o solo loro.

**D8 — un driver di formato è un `provider`?** La domanda è già registrata in
`docs/contracts/handoff-plenora-error.json` con identificatore
`driver-e-un-provider`. Il campo `provider` di `plenora-error-v1` suggerisce un
servizio o un backend remoto; per noi è il formato del file — csv, geoparquet,
shapefile — scelto dal chiamante e senza effetto remoto. Se la destinazione
intende `provider` nel primo senso, il valore appartiene a un `details`
component-owned come `format_id`. Il documento dice che il DTO è l'unico punto
che dovrà cambiare quando la risposta arriva.

**D7 — il tetto di dimensione del codice.** F3 chiede un numero. Fissarlo sul
valore corrente non impedisce nulla; fissarlo più in basso impegna a ridurre.
Il numero è una decisione, non una misura.

---

## Che cosa questo piano non fa

Non modifica API, specifiche condivise o la versione del prodotto. Non crea
documenti paralleli: l'unica estensione al docset è l'ammissione di questo file
e il collegamento da `README.md`, e ogni altro controllo resta com'era. Non
tocca tag, artefatti o evidenze della 2.0.0 e della 3.0.0, che restano
verificabili come lo erano il giorno in cui sono stati pubblicati.
