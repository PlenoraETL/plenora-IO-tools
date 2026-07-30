# Change impact analysis — avvio del programma component RC 0.1.0-rc.4

Data: 2026-07-30.

## Baseline

RC4 parte dal tag annotato e immutabile `v0.1.0-rc.3`, target
`ea0de79677e8fc794d96ac3d95c5bc2c6e30358c`. La baseline di implementazione
contenuta nella release è
`3f3562a4707995549ff5eb8dc03f9e37f2cde355`.

Il workspace passa a `0.1.0-rc.4` come versione di sviluppo. Questo avvio non
crea una nuova RC e mantiene `component_rc`, `system_rc` e
`avionic_certification` a `false`.

## Programma

I tre gruppi differiti da RC3 restano indipendenti:

1. ridurre la materializzazione dei reader KML, DXF e XLSX senza inventare
   schema o semantica e senza superare i veti prestazionali;
2. applicare projection fisica OpenFileGDB soltanto attraverso una API safe o
   un fork governato, senza introdurre `unsafe` nel componente;
3. ottenere un ambiente GDAL/OpenFileGDB Windows riproducibile e rieseguire le
   matrici native, crash/recovery e filesystem senza peggiorare il percorso
   narrow oltre il 5%.

## Primo incremento: XLSX a due passate

`calamine 0.36.1`, già pinnato, espone `Xlsx::worksheet_cells_reader`, un
reader lazy delle celle usate con coordinate esplicite. RC4 usa questa API
senza cambiare il contratto:

- passata 1 durante `open`: intestazione, inferenza monotona dei tipi,
  geometria, dimensioni e tipi WKB, con memoria proporzionale al numero di
  colonne;
- passata 2 nel `LayerReader`: nuova apertura del workbook e produzione di
  `RecordBatch` bounded da `batch_target`, con canale a capacità limitata;
- righe e celle sparse vengono ricostruite senza cambiare l'ordine o inventare
  valori;
- cancellazione verificata durante entrambe le passate;
- errori del parser e valori incompatibili restano fail-closed.

La strategia è deliberatamente a due passate: una sola passata senza
`schema_hint` sceglierebbe i tipi prima di avere osservato tutto il foglio.

## Workstream ancora bloccati

- KML richiede un parser event-based semanticamente equivalente e un benchmark
  interlacciato che superi il veto.
- DXF richiede un iteratore upstream pubblico oppure un fork governato.
- OpenFileGDB richiede una API safe equivalente a
  `OGR_L_SetIgnoredFields` oppure un fork governato.
- Il bundle GDAL 0.19/GDAL 3.12.1 resta respinto per la regressione narrow del
  31,77%; una nuova soluzione deve restare entro il 5%.

I residui CRS combinati e l'osservabilità CLI del `LossReport` reader non
vengono assorbiti implicitamente in RC4: restano rispettivamente decisione
dell'owner ed attività esterna registrata.

## Gate

- rustfmt e Clippy safety;
- test workspace e test mirati XLSX;
- equivalenza di schema, payload, righe sparse e batch multipli;
- cancellazione durante la scansione e durante l'emissione;
- benchmark interlacciato baseline/RC4 su tempo, RSS e allocazioni prima di
  promuovere `ReadMode::StreamingSequential`;
- gate di provenienza release con RC3 immutabile e RC4 ancora `development`.
