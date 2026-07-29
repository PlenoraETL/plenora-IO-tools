# Change impact analysis — riallineamento del candidato pre-freeze

Data: 2026-07-29

## Scopo

Il candidato di componente viene riallineato dalla revisione storica
`92be3f4cd9a84b4dffbfd8b1621cc85a6ec9aa7a` alla revisione finale
`78c2d150b9c7d0ac48e4c97b03f86228e0f0a068`, che comprende la terza campagna
prestazionale e il test deterministico dell'ordine dello schema GeoJSON.

Questa modifica aggiorna esclusivamente provenienza, evidenza e gate
machine-readable. Non modifica codice distribuibile, wire contract, ICD,
dipendenze, capability o formato su disco. Lo stato resta `pre_freeze`;
`independent_review` e `release_tag` restano `false`.

## Evidenza candidata

- CI: `30415766905`, conclusione `success`;
- job verdi: `rust`, `coverage`, `windows`, `macos-publish`;
- artifact: `rust-coverage-lcov`, ID `8710097703`, 94.172 byte;
- digest artifact pubblicato dall'API GitHub:
  `sha256:f5473d8c3e55fcaecf54ff5134872157c94351686356c7fd3db3928c90b701ab`;
- coverage librerie riprodotta con `cargo-llvm-cov 0.8.7` e lo stesso filtro
  della CI: 12.769/15.271 linee, 83,62%;
- soglia fail-closed: 80%.

Il digest proviene dal campo `digest` dell'API GitHub Actions, non dal nome
dell'artifact. Le credenziali CLI locali non consentivano il download
dell'archivio; questo limite è dichiarato invece di sostituire il digest con un
valore ricostruito.

## Gate rafforzato

`check_release_contract.py` fissa e verifica congiuntamente:

- SHA completo del candidato;
- run CI e SHA della run;
- identità, dimensione e digest SHA-256 dell'artifact;
- numeratore, denominatore e percentuale della coverage.

Un test negativo altera separatamente run, digest e conteggio delle linee e
richiede che ogni deriva venga rifiutata.

## Hazard e limiti

- H-07/H-09: revisioni ed evidenze non possono essere aggiornate
  indipendentemente senza far fallire il gate.
- H-08: la CI copre il test che stabilizza l'ordine alfabetico dello schema
  GeoJSON; il fuzz resta un complemento e non una prova di compatibilità fra
  revisioni.
- La coverage è line coverage, non branch coverage, MC/DC o object-code
  coverage.
- Gli strumenti di verifica non sono qualificati DO-330.
- La RC resta del solo componente e non costituisce certificazione
  DO-178C/ED-12C.

## Decisione

Il candidato può restare in stato
`candidate_ci_passed_pending_independent_review`. Non può passare a `frozen`
né ricevere un tag finché una revisione indipendente eleggibile non è
registrata.

Questa decisione pre-freeze è stata successivamente superata, per il solo
legame fra immutabilità e review, dalla
[`CIA del freeze tecnico`](CHANGE_IMPACT_2026-07-29_TECHNICAL_FREEZE.md).
La revisione indipendente resta aperta e nessun tag è autorizzato.
