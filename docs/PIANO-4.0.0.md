# Piano 4.0.0 — adozione dei contratti pubblici

**Documento di pianificazione.** Non descrive ciò che il codice fa oggi — quello
lo dicono [docs/PRODUCT.md](PRODUCT.md) e [docs/ENGINEERING.md](ENGINEERING.md) —
ma lo **scostamento** fra ciò che il codice fa e ciò che
[`plenora-contracts`](https://github.com/PlenoraETL/plenora-contracts) richiede,
e l'ordine in cui chiuderlo.

Nessuna API, specifica condivisa o versione di prodotto è modificata da questo
documento. Tag, artefatti ed evidenze della 2.0.0 e della 3.0.0 restano intatti.

## Il perimetro, deciso

Quattro delle decisioni aperte sono state prese, e il piano smette di essere
una proposta su quei punti.

| decisione | esito |
|---|---|
| **D1** superficie Rust | **inclusa** nell'adozione: export documentati, compatibilità, prova da consumatore esterno |
| **D1b** canale | archivio **sorgente** del workspace, identificato da versione e digest. crates.io non è richiesto |
| **D4** runtime | **non applicabile** in questa fase, e dichiarato tale |
| **D5** revisione dei contratti | `453c8d1ff2eb260840e6cedc033a2b76b58a0b9e`, la prima in cui il profilo nomina la 4.0.0 |
| SDK Python | mantenuto e spedito, aggiornato insieme al CLI, verificato dalla **wheel installata**. Non reclama `plenora-python-sdk-v1`, che il profilo dichiara non applicabile |

Il perimetro è scritto in `contracts/adoption-source.json`, che è la fonte: le
righe qui sopra lo riassumono e non lo ridefiniscono.

## Le revisioni su cui il piano è costruito

Un piano che non nomina le revisioni che ha letto invecchia senza che nessuno se
ne accorga. Queste sono le sei consultate.

| repository | revisione | ruolo |
|---|---|---|
| `plenora-IO-tools` | `29c78a79f325b9b1e2ce6daebe2798b34f474b0e` | `main` al momento dell'analisi |
| `plenora-IO-tools` | `28bf62ccba49d47bda0797b9892e373c49ebea61` | revisione qualificata della 3.0.0, da cui esce l'artefatto misurato |
| `plenora-contracts` | `73ba27dd456fab420d18b2a52013c7eea2040215` | `main` al momento dell'analisi, riferimento normativo letto |
| `plenora-contracts` | `453c8d1ff2eb260840e6cedc033a2b76b58a0b9e` | **la revisione fissata**: `main` dopo l'integrazione della PR #5, che chiude HB1 |
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
| A2 | Pin immutabile del repository dei contratti | ADOPTION §1.2 | **chiusa.** `contracts/adoption-source.json` fissa `453c8d1ff2eb…`, dichiara il perimetro delle superfici e la ragione di ciascun `not_applicable` | — | fatto | `scripts/check_public_contracts.py` rifiuta un checkout la cui `HEAD` non coincide col pin, e una sonda lo verifica |
| A3 | Manifesto di adozione validato dallo schema v4 | ADOPTION §1.5, `adoption-manifest-v4.schema.json` | assente | nessuna dichiarazione di conformità pubblicabile | manifesto con `artifacts`, `contracts`, `deviations`; ogni artefatto porta `version` e `digest` `sha256:<64 hex>` | il manifesto valida contro lo schema v4 al pin; i digest coincidono con quelli congelati in `assurance/current-state.json` |
| A4 | Identità immutabile dell'artefatto | ADOPTION §2 | i sei digest sono già congelati nella candidate e verificati due volte | **nessuno** — la macchina di rilascio produce già esattamente ciò che il manifesto richiede | riusare i digest esistenti invece di ricalcolarli | i digest del manifesto sono gli stessi di `aperto.candidate_release.artefatti` |
| A5 | Prova black-box: il verificatore invoca un binario, non ispetta lo stato privato | ADOPTION §3 | **chiusa.** `scripts/check_public_contracts.py` riceve un percorso e interroga il processo; nessuna sonda legge un sorgente o una struttura interna | — | fatto, col job CI `profilo-pubblico` che costruisce il binario e lo interroga contro il pin | il verificatore protegge **8 requisiti su 19** sull'artefatto 3.0.0 e ne mostra 11 da implementare; 32 sonde provano che le tre regole mordono |
| A5b | Identità dell'artefatto qualificato | ADOPTION §2 | i digest sono congelati e verificati, ma nessun controllo lega il **binario interrogato** a un digest | il verificatore da solo non dice quale artefatto ha interrogato | in **qualifica** il binario si estrae dall'archivio identificato dal digest, e il verificatore riceve quel percorso | l'estrazione ricalcola il digest dell'archivio e lo confronta con `aperto.candidate_release.artefatti` **prima** di invocare; un digest diverso ferma la qualifica |
| A6 | Deviazioni dichiarate con regola, osservabilità e tracking | ADOPTION §5 | assente | una conformità parziale non dichiarata si legge come completa | ogni riga di questa matrice non chiusa alla 4.0.0 diventa una `deviation` con il proprio identificatore | ogni deviazione cita un identificatore esistente (`SURF-001`, `CLI-2.4`, …) e dichiara `detectable_before_invocation` |

### B — API pubbliche: CLI

Questa è l'area con lo scostamento maggiore, ed è anche quella dove le misure
sono più nette. Le righe qui sotto vengono dalle risposte del binario 3.0.0.

| # | Requisito | Fonte | Comportamento attuale (misurato) | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| B1 | `--help` presente | CLI-2.0 §3 | `--help` esce **2** con `CLI_USAGE: uso: plenora-io <catalog\|inspect\|layers\|read\|convert>` | la superficie di scoperta richiesta non esiste: l'aiuto è un errore | implementare `--help` che descrive **solo** i comandi compilati nel binario | `--help` esce 0 e non nomina comandi assenti da quella build |
| B2 | `--version --format json` nella busta comune | CLI-2.0 §3 | `--version` rende `{"status":"ok","version":"3.0.0"}`; con `--format json` esce **2**, «non prende argomenti» | busta non conforme e flag rifiutato | `--format json` accettato ovunque; `result` porta `component_version` e la versione di protocollo CLI | la risposta valida contro `cli-envelope-v2.schema.json` e `result.component_version` vale la versione del workspace |
| B3 | `capabilities --format json` | CLI-2.0 §3, CAP-003…CAP-009 | **chiusa.** Il comando risponde e dichiara sei operazioni: quattro disponibili, `io.read` e `io.write` **non** disponibili con la ragione | — | fatto | quattro requisiti la presidiano: forma, copertura del catalogo, nessuna matrice dei formati negli attributi, e la mappatura dei comandi |
| B3b | La build non cambia le operazioni | profilo io-tools, «Interchange» | `base` e `gdal-backend` producono lo **stesso** documento capability | **nessuno**, ed è la correzione di una mia riga sbagliata: avevo scritto come prova d'accettazione che le due build dovessero produrre documenti **diversi** | niente da fare: la feature cambia quali formati `io.catalog` dichiara, non quali operazioni esistono, e il profilo vieta agli attributi di duplicare quella matrice | il documento capability è identico fra le due build; la differenza si legge nel risultato di `io.catalog` |
| B4 | Una sola versione di protocollo nell'artefatto | profilo io-tools, «cutover» | `catalog` rende `protocol_version: 2`; **ogni percorso d'errore misurato** — uso, `io`, `unsupported` — rende `protocol_version: 1` | v1 e v2 **coesistono** nello stesso binario, che il profilo vieta. Non è il solo errore d'uso: è tutta la superficie d'errore | portare ogni percorso alla busta v2; `--legacy-protocol-v1-unsafe` va rimosso o isolato dietro un artefatto separato | nessuna risposta del binario conforme porta `protocol_version: 1`, successo **ed errore** |
| B5 | Niente su stderr in modo JSON | CLI-2.0 §4 | gli errori escono **su stderr**, stdout vuoto | selezione dello stream non conforme; un consumatore che legge stdout non vede nulla | busta d'errore su **stdout**, un documento e una newline | per ogni comando, in fallimento: stdout è un documento JSON, stderr è vuoto, exit ≠ 0 |
| B6 | Campi d'identità nella busta | CLI-2.0 §5 | busta di successo: `contract`, `determinism`, `drivers`, `protocol_version`, `status` | mancano `component`, `component_version`, `command` | aggiungerli a ogni busta, successo ed errore | ogni busta valida contro `cli-envelope-v2.schema.json`, che li richiede |
| B7 | Dati dell'operazione dentro `result` | CLI-2.0 §5 | `drivers` e `determinism` stanno **al primo livello** | campi di primo livello aggiuntivi, vietati | spostare il corpo dentro `result` senza cambiarne la forma interna | le chiavi di primo livello sono esattamente quelle della busta; tutto il resto è sotto `result` |
| B8 | Identificatore di contratto del catalogo | catalogo io-tools | `contract: plenora-io-catalog-v2` | il catalogo comune fissa `plenora-io-catalog-v1` per `io.catalog@1` | allineare l'identificatore, oppure dichiarare la deviazione e la ragione | l'identificatore emesso coincide con quello del catalogo comune al pin |
| B9 | `--format json` come selettore esplicito | CLI-2.0 §2 | `catalog --format json` esce **2**: il flag non è accettato | la modalità macchina non è selezionabile come il contratto prescrive | `--format json` su tutti i comandi; formato umano esplicito e mai implicito | ogni entrypoint del binding CLI è invocabile **letteralmente** come scritto in `bindings/cli-v1.json` |
| B10 | Comando `write` | catalogo io-tools, `io.write`, profilo «External outcomes» | **chiusa.** `write INGRESSO.arrow DESTINAZIONE --to FORMATO` pubblica un dataset Arrow; `io.write` è `available` nel documento capability, e tutte e sei le operazioni del catalogo lo sono | — | fatto: il formato del sink è **nominato** e non dedotto, le fedeltà d'ingresso e di scrittura restano distinte, e l'esito di pubblicazione porta i due stati osservabili | undici prove del binario, cinque dell'SDK dalla wheel, tre requisiti del verificatore pubblico, due schemi con nove esempi, e due casi nel censimento delle buste. Il giro si chiude: ciò che `read --output` consegna, `write` lo pubblica |
| B10a | Il formato del sink è esplicito | profilo io-tools, «Format-specific behavior MUST NOT be selected by parsing file extensions when the operation requires an explicit format» | **chiusa.** `--to` prende un `id` di `io.catalog`; senza, il comando rifiuta invece di indovinare | `convert` deduce ancora i due formati dalle estensioni: è la riga B17 | `driver_per_formato` risolve per nome, e `ogni_formato_del_catalogo_ha_un_driver` confronta i due insiemi nei due versi | quando `--to` e l'estensione della destinazione si contraddicono non viene scritto **né** l'uno **né** l'altro formato: è l'unico caso in cui la regola si osserva, perché quando concordano le due strade portano allo stesso posto |
| B12 | `io.read` **consegna** dati Arrow | catalogo io-tools, ARROW-001, ARROW-005, binding `read SOURCE --output OUTPUT.arrow` | **chiusa.** `read --output` scrive un file Arrow IPC; senza `--output` la forma che conta resta, e `delivered` vale `null` | — | fatto | diciassette prove aprono il file con `arrow-ipc` — la libreria che userebbe chi ci consuma — e ne verificano valori, tipi, nullabilità, `plenora.contract.version`, `geoarrow.wkb` e il CRS risolto; quattro prove dell'SDK fanno lo stesso dalla wheel; due schemi pubblicati e quindici esempi legano la forma dichiarata all'invocazione reale |
| B12a | Dataset vuoto: zero righe **non** implica schema ignoto | ARROW-VOCABULARY-1.0 §3-4, `capabilities.rs` | **corretta.** Una sorgente con zero righe e schema noto — GeoPackage, Arrow IPC, Parquet, Shapefile — si consegna con lo schema intero; solo una sorgente i cui tipi geometrici non si determinano viene rifiutata | il rilievo: la prova precedente asseriva «un dataset vuoto non si consegna» e ne dava come ragione che una sorgente senza righe non può dichiarare i tipi. Entrambe sbagliate. La regola vera non parla di righe: rifiuta quando il **sink restringe** i tipi geometrici **e** il contratto della sorgente arriva `unresolved` — una sorgente con mille righe di tipi indeterminati sarebbe rifiutata allo stesso modo | fatto: due prove distinte, `zero_righe_con_schema_noto_si_consegnano` e `tipi_non_determinabili_verso_un_sink_che_li_pretende_e_rifiutato` | la prima costruisce un file IPC con lo schema della fixture canonica e **zero** batch, e ne verifica la consegna con schema intero; la seconda verifica che il messaggio nomini la dichiarazione dei tipi e non il numero di righe |
| B12c | Il sink IPC dichiara sette dei sedici tipi canonici | `descriptor.rs` `SIMPLE_WKB_GEOMETRY_TYPES`, ARROW-VOCABULARY-1.0 | il driver IPC trasporta WKB **senza interpretarlo**, e dichiara `WKB_EWKB_PASSTHROUGH_GEOMETRY`, che restringe a sette tipi | è quella restrizione, e non il vuoto, a far rifiutare `unresolved`. Per un passaggio di byte la restrizione è più stretta del vero; ma la costante è condivisa con CSV, GeoParquet, GPKG e XLS, dove non lo è altrettanto | **aperto, e la strada a monte è più lunga di quanto pensassi**: la PR #6 non aggiunge più la chiave alla `1.0`, perché quel vocabolario si dichiara chiuso — l'analisi è la decisione 0006, che lascia due strade: un vocabolario successore che alza la versione del contratto, oppure `plenora.geometry.native.*` come spazio riservato ai provider. **Una metà però non dipende da monte**: il rifiuto scatta sul contratto che il reader costruisce, non sul filo, e un reader che sapesse dire «scandito, nessuna geometria» al proprio sink risolverebbe il caso locale senza toccare nessun contratto condiviso. La parte che serve il filo — dirlo a un altro componente — resta appesa alla decisione 0006 | il caso locale si chiude quando il contratto interno distingue i due stati; quello sul filo quando esiste un successore del vocabolario. Il pin resta `453c8d1`, e nessun requisito del verificatore dipende dalla proposta |
| B12b | Il limite non si combina con la consegna | **nessun requisito lo impone**: SURF-014 vieta solo di riportare un parziale come successo pieno, e PUBLIC-SURFACES-1.0 §9.5 delega la semantica del parziale alla nostra specifica d'operazione | `--limit` insieme a `--output` è rifiutato con `invalid_plan` e codice `LIMIT_WITH_DELIVERY` | il difetto che le prove hanno trovato era reale — `--limit 2 --output` riusciva e consegnava **tutte** le righe con `truncated: true` accanto — ma il rifiuto che l'ha chiuso è una **scelta**, non un obbligo. La necessità del writer di conoscere la cardinalità esatta (`declare_input_total`) è una ragione per progettare la semantica, non per negarla | **D9 decisa per la 4.0.0**: `io.read` non consegna dataset parziali, e la scelta è scritta per esteso in `plenora-io-read-input-v1` — schema, codice, prova e SDK la citano invece di ripeterla | il comando esce `invalid_plan` e non lascia un file; `--limit` da solo continua a valere e governa quante righe si **leggono**. Cambiarla sarebbe un cambio di contratto con un identificatore nuovo, non una riga tolta |
| B13 | Streaming senza materializzazione completa | ARROW-011 | il lettore è già a batch e lo spool è documentato in ENGINEERING | da verificare **al confine**, non nell'implementazione | dichiarare nel descrittore se la materializzazione è limitata, e provarlo | il consumatore elabora il primo batch prima che l'ultimo sia stato prodotto, oppure il descrittore dichiara la materializzazione limitata |
| B17 | `convert` deduce i formati dalle estensioni | catalogo io-tools (`io.convert`: «Convert an external dataset between **explicit** formats»), profilo io-tools | `cmd_convert` chiama `driver_for_path` su entrambi i percorsi: i due formati vengono dall'estensione | la stessa regola che `io.write` ora rispetta. `io.convert` è descritto dal catalogo come operazione fra formati **espliciti**, quindi la deduzione non è un default comodo ma la selezione che il profilo esclude | **chiusa.** `convert IN OUT --from F --to G`; senza i due formati è un errore d'uso, non un ritorno alla deduzione. `inspect`, `layers` e `read` **non** cambiano: quelle riconoscono la sorgente invece di riceverla dichiarata, e il catalogo le distingue operazione per operazione | `docs/INSTALL.md` porta la migrazione 3.x → 4.0.0 con la corrispondenza fra le dieci estensioni di prima e gli identificatori di adesso; le prove d'integrazione passano i due formati, e l'helper che li deriva dichiara perché una prova può dedurre dove il prodotto non deve |
| B14 | Il nome del contratto nella busta diverge dal catalogo | catalogo io-tools, CLI-2.0 §5 e §10 | la busta annuncia `plenora-io-read-v2`, `…-inspect-v2`, `…-layers-v2`, `…-convert-v2`. `write` no: annuncia `plenora-io-write-result-v1`, perché nasce con i suoi schemi pubblicati e non ha consumatori da migrare | il catalogo comune nomina quegli stessi contratti `plenora-io-read-result-v1`, `…-inspect-v1`, `…-layers-v1`, `…-convert-v1`. CLI-2.0 §10 esige che il contratto d'uscita resti **equivalente** fra le superfici, e un nome diverso non è equivalente. Il `-v2` nostro segue il protocollo della busta, che è un'altra cosa dalla versione dell'operazione | **chiusa.** Allineati cinque nomi, non quattro: il confronto ha trovato anche che la busta d'errore è `plenora-error-v1` **senza** `io` (ERRORS-1.0), e che `read` è `-read-result-v1` e non `-read-v1`. Due voci che una sostituzione per somiglianza avrebbe sbagliato | `contracts/nomi-dei-contratti.json` dichiara i nove nomi con la loro fonte, e la sonda `busta.nomi-dal-contratto-fissato` li confronta con il catalogo e con la riga «Contract identifier» delle due specifiche del checkout fissato, poi con ciò che il binario emette |
| B15 | Due grafie per lo stato del CRS | ARROW-007, ARROW-VOCABULARY-1.0 §3 | il risultato JSON rende `declared_but_unresolved` (grafia derivata da serde), i metadati Arrow scrivono `declared_unresolved` (grafia del contratto) | lo stesso stato, due nomi, dentro lo stesso prodotto. Nessuno dei due è sbagliato in sé — il JSON è contratto nostro — ma chi confronta le due viste deve tradurre, e una traduzione non scritta è una traduzione che prima o poi si sbaglia | **chiusa.** Il JSON rende ora `declared_unresolved`, la grafia che `ARROW-VOCABULARY-1.0 §3` chiude e che i metadati Arrow già scrivevano. Il nome Rust resta `DeclaredButUnresolved`, dove si legge meglio: a viaggiare è la grafia del contratto | lo schema del risultato chiude l'enum sui tre valori del contratto, e le buste reali vi validano contro |
| B16 | Opzioni accettate e senza effetto | SURF-014, `plenora-io-read-input-v1` | **chiusa.** `--in-opt` era accettata da `read`, `inspect` e `layers` e **scartata** — solo `convert` la applicava; `--out-opt` e `--durable` erano accettate anche senza `--output`, cioè senza alcun writer a cui rivolgersi | trovate scrivendo lo schema d'ingresso, non leggendo il codice: dichiarare che cosa l'operazione accetta ha reso visibile che accettava di più di quel che usava. Una chiave sbagliata passava in silenzio, indistinguibile da una applicata — e lo stesso valeva per `options=` dell'SDK, il cui docstring prometteva un rifiuto che non arrivava | fatto: `read_options` unisce `--opt` e `--in-opt`; `read` rifiuta `SINK_OPTIONS_WITHOUT_SINK` quando le opzioni del sink arrivano senza destinazione | cinque esempi invalidi dello schema portano l'invocazione CLI equivalente e il codice atteso, e una prova verifica che il binario rifiuti **lo stesso** insieme che lo schema rifiuta; due sonde dell'SDK lo esercitano dalla wheel |
| B11 | Proiezione dei codici d'uscita | CLI-2.0 §8 | la proiezione è agganciata a `IoErrorCode`, **non** alla categoria del contratto: vedi la tabella qui sotto | scostamento **incompatibile**: solo `cancelled → 130` coincide | riscrivere la proiezione sulla `category`, che è l'asse autoritativo | una prova tabellare copre ogni categoria e il suo codice atteso, e fallisce se la chiave torna a essere il codice interno |

### B-bis — I contratti di confine di proprietà nostra

Il profilo non chiede solo che le sei operazioni esistano: chiede che IO-tools
**pubblichi** gli schemi delle dodici forme di ingresso e uscita, con esempi di
conformità, prima che un artefatto reclami il profilo. Sono schemi nostri — il
repository comune ne fissa gli identificatori pubblici e il significato
incrociato, non l'implementazione. Mancava ogni riga operativa.

| # | Requisito | Fonte | Comportamento attuale | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| BB1 | Dodici schemi immutabili per le sei coppie | profilo io-tools, «Component-owned wire contracts» | **chiusa: dodici su dodici** in `contracts/schemas/`, con gli identificatori che il catalogo comune assegna; il CLI emette `plenora-io-catalog-v2` senza uno schema che lo definisca | — | fatto | `ogni_busta_reale_valida_contro_lo_schema_della_sua_operazione` invoca **undici** forme reali — `inspect` su formato a layer unico e multi-layer, `layers` su entrambi, `read` con e senza consegna, `write` verso due sink, `convert` verso un formato che perde e uno che non perde — e valida ciascuna contro lo schema del contratto che la busta annuncia. Le sei uscite del catalogo sono coperte tutte, e la sonda lo conta |
| BB2 | Esempi di conformità validi e invalidi | profilo io-tools | **chiusa: quarantaquattro esempi** per tutte e sei le coppie, con un manifesto che ne dichiara il verdetto atteso | — | fatto | il gate valida i «validi» e **rifiuta** gli «invalidi»; in più **undici** invalidi portano l'invocazione CLI equivalente e il codice che il prodotto deve rendere, così che schema e binario debbano rifiutare lo stesso insieme. Un esempio invalido che il solo validatore rifiuta proverebbe che il validatore sa dire di no |
| BB4bis | Le perdite non sono un documento `plenora-row-diagnostics-v1` | ROW-DIAGNOSTICS-1.0, DIAG-001…013 | il blocco `loss` del risultato ha forma nostra: `counts`, `esempi`, `omesse`, `troncato`, `lossless` | mancano `index_basis`, `knowledge_limits`, `observed_total`, `examples_limit`, e `contract` non vale `plenora-row-diagnostics-v1`. Quel contratto è fra quelli **adottati**, quindi o le perdite vi si conformano o la deviazione va dichiarata nel manifesto con la sua ragione | **chiusa, e la risposta non era rinominare.** `loss` è indicizzata per **layer, campo e classe di tipo**: la sua struttura interna (`Posizione`) non ha proprio un posto dove mettere un indice di riga, quindi non è diagnostica di riga mancata ma un'altra cosa — che cosa il formato non ha saputo rappresentare. Il documento che **è** row-scoped esiste già ed è conforme: `row_diagnostics` nella busta d'errore porta tutti e nove i campi obbligatori, `contract: plenora-row-diagnostics-v1`, `index_basis`, `completeness`, `knowledge_limits` | la sonda `diagnostica.riga-e-il-contratto-condiviso` legge i campi obbligatori e i due enum **dallo schema del checkout fissato** invece di ricopiarli, e li confronta con il documento emesso. Rinominare `loss` avrebbe prodotto due documenti con lo stesso nome che rispondono a domande diverse |
| BB3 | `plenora-io-error-details-v1` | profilo io-tools, ERR-013 | **schema pubblicato**, con i campi che `ErrorContext` già calcola: driver, indice di layer, indice di campo, ragione della capacità, identificatore del contratto | **nessun percorso d'errore del prodotto emette `details`**: la settima chiave delle buste d'errore è `row_diagnostics`, che è un altro contratto. Il profilo lo ammette — «Omitting `details` remains valid when the four common error axes and optional `code` completely express the failure» — e finora lo esprimono | pubblicato lo stesso, perché ERR-013 rende immutabile l'identificatore: sceglierla adesso è meglio che scoprirla sotto pressione al primo errore che ne avrà bisogno. Gli indici e non i nomi, perché un nome viene dalla sorgente e il messaggio deve restare redatto | la sonda `errore.dettagli-tipizzati` verifica che lo schema sia pubblicato e che nessun errore emetta un `details` fuori dalla forma. **Oggi passa per assenza di soggetto, e lo dice nel dettaglio**: un verde che non dichiari di non aver misurato niente è il verde peggiore |
| BB4 | Limiti semantici di ERR-011 e ERR-012 | ERRORS-1.0 §6 | **confrontati**, sette assi in `contracts/limiti-della-diagnostica.json`: su sei governa il prodotto, da due a trentadue volte più stretto | **un asse resta scoperto e il registro lo dichiara**: `MAX_BYTE_BUSTA` limita le cinque sezioni diagnostiche, non la busta d'errore intera, che porta anche identità, messaggio e `row_diagnostics`. Il tetto di ERR-011 è sull'errore **intero**: i due non misurano la stessa cosa | dichiarati con la fonte e la revisione, e confrontati da una sonda invece che a mano. I numeri del contratto sono ricopiati una volta perché ERRORS-1.0 li pubblica come prosa: è la stessa richiesta già registrata come HB2 | `errore.limiti-dichiarati` verifica che nessun tetto del prodotto superi quello del contratto, che la colonna «governa» segua i numeri, e **misura i byte** dell'errore reale con diagnostica di riga. Quella misura è un limite inferiore alla verità — dice che quel caso sta sotto il tetto, non che ogni caso ci stia — e il registro lo scrive invece di lasciarlo credere |
| BB5 | Verifica dei metadati Arrow al confine | ARROW-001…ARROW-012 | **chiusa per sei requisiti**, in `tests/metadati_arrow.rs`, letti con `arrow-ipc` dal file consegnato: ARROW-001, ARROW-003/004, ARROW-005, ARROW-006, ARROW-007, ARROW-012 | i cinque non coperti hanno ciascuno la sua ragione, scritta nel modulo: ARROW-002 è una regola sul **consumatore**; ARROW-008 lo esercita `provenienza_crs.rs` lungo tutte le conversioni; ARROW-009 vieta di *richiedere* metadati del provider, ed è una proprietà di ciò che non facciamo; ARROW-010 vale per chi dichiara pass-through lossless, e `io.read` non lo dichiara; ARROW-011 è streaming, cioè B13 | fatto | ogni requisito è nominato in una sonda sua: una copertura che sparisse si potrebbe nominare anche lei |
| BB5a | L'identità dei campi attributo sotto proiezione | ARROW-003, `projection_support` | **misurato scrivendo la sonda**: solo la colonna geometrica porta `plenora.field_id`; le colonne attributo no | ARROW-003 è condizionale — l'identità deve sopravvivere per i campi *che ne hanno bisogno* — e oggi nessuna superficie pubblica **proietta**: la CLI non ha un argomento per selezionare le colonne, quindi lo schema consegnato è sempre completo e il `field_index` su cui `loss.esempi` indicizza non si sposta mai | **aperto, e legato a B13**: il giorno in cui una proiezione entra nel confine pubblico, `field_index` diventa relativo allo schema proiettato, due letture con proiezioni diverse smettono di essere correlabili, e l'identità degli attributi è esattamente ciò che «deve sopravvivere» | la sonda fissa **quali** campi portano l'identità: se un attributo la acquistasse o la geometria la perdesse, cambierebbe una decisione e andrebbe scritta |

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
| HB1 | Il profilo nomina la prima release conforme | `profiles/io-tools.md`, «First conforming release and cutover» | **chiusa.** Il profilo nomina ora la `4.0.0`, e dichiara `2.x` e `3.x` storiche accanto a `1.x` | — | PR #5 di `plenora-contracts`, integrata in `453c8d1`. `CUTOVER.md` conserva la frase originale come record di ciò che fu deciso, e dice che è cambiata la release che porta la decisione, non la decisione | il pin di `contracts/adoption-source.json` punta a `453c8d1`, che contiene la correzione; `tools/validate_specs.py` passa su quella revisione |
| HB2 | La proiezione dei codici d'uscita esiste in forma macchina | CLI-2.0 §8 | la proiezione vive **solo** come tabella Markdown; nel repository comune non c'è un file macchina che la porti | il nostro verificatore ha dovuto tenerne una copia in `contracts/proiezione-codici-uscita.json`, perché `check_docset.py` vieta a un gate di dipendere dalla prosa — e aveva ragione: il parser che leggeva il Markdown sbagliava già la riga del `70` | proporre a `plenora-contracts` di pubblicarla accanto agli schemi, come gli altri fatti macchina | la copia locale sparisce e il verificatore legge la proiezione dal checkout fissato; finché esiste, la sua completezza è verificata contro l'enum chiuso a ogni corsa |

Va proposta **prima** di fissare il pin (A2): fissare una revisione che dice la
cosa sbagliata, e correggerla dopo, significherebbe cambiare pin a metà
adozione. È anche la ragione per cui questa riga non appartiene alla priorità 1
ma la precede.

### HB3 — `io.read` con `side_effect: none` non è realizzabile da una CLI

Il catalogo comune dichiara `io.read` con `side_effect: none` e, fra i content
type d'uscita, `application/vnd.apache.arrow.stream` e `.file`.
SURFACE-BINDINGS-1.0 §1 esige che una superficie preservi «contratti, default,
assi d'errore, **effetti collaterali** e controlli d'esecuzione» del catalogo.

Ma CLI-2.0 §4 riserva stdout a **esattamente un** documento JSON. I byte Arrow
non hanno altra strada che un file, e scrivere un file è un effetto locale.
Quindi un binding CLI conforme a CLI-2.0 non può servire `io.read` con
`side_effect: none`: le due clausole si escludono.

Oggi il nostro documento capability dichiara `side_effect: local` per `io.read`,
che è vero per questa superficie e diverso dal catalogo. Le uscite possibili
sono tre, e la scelta non è nostra da sola:

1. il catalogo distingue l'effetto **per superficie**, come già fa per i
   content type che una superficie può o non può produrre;
2. `io.read` resta `none` e la consegna su file diventa un'operazione sua;
3. resta come sta e la differenza si dichiara come deviazione nel manifesto di
   adozione, con questa ragione.

**Proposta, PR #6 di `plenora-contracts`.** La strada scelta è la prima delle
tre: il catalogo descrive l'**operazione**, e le meccaniche che una superficie
deve usare per consegnarne il risultato appartengono al **binding**. Una
superficie che materializza un risultato deve accettare la destinazione come
ingresso dichiarato, non dedurla mai, dichiarare l'effetto sulla propria
superficie, e non toccare identificatore, versione, contratti, assi d'errore e
controlli dell'operazione.

È compatibile secondo `COMPATIBILITY.md`: chiarisce la prosa e permette una
dichiarazione veritiera dove non ne esisteva nessuna, senza cambiare dati
accettati o semantica visibile a un consumatore.

Fino all'integrazione `local` resta, perché dichiarare `none` su una superficie
che scrive un file sarebbe la sola delle risposte falsa — e il pin resta
`453c8d1`, perché fissare una revisione che contiene una proposta non ancora
decisa fisserebbe la proposta.

### HB4 — il vocabolario non distingue «scandito, nessuna geometria» da «ignoto»

`ARROW-VOCABULARY-1.0 §3` chiude `types_declaration` su `exact`, `mixed` e
`unresolved`, e §4 aggiunge che `exact` esige una lista **non vuota**. Una
sorgente scandita per intero e priva di geometrie — un GeoJSON con zero feature
— non ha quindi modo di dire «li ho guardati tutti, non ce n'è nessuno»: deve
dichiararsi `unresolved`, cioè indistinguibile da una sorgente i cui tipi non si
sono potuti determinare.

I due stati portano garanzie diverse, e un sink che restringe i tipi potrebbe
accettare il primo e rifiutare il secondo. È la ragione per cui la sorgente
vuota di GeoJSON viene rifiutata (B12a) mentre il GeoPackage vuoto passa: non
perché siano diversi i dati, ma perché uno dei due ha potuto dichiarare.

**Proposta, PR #6 di `plenora-contracts` — e la prima stesura era sbagliata.**
Avevo messo `plenora.geometry.types_scan` direttamente in
`ARROW-VOCABULARY-1.0`, argomentando da `COMPATIBILITY.md` che un campo
facoltativo la cui assenza preserva il comportamento è compatibile. Quell'argomento
non basta, e tre cose me l'hanno mostrato:

* **il vocabolario si chiude da solo** nella sua prima frase — «This document
  closes the wire vocabulary» — quindi un consumatore può validare che un campo
  geometrico porti queste chiavi *e nessun'altra*. Aggiungere a un insieme
  chiuso non è aggiungere un campo facoltativo a uno aperto;
* **`plenora.contract.version` porta già il meccanismo**: §1 dice che le
  versioni ignote falliscono chiuse. Lasciarla a `1` non ne usa niente, e chiede
  ai consumatori esistenti una tolleranza che il documento non ha mai promesso;
* **uno schema permissivo non è un contratto permissivo**: il vettore di
  conformità accetta qualunque chiave di metadato con valore stringa, ma è una
  proprietà di come si validano i vettori, non un'affermazione su che cosa un
  consumatore conforme debba accettare. Dedurre la compatibilità da lì
  renderebbe compatibile ogni chiave futura per costruzione — l'opposto di ciò
  per cui un vocabolario si chiude.

La 1.0 è quindi ripristinata intatta, e l'analisi è diventata la **decisione
0006** con le due strade che lascia: la distinzione condivisa appartiene a un
vocabolario successore che alza la versione del contratto, e un componente che
ne abbia bisogno prima usa `plenora.geometry.native.*`, che `ARROW-INTERCHANGE-1.0
§5` riserva esattamente a questo e che ARROW-009 e ARROW-010 già governano.

Finché non c'è un successore, un sink che restringe i tipi continua a rifiutare
le sorgenti dai tipi indeterminati, comprese quelle certamente vuote. Il rifiuto
è prudente e non perde dati: rifiuta soltanto un lavoro che sarebbe stato
sicuro.

### I — Qualifica finale

| # | Requisito | Fonte | Comportamento attuale | Scostamento | Intervento | Prova di accettazione |
|---|---|---|---|---|---|---|
| I1 | Il gate del contratto di release copre i nuovi obblighi | `scripts/check_release_contract.py` | 37 invarianti, 35 verificati, 0 bloccanti, 2 differiti | i requisiti di questo piano non sono invarianti: nessuno li presidia | un invariante `contratti.profilo-pubblico` bloccante finché l'adozione non è verificata | il gate è **rosso** finché il black-box del profilo non passa |
| I2 | Il black-box gira in CI su ogni push | `AGENTS.md` di database-tools, regola 6 | **chiusa.** Il job `profilo-pubblico` fa il checkout immutabile dei contratti al pin, esegue le sonde del verificatore e lo lancia sul binario appena costruito | — | fatto | il job è rosso se il pin e il checkout divergono, e una sonda fallisce se in CI il checkout manca — senza, tutte le sonde salterebbero e il job sarebbe verde senza aver misurato niente |
| I3 | La qualifica cross-component resta differita o si chiude | invariante `sistema.qualifica-cross-component` | differita e **non verificata** per decisione del titolare | la catena IO → data → database non è provata in nessuna direzione | l'adozione dei contratti comuni è la **precondizione**, non la prova | quando `release/system-rc-gate.json` passa a `satisfied`, l'invariante torna verificato da sé |
| I4 | La 4.0.0 è una major, e lo è per ragioni dichiarate | COMPATIBILITY, «Incompatible changes» | la 3.0.0 è pubblicata e intatta | — | ogni riga incompatibile di questa matrice è elencata nelle note di rilascio | le note nominano B4, B5, B7 e ogni altra rottura osservabile |
| I5 | Le evidenze precedenti restano leggibili | `verifica_release_storiche` | 2.0.0 in archivio, 3.0.0 in `aperto.candidate_release` come `pubblicata` | — | non toccare nulla di ciò che è registrato | i digest, i tag e le evidenze della 2.0.0 e della 3.0.0 sono identici a prima |

---

## Lo stato del verificatore

`scripts/check_public_contracts.py` esiste e gira. Dopo il blocco della busta
CLI e quello delle capability:

```
profilo pubblico: 32 requisiti verificati e protetti su 32.
```

Era 8 su 19 quando il verificatore è nato.

### Trentadue su trentadue non è l'adozione

È il momento in cui quel numero inganna di più, e va detto qui prima che altrove.
Dice che i requisiti **elencati in quel registro** sono verificati sul confine
pubblico. Fuori restano:

* **C1–C4** — la superficie Rust, la sua mappatura versionata e il consumatore
  esterno che la importa;
* **B13 e BB5a** — lo streaming senza materializzazione completa, e con esso
  l'identità dei campi attributo, che diventa necessaria appena una proiezione
  entra nel confine pubblico;
* **A3, A6, I1** — il manifesto di adozione con le sue deviazioni, e la qualifica.

Un registro completo misura tutto ciò che gli è stato chiesto di misurare, non
tutto ciò che serve: il gate non sa che cosa manchi al registro stesso.
Aggiungere una riga lì è il modo in cui la misura cresce, e ogni blocco che
segue ne aggiunge.

I ventiquattro requisiti stanno in `contracts/requisiti-pubblici.json`, ciascuno
con la propria regola, lo stato, e — quando manca — la ragione scritta. Le tre
regole che lo rendono incrementale:

* un requisito **`implementato` che fallisce** è rosso: è ciò che la CI protegge;
* un requisito **`non_ancora` che fallisce** è riportato e non blocca: è ciò che
  la CI rende visibile, e l'alternativa sarebbe una CI rossa dal primo giorno
  fino alla fine dell'adozione, cioè una CI che nessuno guarda;
* un requisito **`non_ancora` che passa** è rosso. È la regola che tiene in
  piedi le altre due: un requisito soddisfatto ma dichiarato mancante lascia il
  registro a descrivere un artefatto che non esiste più, e da quel momento la
  prima regola non lo protegge.

Con `--esigente` la seconda regola cade e qualunque `non_ancora` è rosso: è la
forma che il gate assume quando si qualifica una candidate. Oggi in quella forma
è rosso su undici requisiti, ed è l'esito giusto.

Gli otto già protetti sono i quattro assi d'errore, la categoria e la fase
dentro gli enum chiusi, la coerenza di `retry`, e quattro proprietà della busta
di successo. Non sono poco: sono la parte del contratto che il prodotto
rispettava già prima di sapere di doverlo fare.

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

**Priorità 3 — la scoperta.** ✅ **chiusa.** B3 e A1. Il documento capability
rende selezionabile il componente da un orchestratore, e dichiara con la ragione
le due operazioni che non serve. L'identificatore `plenora-io-tools` era la
precondizione, perché il documento lo contiene.

**Priorità 4 — la separazione dei test.** F1 e F2. Quarantaquattro moduli da
spostare sono lavoro meccanico e voluminoso: conviene farlo quando la superficie
pubblica è ferma, non mentre cambia. F4, l'estrazione dei tipi di confine, viene
qui perché è la precondizione di C1.

**Priorità 5 — la superficie Rust.** C1, C2, C3, C4, dopo la decisione D1. È il
punto con più incertezza e più costo, e non blocca nulla di ciò che precede.

**Priorità 6 — le due operazioni sui dati.** B12 ✅ e B10 ✅ **chiuse**:
`io.read` consegna e `io.write` pubblica, e il giro fra le due si chiude — ciò
che esce dalla prima entra nella seconda, con lo stesso contratto
d'interscambio dichiarato dalle due parti. Resta **B13**, lo streaming senza
materializzazione completa, che segue da qui.

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

**D9 — decisa: `io.read` non consegna dataset parziali.** Il contratto fissato
non lo imponeva — SURF-014 vieta soltanto di riportare un esito parziale come
successo pieno, e la busta lo riporta già con `truncated`; PUBLIC-SURFACES-1.0
§9 punto 5 delega «success, partial and failure semantics» alla specifica
dell'operazione, che è nostra — quindi la scelta era nostra, ed è questa.

La ragione non è il writer, ed è più stretta di come l'avevo scritta la prima
volta. Sta in **questo** contratto d'uscita, che porta un totale **solo**. Con
uno solo le due letture si escludono: se vale quello della sorgente, il file
consegnato non gli corrisponde e chi lo rilegge conta meno righe di quante il
documento ne dichiari; se vale quello consegnato, sparisce l'informazione su
quanto è rimasto indietro, che è precisamente ciò per cui il limite era stato
chiesto.

Non è una proprietà del problema: è una proprietà della forma scelta. **Un
contratto futuro che rappresentasse separatamente le righe della sorgente e
quelle consegnate direbbe entrambe le cose senza che nessuna cancelli l'altra**,
e allora una consegna parziale sarebbe esprimibile e questa politica andrebbe
riaperta. Rifiutare adesso non chiude quella porta: la tiene aperta, perché un
rifiuto si toglie mentre una semantica già consegnata no.

«Le prime N righe come dataset» resta intanto un'operazione legittima e
**diversa**: è una proiezione, non una lettura limitata. Fino ad allora `limit`
governa quante righe si **leggono**, non quante se ne consegnano.

La decisione è scritta in
`contracts/schemas/plenora-io-read-input-v1.schema.json`; codice, prova e SDK la
citano invece di ripeterla, e cambiarla sarebbe un cambio di contratto con un
identificatore nuovo.

**D10 — decisa: il nome del file è del chiamante.** Ogni driver scrivibile
pretendeva l'estensione sua, e la domanda era se il formato esplicito bastasse a
scegliere la destinazione. La risposta è venuta separando due cose che si
somigliavano.

In **sette** driver su dieci il controllo era una convenzione: dopo di esso
l'estensione non veniva usata per niente — né per derivare un nome, né per
scegliere un comportamento. Quelli sono stati tolti, e `--to` è diventato il
selettore vero invece di un controllo di coerenza.

In **due** il vincolo è del formato, e resta:

* `gpkg` perché la specifica GeoPackage lo impone (OGC 12-128r, requisito 2):
  gli stessi byte con un altro nome non sono un GeoPackage conforme, e
  scriverli sarebbe produrre un artefatto che si sa già fuori specifica;
* `shp` perché i file companion derivano il nome dal principale, e `.shp`
  contro `.shp.d` sceglie fra due pubblicazioni con garanzie di atomicità
  diverse. Lì il suffisso non descrive il contenuto: lo determina.

Nessun percorso viene riscritto: un vincolo non soddisfatto è un rifiuto
tipizzato, mai una rinomina silenziosa. E la metà che restava vera — che un
artefatto scritto su un nome qualsiasi non venga **riconosciuto** da chi lo
rilegge senza dichiarare il formato — non è sparita con il controllo:
`recognised_suffixes` la dichiara, il catalogo la pubblica, e la sonda
`write.vincoli-del-percorso` verifica che ogni driver dichiari entrambe le cose.

Restava una terza categoria che non avevo voluto inventare, ed è ora
**misurata**. `filegdb` è un dataset a directory e il suo staging è una `.gdb`,
quindi la tentazione era di dichiararlo vincolato per simmetria. Con
`gdal-backend` acceso la misura dice altro:

```text
write g.arrow senza_suffisso --to filegdb   -> ok, 2 righe
  inspect senza_suffisso                    -> estensione non riconosciuta
write g.arrow con.gdb        --to filegdb   -> ok, 2 righe
  inspect con.gdb                           -> ok
```

Scrivere su un nome qualunque **riesce** e produce un dataset valido; è
rileggerlo per deduzione che non si può. È il comportamento dei sette driver
liberi, non quello dei due vincolati — dove un nome sbagliato produrrebbe un
artefatto fuori specifica o lascerebbe i companion senza un posto da cui
prendere il nome. `Free` con i suoi `recognised_suffixes` era la dichiarazione
giusta, e ora lo si sa invece di supporlo:
`crates/driver-filegdb/tests/descrittore_pubblico.rs` la fissa, e gira in
entrambe le build perché il descrittore è statico — ciò che la feature
cambia è se il driver sappia lavorare, non che cosa dichiari.

La build con `gdal-backend` conferma anche il resto del perimetro: tutti e dieci
i descrittori portano `recognised_suffixes` e `sink_path`, e `filegdb` risulta
`available: true`.

---

## Che cosa questo piano non fa

Non modifica API, specifiche condivise o la versione del prodotto. Non crea
documenti paralleli: l'unica estensione al docset è l'ammissione di questo file
e il collegamento da `README.md`, e ogni altro controllo resta com'era. Non
tocca tag, artefatti o evidenze della 2.0.0 e della 3.0.0, che restano
verificabili come lo erano il giorno in cui sono stati pubblicati.
