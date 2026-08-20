# Change impact analysis — S9 tranche 2: `plenora-io-core` redatto

Data: 2026-08-20. Sigla: **S9 / tranche 2**.
Baseline: `2b38c4e` (tranche 1, `plenora-io-model`).
`plenora-io-error-v1` **invariato**.

## Problema

INV-10 vieta che un errore pubblico porti testo costruito a runtime — dal
payload, da una dipendenza, da un valore letto. La tranche 1 ha chiuso
`plenora-io-model` e ha lasciato in piedi la via legacy per gli altri crate.

Il censimento autorevole (`scripts/check_errori_redatti.py`) contava **226** usi
legacy di produzione in dodici crate, e **78** erano in `plenora-io-core` — un
terzo del debito in un crate solo. Finché core costruisce errori con `format!`,
ogni driver che gli passa attraverso eredita la stessa via.

## Cosa cambia

`plenora-io-core` passa **interamente** alla via redatta: `PublicMessage`,
`ErrorContext`, `ContractIdentifier`. Il registro scende **226 → 148**, e il
crate entra in `MIGRATI` accanto a `plenora-io-model`.

### I 78 usi diretti

| File | usi legacy | di cui `format!` | di cui valori interpolati | di cui testo di dipendenza |
|---|---|---|---|---|
| `driver.rs` | 39 | 9 | 11 | 0 |
| `driver/reader_adapters.rs` | 20 | 3 | 2 | 0 |
| `publish.rs` | 10 | 2 | 0 | 0 |
| `driver/batch_worker.rs` | 4 | 0 | 2 | 0 |
| `capabilities.rs` | 2 | 1 | 1 | 0 |
| `driver/spool.rs` | 2 | 0 | 2 | 0 |
| `request.rs` | 1 | 1 | 0 | 0 |
| **totale** | **78** | **13** | **22** | **0** |

Le tre colonne non sono sinonimi e vanno lette separate: «usi legacy» conta le
chiamate ai costruttori storici, `format!` conta quelle che costruivano il
messaggio interpolando, «valori interpolati» conta quelle che facevano entrare
nel testo un valore — indice, conteggio, nome, enum. Il testo di dipendenza in
core era **già zero**: quel debito sta nei driver.

### I 30 che il censimento non vedeva

`capabilities.rs::violation` (23 chiamate) e `driver.rs::geometry_violation`
(7) avevano firma `detail: impl Into<String>` e `field: &str`. Non sono
`PlenoraIoError::…`, e il gate non li contava — ma erano la stessa via aperta al
testo libero un livello più in basso.

**«Zero chiamate dirette» non sarebbe bastato a chiudere il crate.** Le firme
sono ora `detail: &PublicMessage` e `field: Option<&ContractIdentifier>`.

È anche una lezione sul gate: conta i costruttori, non gli helper che li
avvolgono. Per le tranche successive va guardato a mano se il crate ha una
funzione di comodo con `impl Into<String>`.

## Errata S9.1 — tre aggiunte ratificate

Registrate nel Decision Package (§ *Errata S9.1*) e nel design (§ 18).

1. **Sei costruttori redatti** (`non_supportato_redatto`, `schema_redatto`,
   `crs_redatto`, `formato_redatto`, `capability_redatta`,
   `destinazione_esistente`) che rispecchiano uno a uno i cinque assi e il
   `IoErrorCode` dei costruttori storici. Senza, una svista in uno di 78 siti
   avrebbe cambiato la categoria sul wire in silenzio.
2. **`PublicMessage::CuratedPair(&'static str, &'static str)`** — due statici da
   vocabolari chiusi, la stessa garanzia di `Curated` applicata due volte.
   L'unica variante con testo runtime resta `OpzioneRifiutata`.
3. **`nome()`** su `GeometryEncoding`, `CoordinateDimensions`,
   `SpatialSemantics`, `ArrowTypeClass`, più
   `ContractIdentifier::from_geometry_column`. Sostituiscono i `{:?}`: `Debug`
   non è un formato che qualcuno abbia promesso di tenere stabile.

## Cosa esce dai messaggi

| Informazione | Prima | Ora | Esito |
|---|---|---|---|
| nome del campo (schema) | interpolato | `ContractIdentifier::from_schema_field` nel campo `field` | spostato |
| nome della colonna geometrica | interpolato | `ContractIdentifier::from_geometry_column` nel campo `field` | spostato |
| driver nei rifiuti di scrittura | interpolato | era già nel campo `driver` | spostato (era un duplicato) |
| nome del layer del `WritePlan` | interpolato | indice del layer nel piano | trasformato |
| CRS atteso / CRS dichiarato | interpolati | assenti | **perduti dal messaggio**, leggibili da capability e piano |
| SRID payload / SRID dichiarato | interpolati | assenti | **eliminati**: numeri letti dal payload |
| classi Arrow, encoding, dimensioni, semantica, tipo geometrico | `{:?}` | `nome()` / `canonical_name()` | invariati nella sostanza, stabili nella forma |

`WriteLayer` non è un `LayerContract`, quindi `ContractIdentifier::from_layer`
non si applica ai layer del `WritePlan`. **Non è stata aperta** una via di
costruzione per nome nudo: avrebbe tolto al tipo l'unica proprietà che ha — che
il nome venga da un contratto validato — per comodità di due siti. Il layer si
identifica per indice, che è un numero strutturale vero.

