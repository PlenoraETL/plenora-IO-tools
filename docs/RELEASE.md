# Rilascio ? stato corrente

La candidate corrente 10.0.0 e `e4c9762f77d36486fc1f1acb8f5fb9ccfc7c882f`,
congelata e qualificata L2 nel tentativo 2. La candidate `edb482d8` e
ritirata il 18/11 per difetti silenziosi, sostituita da e4c9762f.
Il [verbale di ritiro](../assurance/evidence/ritiro-candidate-1000.json) e la
registrazione del 18/11 restano storia. Il titolare ha autorizzato il rilascio
della candidate corrente il 21/11/2026; tag, gate finale e pubblicazione restano da eseguire.
L evidenza edb482d conserva le misure originali ed e classificata come
registrazione documentale storica nel registro, con digest precedente
alla classificazione conservato in `archiviazione`.

## Stato

<!-- generato da assurance/current-state.json: inizio -->

> Questo blocco è **generato**. La sua autorità è
> [`assurance/current-state.json`](../assurance/current-state.json); modificarlo
> a mano crea la seconda verità che esiste per impedire.

| Campo | Valore |
|---|---|
| baseline documentale | `2fe9b54` |
| ultima qualificata | `e4c9762` |
| revisione misurata | `e4c9762` |
| passi del checkpoint | 110 |
| passi verdi | 110 |
| passi omessi | 0 |
| passi falliti | 0 |
| input di replay | 208 |
| target di replay | 15 |
| crash di replay | 0 |
| target di smoke eseguiti | 15 |
| target di smoke totali | 15 |
| finding di smoke | 0 |
| target in quarantena | 0 |
| copertura LCOV | 90,02% |
| righe coperte LCOV | 37 191 |
| righe strumentate LCOV | 41 313 |
| copertura cargo | 87,90% |
| soglia di copertura | 80,00% |
| baseline differenziale | `7314ca25b62788a3a9d2af0ec638d0d7f3bdffea` |
| esito differenziale | 89.37% |
| gruppi ASSURANCE-N1 | 50 |
| gruppi ASSURANCE-N1 aperti | 0 |
| blocchi | 2 |
| capacità differite | 2 |
| S9, qualificato su | `e4c9762` |
| candidate, versione del manifesto | `10.0.0` |
| candidate, stato | attiva |
| candidate, revisione congelata | `e4c9762f77d36486fc1f1acb8f5fb9ccfc7c882f` |
| candidate, versione del workspace | `10.0.0` |
| candidate, artefatti congelati | 7 |
| candidate, tag previsto | `v10.0.0` |
| candidate, tag creato | no |
| candidate, revisione del tag | nessun tag |
| candidate, tag sulla candidate | no |
| candidate, assurance entro l'allowlist | sì |
| candidate, release_action consentita | no |
| release_authorized | `true` |

I blocchi sono l'elenco esatto dei `release_blocking` del
[registro del contratto corrente](../assurance/registries/release-contract-current.json)
— non un riassunto:

| Blocco | Sintesi |
|---|---|
| `release.candidate-non-valida-per-head` | Candidate e4c9762 qualificata L2 e autorizzata dal titolare: restano tag e condizioni finali di release. |
| `distribuzione.artefatti-qualificati` | Sette artefatti e 32 referti verificati; attestazioni Windows correnti 32/32, manifesto generato; restano attestazioni Linux e confronto bozza. |

Le capacità **differite** non sono blocchi chiusi: non sono richieste
da questa release e **non sono verificate**. Ciascuna dichiara che cosa
la release non promette, ed è la sola lettura autorizzata del rinvio:

| Capacità | Sintesi | La release non promette |
|---|---|---|
| `sistema.qualifica-cross-component` | differita: manca il bundle completo dell owner per le candidate correnti; GEO-005/REJ-001 prevalgono sul gate | La candidate non promette interoperabilita end-to-end qualificata fra i tre componenti. La corsa Linux storica del 13/11/2026 costituisce evidenza parziale, anche nella direzione inversa; non copre la matrice completa, Windows nativo e tutte le superfici. Il bundle corrente manca. I conflitti CRS proibiti devono essere rifiutati secondo GEO-005/REJ-001; solo rappresentazioni lecite sono soggette a conservazione/propagazione. Le prove sintetiche del validatore non sono evidenza della corsa congiunta. |
| `distribuzione.licenza-first-party` | differita: dalla 4.1.1 gli artefatti portano la licenza proprietaria decisa dal titolare; manca la sua conferma esterna | Fino alla 4.1.0 compresa gli archivi non portano termini first-party. Dalla 4.1.1 portano `LICENSE` con la licenza proprietaria; la prova esterna che questo invariante pretende -- `release/licenza-first-party.json`, la conferma del titolare del testo e della propria denominazione legale -- non c'e', e l'invariante resta differito e **non verificato**: il testo negli artefatti non dimostra da solo di essere l'atto del titolare. Le licenze dei componenti di terzi restano incluse e applicabili. Il classificatore Private :: Do Not Upload e mantenuto nei pacchetti Python; nessun pacchetto e pubblicato su PyPI. |

