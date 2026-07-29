# Change impact analysis — decisione component RC 0.1.0-rc.3

Data: 2026-07-30.

## Decisione

La revisione di implementazione
`3f3562a4707995549ff5eb8dc03f9e37f2cde355` è autorizzata come candidato
tecnico per `plenora-IO-tools v0.1.0-rc.3`, con claim
`verified_internally`.

Questa decisione:

- autorizza una nuova RC del solo componente IO-tools;
- non modifica né sposta il tag immutabile `v0.1.0-rc.2`;
- non dichiara una RC del sistema a tre componenti;
- non dichiara revisione indipendente o certificazione avionica;
- autorizza il tag `v0.1.0-rc.3` soltanto dopo manifesti pre-tag coerenti, CI
  verde della revisione pre-tag e record finale verificato.

## Contenuto del candidato

Rispetto a `v0.1.0-rc.2`, RC3 comprende:

1. codec lossless esteso a tutti i tipi geometrici concreti canonici R3.1,
   senza linearizzare curve o superfici;
2. harness fuzz committati e campagna coverage-guided lunga riproducibile, con
   zero finding;
3. dichiarazione al bordo di lettura dell'incoerenza fra `crs_id` EPSG e
   `plenora.geometry.srid`, preservando entrambe le rappresentazioni e
   registrando `inconsistent_crs_representations` nel `LossReport`.

La decisione di perimetro completa è registrata in
[`CHANGE_IMPACT_2026-07-30_RC3_CRS_SCOPE.md`](CHANGE_IMPACT_2026-07-30_RC3_CRS_SCOPE.md).

## Evidenza della revisione candidata

La CI
[`30500304709`](https://github.com/PlenoraETL/plenora-IO-tools/actions/runs/30500304709)
copre esattamente `3f3562a4707995549ff5eb8dc03f9e37f2cde355` ed è verde sui
job `rust`, `windows`, `macos-publish` e `coverage`.

L'artifact `rust-coverage-lcov`:

- id: `8743219769`;
- dimensione: `95566` byte;
- digest:
  `sha256:d37a8296fc1e10a758ac18d910f56e51d6f6bd3dc365bcd7a8ba7b991ed23c25`;
- stato del gate di copertura librerie: superato, soglia 80%.

Dopo il push della revisione candidata sono stati inoltre rieseguiti su
working tree pulito:

- `cargo fmt --all -- --check`;
- `cargo test --workspace`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- gate del contratto di release e i relativi 16 test.

## Limiti e residui dichiarati

RC3 non comprende:

- confronto fra `crs_definition` WKT/PROJJSON e SRID; R4.3.1 non è dichiarata
  completamente implementata;
- esposizione del `LossReport` dei reader nel documento JSON della CLI;
- decisione sul comportamento di un comando che legge e scrive nello stesso
  processo quando incontra rappresentazioni CRS discordanti; la distinzione
  fra propagare e scegliere è rimessa all'owner dell'ICD;
- streaming bounded di KML/DXF/XLSX;
- pushdown nativo OpenFileGDB;
- matrice FileGDB/GDAL Windows e filesystem reali.

Gli ultimi tre workstream sono differiti a RC4. Gli altri sono residui
espliciti o decisioni esterne al perimetro tecnico di RC3 e non vengono
silenziosamente promossi a conformità.

## Claim

Il claim resta `verified_internally`. La revisione indipendente è un attributo
di assurance aperto e non blocca questa component RC, ma impedisce qualunque
claim `verified_independently`.

La qualifica della matrice a tre componenti resta separata. Poiché RC3 modifica
il codice della libreria rispetto alla revisione qualificata in precedenza, la
matrice deve essere rieseguita sul tag immutabile prima di produrre nuova
evidenza di sistema.

## Sequenza autorizzata

1. committare questa decisione e ottenere CI verde;
2. generare i manifesti pre-tag con `component_rc: false`;
3. eseguire la CI sulla revisione pre-tag;
4. legare SHA e run CI nel record finale;
5. eseguire la CI del record finale;
6. creare e pushare il tag annotato non firmato `v0.1.0-rc.3`;
7. verificare la CI attivata dal tag.