## Impatto sui consumatori

**Il testo di `message` cambia** in tutti i siti migrati. È la rottura già
ratificata (decisione 2 di S9): i consumatori devono usare
`(category, phase, code, retry)`, e `message` non è una chiave di
compatibilità.

**Lo schema di `plenora-io-error-v1` non cambia.** Nessun campo aggiunto, tolto
o rinominato; `field` continua a portare una stringa, che ora nasce da un
`ContractIdentifier` invece che da un `&str` qualunque. Nessuna doppia
emissione.

**Categoria, fase, effetto remoto, retry e `IoErrorCode` sono preservati sito
per sito**: è per questo che i sei costruttori redatti esistono, invece di
lasciare che ogni chiamante li ridichiari.

## Rischio residuo

Il gate `check_errori_redatti.py` conta i costruttori di `PlenoraIoError`.
**Non vede** un helper interno che accetti `impl Into<String>` e li avvolga —
è esattamente ciò che è successo con `violation` e `geometry_violation`. Il
gate resta corretto per quello che misura; l'ispezione degli helper resta
manuale, una volta per crate, e va fatta prima di dichiarare migrata una
tranche.

## Verifica

* `scripts/check_errori_redatti.py`: **148** residui in dodici crate;
  `plenora-io-model` e `plenora-io-core` migrati e a **zero**;
* sonde del censimento: 7/7 su albero finto, in entrambi i versi;
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: pulito;
* `cargo test --workspace --all-features`: verde;
* `check_assurance_fallbacks.sh`: `plenora-io-core` resta a **16** fallback
  registrati. La prima stesura ne aveva introdotti tre
  (`unwrap_or(u64::MAX)` per convertire `usize`); sono stati sostituiti con
  `driver::saturating_u64`, che esisteva già — il registro non è stato alzato
  per far passare il gate;
* nuovi test: `CuratedPair` in contesto `const`, rendering sotto
  `MAX_MESSAGE_BYTES`, quattro doctest `compile_fail` su argomenti runtime;
  esaustività e distinzione dei vocabolari `nome()`; rifiuto — non troncamento
  — dei nomi non attestabili in `from_geometry_column`.

## Prossimo passo

Tranche 3: **solo** `driver-common`, 10 usi legacy di produzione nel registro
autorevole.

---

## Addendum del 2026-08-21 — un'affermazione di questo documento e' falsa

**Il corpo resta com'era.** La regola append-only serve proprio a questo: chi
rilegge deve poter distinguere cio' che si sapeva allora da cio' che si e'
capito dopo. L'affermazione sbagliata resta scritta, e qui viene marcata.

### L'affermazione

Nella sezione *Impatto sui consumatori*:

> **Categoria, fase, effetto remoto, retry e `IoErrorCode` sono preservati sito
> per sito**: e' per questo che i sei costruttori redatti esistono, invece di
> lasciare che ogni chiamante li ridichiari.

**E' falsa per un sito**, in `crates/plenora-io-core/src/driver.rs`, nel
controllo del batch contro il contratto dichiarato.

### Che cosa e' successo

| | prima | dopo |
|---|---|---|
| category | Schema | Schema |
| phase | Validate | Validate |
| remote_effect | None | None |
| retry | Never | Never |
| **code** | **`Generic`** | **`Schema`** |

Il sito usava `PlenoraIoError::new(ErrorCategory::Schema, ErrorPhase::Validate,
…)`. `new` non nomina il codice: lo mette a `Generic`. Convertendolo in
`schema_redatto`, che impone `code = Schema`, **il codice sul wire e' cambiato**.

`code` fa parte della chiave di compatibilita' ratificata dalla decisione 2 di
S9, insieme a category, phase e retry. Non era quindi un dettaglio del refactor:
era una rottura, introdotta senza accorgersene e dichiarata assente.

### Perche' non si e' visto

Nel diff **non compare una sola riga** di `ErrorCategory::`, `ErrorPhase::`,
`RemoteEffect::` o `RetryDisposition::` cambiata: quelle righe sono sparite
insieme alla chiamata a `new`, e il costruttore nuovo non le nomina. Un controllo
sulle varianti enum -- che era la mia evidenza -- non poteva vederlo.

Trovato il 2026-08-21 costruendo la matrice dei gruppi differenziali, cioe' da
un lavoro fatto per un'altra ragione.

### Correzione

* codice: `6da790b` ripristina `redatto(IoErrorCode::Generic, ErrorCategory::Schema, …)`;
* gate: `scripts/check_quartetto_sito.py` (`b99289b`, corretto in `7593faa`)
  verifica lo snapshot dei quartetti per `percorso::funzione`, sul codice
  presente e non su un diff -- cosi' resta valido quando i costruttori legacy
  spariranno;
* nessun altro sito e' interessato: la verifica sull'intero perimetro di S9 non
  ne trova altri.

`Schema` sarebbe piu' preciso di `Generic` per una discordanza di schema, ma
renderlo tale e' una decisione da ratificare separatamente, non una conseguenza
di un refactor sui messaggi.