<!-- generato da assurance/current-state.json: fine -->

## Qualifica di famiglia

<!-- generato da release/system-rc-gate.json: inizio -->
### 4. Qualifica cross-component

Stato: `not_satisfied`; evidenza: `not_run`. Owner: `plenora-contracts/conformance`.

ICD adottato: `38d68b402389855edbcecb7ce57c31e91282cb4e`, da `contracts/adoption-source.json`.

Matrice: 112 combinazioni di base, 560 celle con 5 superfici; 15 proprieta per hop.

- `required_directions`: `io_to_data_to_database`, `database_to_data_to_io`
- `required_platforms`: `linux-native`, `windows-native`
- `required_surfaces`: `rust`, `cli`, `python-sync`, `python-async`, `runtime`
- `fixture_variants`: `canonical_only`, `geoarrow_extension`
- `required_fixtures`: `axis_lat_lon`, `conflicting_crs`, `crs_missing`, `crs_unresolved`, `dimensions_unknown`, `geography_semantics`, `int64_beyond_2_53`, `multipolygon_xyzm_srid`, `point_z`, `point_zm`, `polygon_xy_projected`, `types_mixed`, `uint64_ordering`, `unknown_key`
- `required_properties`: `contract_version`, `field_id`, `types_declaration`, `geometry_types`, `dimensions`, `encoding`, `geometry_or_geography`, `precision`, `srid`, `crs_id`, `crs_resolution`, `crs_definition`, `crs_definition_format`, `axis_order`, `native_metadata`
- `required_operations`: `identity`, `derived`, `explicit_plan`

Le classi di ingresso restano distinte:

- `prohibited_crs` (conflicting_crs): GEO-005/REJ-001: reject crs at the first validating boundary; all surfaces, four axes, no effect; preservation skipped with reason.
- `lawful_discrepancy` (crs_unresolved): Only discrepancies not forbidden by the adopted contract: preserve and propagate unless an explicit lawful plan changes them.
- `lawful` (axis_lat_lon, crs_missing, dimensions_unknown, geography_semantics, int64_beyond_2_53, multipolygon_xyzm_srid, point_z, point_zm, polygon_xy_projected, types_mixed, uint64_ordering, unknown_key): Identity preserves properties byte-for-byte; derived and declared transformations require an independent expected result and localized reports.

Condizioni del gate:

- The adopted normative contract takes precedence: lawful declarations and non-prohibited discrepancies are tested separately from prohibited CRS conflicts.
- Lawful inputs: identity preserves every property byte-for-byte; derived fields are reconstructed from the actual result; explicit lawful plans and unavoidable transformations have localized reports and independently verified expected output.
- GEO-005/REJ-001 prohibited conflicts: each component rejects at its first public validating boundary with category crs, four error axes, remote_effect none and matching before/after destination snapshots, on every surface.
- Preservation and propagation after a prohibited rejection are skipped with reason, never counted as passed.
- All three components, ICD and executed harness have immutable revisions backed by Git commit objects; executed artifacts are hashed and bound to platform, surface, command and run.
- All required fixtures, variants, directions, native platforms, surfaces and properties have complete evidence; not_run, partial, failed and unrelated artifacts cannot satisfy the gate.
- The owner bundle binds manifest, lock, actual Arrow input/output/expected observations, loss reports, commands, environment, logs and absence-of-effect evidence by SHA-256.

Bundle v2: validatore `scripts/system_qualification.py`.

SHA-256 of every referenced artifact; Git commit object identities; complete per-cell execution and property comparisons.

Offline consistency and byte comparisons do not authenticate the owner, prove execution, or establish membership of a Git object in a remote repository.

