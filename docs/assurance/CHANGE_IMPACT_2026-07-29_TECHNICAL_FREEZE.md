# Change impact analysis — freeze tecnico della baseline

Data: 2026-07-29

## Decisione

La revisione
`78c2d150b9c7d0ac48e4c97b03f86228e0f0a068` viene congelata come baseline
tecnica immutabile del componente IO-tools.

Il freeze registra quale codice non deve più cambiare in-place. Non afferma che
la revisione indipendente sia avvenuta, non autorizza un tag e non promuove il
claim oltre `verified_internally`. I tre stati restano distinti e
machine-readable:

- baseline tecnica: `frozen`;
- revisione indipendente: `false`;
- autorizzazione di release e tag: `false`.

Qualunque modifica al codice distribuibile dopo questa decisione richiede un
nuovo candidato, una nuova revisione completa e una nuova CIA; non può
sostituire la revisione congelata conservandone l'identità.

## Base normativa e processo locale

`plenora-contracts@v2.0-rc8`, revisione
`62b12e3496466d2c908dac3cc098640b99b52e21`, dichiara §0 come `proposta`.
R0.4 richiede che una modifica al codice di confine riceva una revisione da una
persona diversa dall'autore e, in sua assenza, limita il massimo stato
dichiarabile a `verificato internamente`.

Il processo locale collegava in precedenza tale review anche al freeze della
configurazione. Questa CIA separa i concetti senza derogare alla sostanza di
R0.4: il codice può essere reso immutabile mentre il relativo claim resta
`verified_internally`. La revisione indipendente continua a essere necessaria
prima di un claim indipendente o di un tag di release.

L'assistente che ha implementato parte del candidato non è registrato come
revisore: è coautore sostanziale e non è una persona eleggibile ai sensi di
R0.4.

## Evidenza della baseline

- revisione congelata: `78c2d150b9c7d0ac48e4c97b03f86228e0f0a068`;
- CI candidata: `30415766905`, verde su `rust`, `windows`, `coverage` e
  `macos-publish`;
- coverage librerie: 12.769/15.271 linee, 83,62%;
- artifact LCOV: ID `8710097703`, digest
  `sha256:f5473d8c3e55fcaecf54ff5134872157c94351686356c7fd3db3928c90b701ab`;
- evidenza e gate di provenienza: commit `aefec48`, CI `30416387715` verde;
- fuzz strutturato della campagna: 28.900.000 iterazioni, zero finding;
- benchmark A/B: GeoJSON read +6,59%, CSV write +12,59%, GeoParquet write
  +48,64%, nessuna coppia oltre il veto del −5%.

## Modifiche

- `contract-provenance.json` passa a `freeze_status=frozen`, registra il
  perimetro `technical_baseline` e mantiene la review `not_performed`;
- `freeze-readiness.json` passa a `frozen_with_open_assurance_gates`, aggiunge
  `technical_baseline_frozen=true` e mantiene `release_authorized=false`;
- il bundle di evidenza registra separatamente decisione di freeze, review e
  tag;
- il gate rifiuta rollback a `pre_freeze`, promozione assurance o tag mentre la
  review è aperta;
- la documentazione distingue freeze tecnico, verifica indipendente e release.

Nessuna modifica interessa codice distribuibile, wire contract, formati,
dipendenze, capability, toolchain o dati.

## Hazard

- H-07: lo SHA congelato identifica una baseline immutabile; un nuovo codice
  non può riutilizzarne l'identità.
- H-08: l'assenza della review resta visibile e impedisce claim o tag più forti.
- H-09: la variazione del processo locale è registrata e verificata da test
  negativi.

## Criteri di verifica

Devono passare:

- gate del contratto release e relativi test negativi;
- gate di pin, identità, dipendenze e fallback;
- formatting, Clippy safety, test workspace e build release;
- CI Linux, Windows, macOS e coverage sul commit del freeze.

## Residui

- revisione indipendente non eseguita;
- tag RC non creato;
- nessuna dichiarazione di RC di sistema o certificazione avionica;
- campagna lunga coverage-guided, MC/DC, object-code coverage e qualificazione
  strumenti ancora aperte.
