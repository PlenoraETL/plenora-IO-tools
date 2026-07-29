# Change impact analysis — avvio del programma component RC 0.1.0-rc.3

Data: 2026-07-29

## Decisione

`v0.1.0-rc.2`, target
`f47bf4605b248d127205e49a7e6ebd2a0984a83f`, resta una baseline immutabile.
Lo sviluppo successivo appartiene al candidato `v0.1.0-rc.3` e non modifica
retroattivamente i claim o le evidenze di RC2.

Il programma RC3 resta limitato al componente `plenora-IO-tools`. Non autorizza
modifiche a `plenora-data-tools`, `plenora-database-tools` o
`plenora-contracts`.

## Workstream autorizzati

1. Eseguire una campagna coverage-guided lunga sui parser e sul codec
   WKB/EWKB, conservando seed, durata, log, corpus e finding.
2. Mantenere predisposto il pacchetto per la revisione indipendente. La review
   deve essere svolta da una persona eleggibile: automazione e self-review non
   possono chiuderla.
3. Estendere il codec lossless dai sette tipi WKB semplici ai sedici tipi
   canonici R3.1, senza normalizzare curve o superfici in tipi lineari.
4. Ridurre la materializzazione nei reader KML, DXF e XLSX e migliorare la
   latenza di cancellazione. Le chiamate sincrone non interrompibili delle
   dipendenze restano un limite dichiarato finché non vengono sostituite.
5. Valutare e, se compatibile con il profilo safety, implementare il pushdown
   nativo OpenFileGDB tramite `OGR_L_SetIgnoredFields`, con benchmark A/B e
   verifica dell'ordine/schema.
6. Ampliare la matrice FileGDB a Windows nativo e a filesystem/GDAL
   identificati. Un ambiente non disponibile resta `not_run`, mai `pass`.
7. Verificare la revisione ICD adottata prima del freeze RC3 e migrare soltanto
   in presenza di una ratifica o di una modifica effettiva del contratto.

## Hazard e invarianti

- **H-01 — reinterpretazione geometrica:** i nuovi type code devono
  round-trippare con tipo, dimensioni, SRID, ordine dei figli e coordinate
  invariati. Gli adattatori `geo-types` XY devono rifiutare i tipi estesi che
  non possono rappresentare senza perdita.
- **H-03 — esaurimento risorse:** tutti i nuovi contenitori WKB consumano lo
  stesso budget `WkbLimits`; le campagne fuzz mantengono limiti RSS, lunghezza
  e timeout.
- **H-04 — arresto:** restano vietati `unsafe` e primitive esplicite di panic
  nel codice distribuibile. Un binding FFI FileGDB non può essere introdotto
  implicitamente per ottenere il pushdown.
- **H-06 — CRS:** nessun intervento prestazionale può modificare SRID, cinque
  chiavi CRS o ordine assi.
- **H-08/H-09 — verifica insufficiente:** ogni variazione funzionale richiede
  test negativi, CIA, confronto pre/post e CI sulle piattaforme dichiarate.

## Veto prestazionali

- Codec: nessuna regressione superiore al 5% nel throughput del percorso
  semplice WKB 1–7; nessuna crescita non motivata delle allocazioni.
- KML/DXF/XLSX: conservare o migliorare tempo e picco RSS sulle fixture
  ripetibili; una regressione oltre il 5% annulla il cambiamento.
- FileGDB: il pushdown è accettabile solo se checksum, schema e determinismo
  restano invariati e la proiezione stretta migliora in modo misurabile. Il
  percorso full projection non può peggiorare oltre il 5%.

## Gate esterni non chiudibili con codice locale

- revisione indipendente;
- disponibilità di un ambiente FileGDB/GDAL Windows nativo qualificabile;
- ratifica delle sezioni candidate dell'ICD;
- claim di RC di sistema o certificazione avionica.

Questi gate restano visibili e non bloccano l'avvio dello sviluppo RC3, ma
impediscono qualsiasi claim che li presupponga.
