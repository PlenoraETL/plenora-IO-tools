# Change impact analysis — decisione component RC 0.1.0-rc.4

Data: 2026-07-30.

## Decisione

La revisione
`dc85f5163860bd16c4cf0bfa1066276980d38e8c` è autorizzata come candidato
tecnico per `plenora-IO-tools v0.1.0-rc.4`, con claim
`verified_internally`.

Questa decisione:

- autorizza una nuova RC del solo componente IO-tools;
- non modifica né sposta il tag immutabile `v0.1.0-rc.3`;
- non dichiara una RC del sistema a tre componenti;
- non dichiara revisione indipendente o certificazione avionica;
- autorizza il tag `v0.1.0-rc.4` soltanto dopo manifesti pre-tag coerenti, CI
  verde della revisione pre-tag e record finale verificato.

## Contenuto del candidato

Rispetto a `v0.1.0-rc.3`, RC4 comprende:

1. reader XLSX bounded tramite spool temporaneo, con inferenza completa prima
   dell'emissione e senza materializzare l'intero dataset Arrow;
2. reader KML event-based e reader DXF progressivo, entrambi con memoria
   bounded e benchmark A/B sopra il veto prestazionale;
3. pushdown fisico OpenFileGDB degli attributi e della geometria esclusi,
   tramite il fork governato di `gdal 0.17.1`;
4. ambiente Windows GDAL 3.10.3/OpenFileGDB riproducibile, fissato per digest,
   con test nativi, crash/recovery, cross-volume e benchmark narrow;
5. correzione della classificazione dei WKT proiettati Shapefile e
   dichiarazione bounded dei valori DBF interi la cui precisione non è
   verificabile;
6. fork governato di `dxf 0.6.1`, con provenienza, tree hash e suite upstream
   verificati.

Programma, alternative respinte, misure e residui sono registrati in
[`CHANGE_IMPACT_2026-07-30_RC4_PROGRAM.md`](CHANGE_IMPACT_2026-07-30_RC4_PROGRAM.md).

## Evidenza della revisione candidata

La CI
[`30605882153`](https://github.com/PlenoraETL/plenora-IO-tools/actions/runs/30605882153)
copre esattamente `dc85f5163860bd16c4cf0bfa1066276980d38e8c` ed è verde sui
job `rust`, `windows`, `macos-publish` e `coverage`.

L'artifact `rust-coverage-lcov`:

- id: `8783562020`;
- dimensione: `103610` byte;
- digest:
  `sha256:a8c262e8f3d330f70c1c820f9e355a3a349bd9bb86a0d29193e1151b365b7d24`;
- stato del gate di copertura librerie: superato, soglia 80%.

L'artifact `windows-filegdb-narrow-benchmark`:

- id: `8783600878`;
- dimensione: `774` byte;
- digest:
  `sha256:5b4f56c896813e9d72227f53094b7e72625ae23f43949d838a4982b3dfb89a6e`;
- stato del veto prestazionale: superato.

La stessa CI verifica inoltre i fork governati GDAL e DXF, i 92 fallback
semantici censiti, Clippy su tutte le feature, i test workspace, i test
FileGDB nativi e la build release locked.

## Limiti e residui dichiarati

RC4 non comprende:

- confronto fra `crs_definition` WKT/PROJJSON e SRID; R4.3.1 non è dichiarata
  completamente implementata;
- esposizione machine-readable del `LossReport` dei reader nel documento
  prodotto dalla CLI;
- decisione sul comportamento di un comando che legge e scrive nello stesso
  processo quando incontra rappresentazioni CRS discordanti;
- revisione dell'ambiguo `loss.lossless`, che descrive il solo writer e non la
  fedeltà end-to-end della conversione;
- qualifica della matrice a tre componenti.

I primi quattro punti restano rispettivamente residui dichiarati, attività
esterna o decisione dell'owner. Il gate di sistema resta separato e
`not_satisfied`.

## Claim

Il claim resta `verified_internally`. La revisione indipendente è un attributo
di assurance aperto e non blocca questa component RC, ma impedisce qualunque
claim `verified_independently`.

La qualifica della matrice a tre componenti deve essere rieseguita sul tag
immutabile. Il pass di una fixture che conserva rappresentazioni CRS
discordanti non verifica, da solo, la dichiarazione nel `LossReport` finché il
runner esterno non legge tale rapporto in forma machine-readable.

## Sequenza autorizzata

1. committare questa decisione e ottenere CI verde;
2. generare i manifesti pre-tag con `component_rc: false`;
3. eseguire la CI sulla revisione pre-tag;
4. legare SHA e run CI nel record finale;
5. eseguire la CI del record finale;
6. creare e pushare il tag annotato non firmato `v0.1.0-rc.4`;
7. verificare la CI attivata dal tag.
