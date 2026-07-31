# Change impact analysis — decisione component RC 1.0.0-rc.1

Data: 2026-07-31.

## Decisione

La revisione
`796a1f94e0735e4f5b9e8bfca1056c295bda4814` è autorizzata come candidato
tecnico per `plenora-IO-tools v1.0.0-rc.1`, con claim
`verified_internally`.

Questa decisione:

- autorizza una RC del solo componente IO-tools;
- congela come superficie compatibile 1.x soltanto le sei buste JSON della CLI
  dichiarate in `release/cli-protocol-v1.json`;
- mantiene le API Rust interne, non pubblicate e fuori dalla superficie SemVer;
- non modifica né sposta il tag immutabile `v0.1.0-rc.4`;
- non dichiara una RC del sistema, revisione indipendente o certificazione
  avionica;
- autorizza il tag annotato non firmato `v1.0.0-rc.1` soltanto dopo
  allineamento della versione, manifesti pre-tag coerenti, CI verde della
  revisione pre-tag e record finale verificato.

## Evidenza del candidato

La CI
[`30617658910`](https://github.com/PlenoraETL/plenora-IO-tools/actions/runs/30617658910)
copre esattamente
`796a1f94e0735e4f5b9e8bfca1056c295bda4814` ed è verde sui job `rust`,
`windows`, `macos-publish` e `coverage`.

La pipeline comprende formattazione, gate di assurance, Clippy con tutte le
feature, safety lint, test dell'intero workspace su Linux e Windows, build
release locked, audit dipendenze, coverage, publish macOS e matrice
GDAL/OpenFileGDB nativa Windows con benchmark narrow.

## Contenuto della RC

Rispetto a `v0.1.0-rc.4`, la candidata comprende:

1. busta d'errore CLI machine-readable con i quattro assi e retry strutturato;
2. contratto `convert` con `read_loss`, `write_loss` e fedeltà end-to-end;
3. capability writer generale per `crs_id`, `srid` e `crs_definition`, con
   stati `preserved`, `absent` e `derived`;
4. preflight conforme alla proposta R4.6.5: propagazione delle
   rappresentazioni discordanti soltanto quando la destinazione le conserva
   tutte indipendentemente, altrimenti rifiuto fail-closed;
5. dichiarazione per rappresentazione nel `LossReport` quando un writer non
   conserva un valore CRS presente.

## Stato normativo e residui

La RC implementa la proposta R4.6.5 di `plenora-contracts 2.0-rc13`. Lo stato
di proposta limita il claim di conformità, non il tag di componente.

Restano esplicitamente registrate e non implementate:

- R2.8, riconoscimento geometrico IPC dalle sole chiavi canoniche;
- R4.1.1, stato `declared_unresolved` senza `crs_definition`;
- confronto semantico fra `crs_definition` e SRID, che richiede un resolver;
- budget condivisi R7.5–R7.7 e qualifica inversa della catena a tre
  componenti.

Questi residui bloccano soltanto i claim più forti indicati in
`release/rc5-development.json`. La revisione indipendente resta un attributo
di assurance aperto: non blocca questa component RC con claim
`verified_internally`, ma impedisce un claim `verified_independently`.

## Sequenza autorizzata

1. committare questa decisione e ottenere CI verde;
2. allineare workspace, lockfile e manifesti a `1.0.0-rc.1`, mantenendo
   `component_rc: false`;
3. eseguire la CI sulla revisione pre-tag;
4. legare SHA e run CI nel record finale;
5. eseguire la CI del record finale;
6. creare e pushare il tag annotato non firmato `v1.0.0-rc.1`;
7. verificare la CI attivata dal tag.
