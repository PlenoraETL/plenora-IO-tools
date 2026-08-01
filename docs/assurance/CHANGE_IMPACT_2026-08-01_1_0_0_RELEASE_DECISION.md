# Change impact analysis — decisione candidata 1.0.0

**Data:** 2026-08-01  
**Componente:** `plenora-IO-tools`  
**Stato:** metadata candidate; nessun tag, merge o publish autorizzato

## Decisione

Il workspace viene preparato con versione `1.0.0` per una release finale del
solo componente. La superficie di compatibilità 1.x resta limitata alle sei
buste JSON della CLI descritte in `release/cli-protocol-v1.json`; i crate Rust
restano `publish = false` e API interne instabili.

La baseline funzionale `938dab99567fffde6510bb3c3e5e944e6bff42df` ha CI
same-SHA verde nel run `30692495395`. Questa evidenza non si trasferisce
silenziosamente al diff metadata: il futuro commit deve rieseguire l'intera CI
same-SHA e il gate `--qualify-current` prima di qualsiasi tag.

## Provenienza e limiti

- `v1.0.0-rc.2` e tutti i suoi manifesti/evidence restano immutabili;
- il tag previsto `v1.0.0` non è stato creato;
- `system_rc`, verifica indipendente e certificazione avionica restano false;
- il gate di sistema resta `not_satisfied` e non è promosso dalla release del
  componente;
- la baseline Contracts è citata per revisione esatta `e81c3ce7941bacbdb0e083f03c512ae22a6b924a`, senza inventare un tag rc16.

## Gate richiesti

1. gate storico RC.2 ancora verde;
2. workspace e lockfile coerenti a `1.0.0`;
3. test del protocollo CLI v1;
4. CI completa Linux/Windows/macOS/FileGDB/coverage/audit;
5. qualifica same-SHA del commit metadata;
6. eventuale tag soltanto dopo autorizzazione separata.
