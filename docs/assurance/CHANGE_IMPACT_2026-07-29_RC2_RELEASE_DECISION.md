# Change impact analysis — decisione component RC 0.1.0-rc.2

Data: 2026-07-29

## Decisione

La revisione di implementazione
`179ad037aad18c3c92ff3c703315a7033ff43773` è autorizzata come candidato
tecnico per `plenora-IO-tools v0.1.0-rc.2`, con claim
`verified_internally`.

La revisione contiene l'estrazione FileGDB mediante indici OGR pre-risolti e
validati, il benchmark ripetibile e la relativa CIA. La revisione
`120f0cfb572e248c503d838543269db0bb830ead` aggiunge soltanto la provenienza
della CI del candidato e non modifica l'implementazione.

Questa decisione:

- autorizza una nuova RC del componente, non una RC del sistema;
- non modifica né sposta il tag immutabile `v0.1.0-rc.1`;
- non dichiara revisione indipendente;
- non dichiara certificazione avionica;
- autorizza il tag `v0.1.0-rc.2` soltanto dopo allineamento della versione,
  gate locali, CI verde della revisione pre-tag e record finale coerente.

## Base e contenuto

- base congelata precedente: tag annotato `v0.1.0-rc.1`, target
  `ca39d6272b06e290f727b62200ea36cc25d6f826`;
- candidato di implementazione:
  `179ad037aad18c3c92ff3c703315a7033ff43773`;
- ICD: `plenora-contracts v2.0-rc8`, revisione
  `62b12e3496466d2c908dac3cc098640b99b52e21`;
- wire contract: versione `1`, invariata;
- dipendenze e capability pubbliche: invariate;
- differenza funzionale: nessuna; cambia soltanto il costo dell'estrazione
  degli attributi FileGDB.

## Evidenza candidata

La CI
[`30442548998`](https://github.com/PlenoraETL/plenora-IO-tools/actions/runs/30442548998)
copre esattamente la revisione di implementazione ed è verde sui job `rust`,
`windows`, `macos-publish` e `coverage`.

Artifact coverage:

- id: `8720051575`;
- nome: `rust-coverage-lcov`;
- dimensione: `93803` byte;
- SHA-256:
  `4e1ffe7ea3e72977321270c2e9135c9a2da36a45fce3971d744f288671ab47b7`;
- line coverage delle librerie: `12769 / 15271`, pari a `83,62%`.

Verifica locale candidata:

- test workspace completi, tutti i target e tutte le feature: pass;
- Clippy workspace e gate safety delle librerie: pass;
- FileGDB feature-on su GDAL 3.10.3: 22 test pass, 2 helper controllati
  dai test padre;
- registro fallback: 88, invariato;
- replay corpus WKB/EWKB: 18 casi, nessuna differenza non classificata;
- fuzz strutturato, seed `20260729`: 29.180.000 iterazioni, zero finding;
- A/B FileGDB 50.000 × 64: `+458,8%` full scan e `+50,4%`
  proiezione stretta, checksum invariati.

## Claim e limiti

Il claim resta `verified_internally`. Lo stato della revisione indipendente è
`not_performed` e non blocca una component RC interna, ma impedisce qualunque
claim `verified_independently`.

Restano esclusi:

- RC del sistema a tre componenti;
- FileGDB/GDAL nativo Windows nel perimetro operativo;
- pushdown nativo `OGR_L_SetIgnoredFields`;
- qualifica degli strumenti e copertura MC/DC;
- certificazione o conformità DO-178C/ED-12C.

## Sequenza autorizzata

1. registrare e verificare questa decisione;
2. portare workspace e lockfile a `0.1.0-rc.2`;
3. generare i manifesti pre-tag senza dichiarare il tag creato;
4. eseguire la CI sulla revisione pre-tag;
5. registrare SHA e CI pre-tag nel record finale;
6. eseguire la CI del record finale;
7. creare e pushare il tag annotato non firmato `v0.1.0-rc.2`;
8. verificare la CI attivata dal tag.