Il puntatore `evidence.bundle` contiene percorso e SHA-256 del bundle indice in `assurance/evidence/`. Ogni artefatto referenziato deve essere presente offline, non vuoto, hashato e usato. Il bundle include lock, manifest, report, comandi, ambienti e indice fixture; il lock lega oggetti Git di componenti/ICD/harness e artefatti eseguiti a piattaforme e superfici. Le osservazioni fixture/output/expected sono file Arrow IPC: metadati confrontati byte per byte e fixture CRS in WKT, WKT2 o PROJJSON, con identificatori alla radice verificati dall oracolo al pin adottato. I payload WKB/EWKB devono esercitare realmente tipi, dimensioni e SRID richiesti. I valori sono confrontati dopo serializzazione Arrow canonica. Le operazioni identitarie si confrontano anche con la sorgente; derivazioni e trasformazioni richiedono attese e rapporti localizzati. I log riproducono il comando osservato; le prove CRS dei tre componenti restano indipendenti e referenziano errori e stato completo della destinazione prima/dopo, senza propagare il conflitto proibito.

Vocabolario, parser WKB/EWKB, CRS e primaria derivano dagli oggetti Git del contratto adottato, disponibili offline in `PLENORA_CONTRACTS` o `.plenora-contracts`; un checkout modificato non cambia il pin. Le varianti mantengono entrambe `geoarrow.wkb`: cambia soltanto `ARROW:extension:metadata`. Le fixture intere non richiedono geometria; uint64 parte da valori non ordinati. I requisiti di campagna sono dichiarati in `fixture_requirements` e `variant_rules`, separati dal vocabolario normativo. Ogni comando positivo porta `semantics` nel log: identificatore pubblico e kind; un `geometry_compute` attesta geometry_selection e produced_geometry_ids. Il validatore verifica operandi PRIM-007, applicabilita CRS GEO-006/015/019/020, precisione GEO-014 e transizione della primaria PRIM-005. requested_primary o replaces_primary_with dichiarano una richiesta o un contratto di sostituzione espliciti; sort e pass-through non attestano una computazione geometrica. Un calcolo fuori dal dominio decidibile dall oracolo non e qualificato e non rende invalido il relativo ingresso. L audit enumerato e `release/system-qualification-audit.json`.

Il formato dettagliato e gli esempi sintetici di validazione sono nei moduli `scripts/system_qualification.py` e `scripts/test_system_qualification.py`. Questi test non attestano una corsa congiunta. Il bundle completo dell owner resta necessario: `verified` con evidenza non superata e `differita` con evidenza completa superata sono incoerenti.

Blocchi aperti:

- No complete owner bundle for the current three-component candidates and adopted ICD has been delivered.
- Native Windows execution of the complete matrix is not qualified.
- The full property matrix, derived operations, lawful explicit plans and prohibited CRS rejection probes remain to be qualified in both directions on all surfaces.

Historical Linux evidence from 13 November 2026 is partial and does not satisfy this gate; the inverse direction is no longer described as never executed. No historical result is promoted.

<!-- generato da release/system-rc-gate.json: fine -->

## Preparazione della release

Congelare sorgenti e artefatti solo dopo le verifiche della candidate finale.
Il registro `assurance/registries/release-contract-current.json` governa gli
invarianti e le condizioni congiunte; identita, digest, pin e autorizzazione
restano obbligatori. Eseguire il gate di release e la verifica degli artefatti
prima della pubblicazione, poi verificare i byte pubblicati.

Vedi [Ingegneria](ENGINEERING.md) per le prove e [Installazione](INSTALL.md)
per i profili di distribuzione.

#### Installazione, aggiornamento, rollback e recovery

Gli archivi sono installazioni **affiancate**, non aggiornamenti in-place. La
directory estratta è immutabile: modificarne un file invalida manifesto,
checksum e qualifica.

**Installazione.** Scaricare insieme archivio, file `.sha256` e provenance;
verificare il checksum prima di estrarre. Su Linux, dalla directory che contiene
entrambi, usare `sha256sum -c <file>.sha256`. Su Windows confrontare il valore
nel sidecar con `(Get-FileHash -Algorithm SHA256 <file>).Hash.ToLowerInvariant()`:
stampare l'hash senza confrontarlo non è una verifica. Estrarre in una directory
temporanea sullo stesso filesystem della destinazione e verificare i digest:

```sh
python3 scripts/check-digest-manifesto.py --albero <directory-estratta>
```

