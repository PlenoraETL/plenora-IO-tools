# Rilascio — stato corrente

La candidate 10.0.0 e aperta, non congelata e non autorizzata.
La decisione del titolare del 15 e 17 novembre 2026 elimina la compatibilita
con release Plenora precedenti. Le cinque rimisure di profondita fuzz e confine
ASan FileGDB, fornite dalla VM di qualifica sul perimetro di d196f7a621b76b91b4d19b85ea7ca6abc7b31f68, sono valide
per il perimetro corrente. Checkpoint e replay sotto conservano gli SHA originali
e non qualificano la 10.0.0. Il congelamento avverra sul commit di merge della PR
di preparazione; distribuzione, replay completo e L2 restano da eseguire.

## Stato

<!-- generato da assurance/current-state.json: inizio -->

> Questo blocco è **generato**. La sua autorità è
> [`assurance/current-state.json`](../assurance/current-state.json); modificarlo
> a mano crea la seconda verità che esiste per impedire.

| Campo | Valore |
|---|---|
| baseline documentale | `2fe9b54` |
| ultima qualificata | `7314ca2` |
| revisione misurata | `7314ca2` |
| passi del checkpoint | 112 |
| passi verdi | 112 |
| passi omessi | 0 |
| passi falliti | 0 |
| input di replay | 207 |
| target di replay | 15 |
| crash di replay | 0 |
| target di smoke eseguiti | 15 |
| target di smoke totali | 15 |
| finding di smoke | 0 |
| target in quarantena | 0 |
| copertura LCOV | 88,87% |
| righe coperte LCOV | 32 418 |
| righe strumentate LCOV | 36 480 |
| copertura cargo | 86,79% |
| soglia di copertura | 80,00% |
| baseline differenziale | `6e494a6ff7c31bfb0f9abd664d08b9f8fe2a97c0` |
| esito differenziale | 96.42% |
| gruppi ASSURANCE-N1 | 50 |
| gruppi ASSURANCE-N1 aperti | 0 |
| blocchi | 1 |
| capacità differite | 2 |
| S9, qualificato su | `7314ca2` |
| candidate, versione del manifesto | `10.0.0` |
| candidate, stato | iniziale |
| candidate, revisione congelata | non congelata |
| candidate, versione del workspace | `10.0.0` |
| candidate, artefatti congelati | non congelati |
| candidate, tag previsto | `v10.0.0` |
| candidate, tag creato | no |
| candidate, revisione del tag | nessun tag |
| candidate, tag sulla candidate | no |
| candidate, assurance entro l'allowlist | non derivabile senza congelamento |
| candidate, release_action consentita | no |
| release_authorized | `false` |

I blocchi sono l'elenco esatto dei `release_blocking` del
[registro del contratto corrente](../assurance/registries/release-contract-current.json)
— non un riassunto:

| Blocco | Sintesi |
|---|---|
| `distribuzione.artefatti-qualificati` | Candidate 10.0.0 iniziale: distribuzione, artefatti congelati e confronto dei byte della bozza ancora da eseguire. |

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