Controllare che `MANIFEST.json` riporti versione, piattaforma, profilo e canale
attesi. Eseguire `<directory-estratta>/bin/plenora-io[.exe] --version` e `catalog`;
per una candidate usare inoltre i referti dello smoke prodotto dal workflow,
non sostituirli con una prova nell'albero sorgente. Solo dopo queste verifiche
rinominare la directory temporanea col nome definitivo. Non estrarre mai sopra
una versione già presente.

Su Windows controllare che `MANIFEST.json` dichiari
`firma.stato: non_richiesta`. L'assenza di Authenticode è intenzionale; chi
opera sotto una policy che ammette soltanto editori firmati deve distribuire il
binario attraverso un canale aziendale approvato oppure non installarlo.

**Attivazione e aggiornamento.** Il programma non installa un servizio e non
gestisce un collegamento `current`: il supervisore del deployment deve puntare
al percorso assoluto del binario scelto. Per aggiornare, installare la nuova
versione affiancata, verificarla, fermare le nuove invocazioni, cambiare quel
solo percorso e riavviare. Conservare la directory precedente fino alla fine
della finestra di osservazione. Profilo e versione sono parte dell'identità:
passare da `base` a `filegdb` non è una sostituzione trasparente.

**Rollback.** Fermare le nuove invocazioni, ripristinare nel supervisore il
percorso assoluto della directory precedente, riavviare e ripetere `--version`
e `catalog`. Non copiare file dalla nuova installazione nella vecchia e non
riusare il runtime GDAL di un'altra versione. La CLI è stateless: il rollback
del binario non migra né ripara gli output già pubblicati.

**Recovery dei dati.** Una busta con `remote_effect: none` non ha lasciato una
destinazione visibile e il comando può essere ripetuto dopo aver corretto la
causa. Con `remote_effect: partial` e `retry: requires_recovery` non rilanciare
alla cieca: isolare la destinazione, inventariare i companion già visibili e
applicare la procedura Shapefile di `docs/PRODUCT.md` prima di ritentare. Dopo
un kill forzato vale la stessa cautela, perché non esiste una busta che possa
certificare lo stato. Gli spool non richiedono pulizia: sono file senza nome;
lo staging ordinario viene rimosso al rientro cooperativo, ma non va assunto
dopo la terminazione forzata.


### 6. Decisione finale di rilascio

La decisione esplicita del titolare riguarda soltanto gli artefatti qualificati della candidate corrente.


### Registrazione distribuzione, replay finito e L2 della 10.0.0 - 2026-11-18

**Storica: ritirata il 18/11 per difetti silenziosi, sostituita da e4c9762f.**

La candidate congelata e distribuita e
`edb482d892da11ecb421d5441079209a65d61fa0`. La qualifica riguarda questo SHA,
non il futuro commit di registrazione. Nessun tag o permesso di pubblicazione
viene creato. Precedente applicato: `a6a334d`, registrazione della 9.0.0.

L'[evidenza](../assurance/evidence/checkpoint-edb482d.json) e generata da
`scripts/registra-evidenza-s9.py` sui 116 file originali in
`target/rel1000/io/target/rel1000/l2-edb482d892da11ecb421d5441079209a65d61fa0-attempt-1/`.
Il registratore usa la copia byte per byte dei 115 file di passo in
`target/rel1000/registrazione/corsa-contratto/`. Il manifesto ne conserva gli
SHA-256; `console.log` resta fra le fonti esterne, per il gate congelato.
`replay.log`, `esiti.txt` e la
ricevuta esterna hanno digest separati in `provenienza_della_corsa`.
Unico tentativo, nessuno scartato, secondo l'operatore.

| Misura | Valore | Fonte nella directory L2 |
|---|---|---|
| Passi distinti/verdi | 110/110; 0 omessi, falliti, duplicati | `risultato.json`; riscontro `console.log` |
| Albero | 0 file sporchi; revisione e impronta invariate | `risultato.json` |
| Replay | 207 input, 15 target, 0 crash | `fuzz_replay.log` |
| Verifica ricevuta | 15/15 target, 0 finding, 0 quarantena | `fuzz_smoke.log`, `fuzz-smoke-ultima.json` |
| LCOV | 32 224/36 247 righe, 88,90% | `coverage_soglia_dal_report.log` |
| Cargo | 86,81%; soglia 80% | `coverage_soglia_controprova.log` |
| Differenziale | 29/29 coperte, 0 scoperte, 100,00%; 146 non eseguibili | `coverage_diff.log` |

Impronta iniziale e finale:
`3ef0f10e2187fe1b3c7c91224f4abb3fefe51dbf442ad7f67918f74118e91bbd`.
Baseline differenziale: `7314ca25b62788a3a9d2af0ec638d0d7f3bdffea`, ultimo
checkpoint superato (9.0.0). LCOV e cargo contano proiezioni diverse dello
stesso profdata; il differenziale riguarda solo le righe cambiate di libreria.

Ambiente dichiarato dall'operatore: VM Ubuntu 24.04, 192.168.2.140, clone
pulito della candidate, `.plenora-contracts` al pin
`38d68b402389855edbcecb7ce57c31e91282cb4e`; Python 3.14.6, pyarrow 25.0.1,
PyYAML 6.0.3, openpyxl 3.1.5, jsonschema 4.26.0, come nei workflow.
`S9_CHECKPOINT_BASE` indica la baseline sopra; `S9_CI_FUZZ_RECEIPT` la ricevuta
in `target/rel1000/io/target/rel1000/replay-edb482d892da11ecb421d5441079209a65d61fa0/`.
La ricevuta L2 aggiunge solo `riuso`: `source_sha256` verificato sui byte della
ricevuta esterna, tutti gli altri campi identici. Policy `io-9-replay-v1`:
`fuzz_smoke` verifica la ricevuta, non esegue una seconda campagna; nessun soak.
Prima del replay l'operatore dichiara verdi `check_profondita_fuzz.py --qualifica`
e `check_asan_filegdb.py --qualifica`: quattro profondita valide; i 181833
contatori ASan hanno riscontro in `target/rel1000/replay.log`.
Profondita e confine ASan restano prove separate: il replay Rust non qualifica
l'interno del GDAL di sistema non strumentato.

Distribuzione GitHub Actions `44222062212`, canale `candidate`, esito `success`
e revisione sopra comunicati dall'operatore; nessuna consultazione GitHub.
I download originali in `target/rel1000/distribuzione-44222062212/` sono immutati.
I sette artefatti ritirati, archiviati in `candidate_ritirata.artefatti` del
verbale di ritiro, riportano nome, dimensione
e SHA-256 ricalcolati dai download, con revisione verificata da provenance e
manifesto sorgente. `check-deliverable.py` passa sui sette archivi, inclusi i
due Python; `check-distribuzione-completa.py` passa sui 32 referti attesi e trovati.
Fonti, dimensioni, digest e comandi sono in `distribuzione_candidate` dell'evidenza.
Questa verifica non e il confronto dei byte della bozza o della release pubblicata.

La candidate descritta in questa registrazione e ritirata. Prima di riprendere
quei passi occorrono le correzioni R1..R5, un nuovo congelamento 10.0.0 e nuove
qualifiche e artefatti. La bozza e gli archivi di questa corsa non possono essere
riutilizzati per autorizzare il nuovo rilascio.
I due blocchi restano espliciti; `release_authorized` e
`release_action_allowed` restano `false`. La qualifica cross-component e la
conferma esterna della licenza first-party restano differite e non verificate.
Comandi operativi e resoconto: `target/REGISTRA-1000.md`.

Il checkpoint storico 9.0.0 resta nella storia Git e nella copia locale
`target/rel1000/registrazione/checkpoint-7314ca2.json`; viene rimosso dalla
directory corrente secondo il vincolo di insieme esatto del gate congelato.


### Registrazione distribuzione e L2 della 10.0.0 - 2026-11-21

Candidate: `e4c9762f77d36486fc1f1acb8f5fb9ccfc7c882f`. Precedente applicato:
registrazione 9.0.0 `a6a334d`; nessun commit, tag o pubblicazione creato.
L'[evidenza](../assurance/evidence/checkpoint-e4c9762.json) e generata con
`scripts/registra-evidenza-s9.py` dal tentativo 2. Le fonti originali in
`target/rel1000/` restano immutate; i log di passo sono copiati byte per byte
in `target/rel1000/registrazione/corsa-contratto/`. `console.log` e hashato
separatamente nella provenienza, come nel precedente del 18/11.

Tutti e tre i tentativi sono dichiarati con percorso, esito e SHA-256 di ogni
file nel `registro_dei_tentativi`, richiamato dalla provenienza:

- Tentativo 1: 109/110, scartato per **AMBIENTE**. Solo `sonde_sdk_python`
  fallisce: pyarrow 26.x nell'ambiente, mentre l'SDK e qualificato con
  pyarrow 25.x. La causa ambientale e dichiarata dall'operatore; il risultato
  rosso originale resta conservato, senza valore di qualifica.
- Corsa `interrotto-0106`: interrotta dal riavvio della sessione a meta;
  `risultato.json` resta `in_corso`, senza valore di qualifica.
- Tentativo 2: 110/110, `S9 checkpoint level 2 passed`; unica fonte delle
  misure correnti. Il registratore supporta gia il registro dei tentativi
  nel giudizio: nessun vincolo di unico tentativo e nessuna estensione al codice.

Ambiente dichiarato: **WSL Ubuntu 26.04, utente non root**, anziche la VM
usata per la candidate storica. `.plenora-contracts` al pin
`38d68b402389855edbcecb7ce57c31e91282cb4e`;
`S9_CHECKPOINT_BASE=7314ca25b62788a3a9d2af0ec638d0d7f3bdffea` (9.0.0).
Registrazione locale con Python 3.14.6 dal venv `C:/pg/io-fix1000/target/r3-venv`.

| Misura | Esito |
|---|---|
| Passi distinti/verdi | 110/110; 0 omessi, falliti o duplicati |
| Albero misurato | pulito; revisione e impronta invariate |
| Replay | 208 input, 15 target, 0 crash |
| Ricevuta fuzz | 15/15 target; 0 finding e quarantena; nessun secondo soak |
| LCOV | 37 191/41 313, 90,02% |
| Cargo | 87,90%; soglia 80% |
| Differenziale | 5 994/6 707, 89,37%; 713 scoperte; 15 396 non eseguibili |

Distribuzione GitHub `44782545519`, canale `candidate`, verde secondo
l'operatore. I sette archivi hanno dimensioni e SHA-256 ricalcolati, checksum
e revisione verificati da `check-deliverable.py`; `check-distribuzione-completa.py`
verifica 32/32 referti. Inventario completo dei file e digest in
`distribuzione_candidate` dell'evidenza. Verifica offline dei download,
nessuna verifica API o confronto dei byte della bozza/release pubblicata.

**Attestazioni:** nei download forniti non risultano attestazioni del profilo
pubblico con `conforme`/`esigente`; le quattro gia versionate datano 18/11 e
hanno i digest della candidate ritirata. Restano storiche, non qualificano
questi archivi. Occorrono quelle della corsa corrente e il manifesto di adozione.

Il cancello di verifica, senza pubblicazione, e:

```powershell
& C:/pg/io-fix1000/target/r3-venv/Scripts/python.exe scripts/check_release_contract.py --release
```

`release_authorized` resta `false`: la decisione scritta spetta al titolare.
Restano inoltre il confronto della bozza, il tag e tutte le condizioni
congiunte del gate. Esito e diagnostica della verifica locale sono riportati
in `target/REGISTRAZIONE-1000.md` e nei log sotto `target/rel1000/registrazione/`.
Il comando integrale e stato eseguito e interrotto senza verdetto completo.
Le nove condizioni congiunte, eseguite separatamente con le funzioni dello
stesso gate, danno esito bloccato: decisione, tag e manifesto mancanti,
due invarianti aperti, e controllo ASan locale non eseguibile senza
`gdal_i.lib`. Il risultato L2 WSL resta distinto da questa verifica locale.


### Decisione del titolare 10.0.0 - 2026-11-21

Decisione esplicita del titolare del 21/11/2026: «si procedi», dopo L2 110/110, distribuzione 44782545519 verde e registrazione e4646ed1. Fonte: assurance/evidence/autorizzazione-10.0.0.json. Autorizzazione registrata; attestazioni Windows correnti 32/32 per profilo e manifesto generati offline. Restano attestazioni Linux correnti, confronto bozza, tag e gate finale; pubblicazione e verifica dei byte pubblicati non eseguite. release_action_allowed resta false fino alla registrazione delle condizioni finali.

Questa decisione supera le indicazioni precedenti di autorizzazione assente;
i verbali storici restano invariati. Tag non creato e nessuna pubblicazione
registrata. I due blocchi restano aperti per le condizioni non ancora eseguite.
Le attestazioni Linux del 18/11 sono storiche e non qualificano questa candidate.
Comandi per Linux e pubblicazione: `target/rel1000/attestazioni-linux.sh` e
`target/rel1000/PUBBLICAZIONE.md`.
