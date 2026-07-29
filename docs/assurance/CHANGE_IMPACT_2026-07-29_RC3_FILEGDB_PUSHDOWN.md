# Change impact analysis — decisione pushdown nativo FileGDB RC3

Data: 2026-07-29

## Obiettivo

Evitare che GDAL materializzi gli attributi esclusi da una projection
`Required`, mantenendo schema Arrow, ordine, checksum e determinismo della
baseline RC2.

## Stato verificato

Il componente usa `gdal = 0.17.1`. La sorgente pinnata della dipendenza non
espone un metodo safe per `OGR_L_SetIgnoredFields`; la funzione è disponibile
soltanto nel livello C/`gdal-sys`. L'estrazione RC2 pre-risolve e valida gli
indici OGR, ma non impedisce al driver OpenFileGDB di leggere i campi esclusi.

## Alternative

1. **Chiamata diretta `gdal-sys` nel driver.** Respinta: richiede `unsafe`,
   allarga il perimetro FFI del componente e viola il gate corrente
   `#![forbid(unsafe_code)]`.
2. **Wrapper locale che nasconde l'`unsafe`.** Respinto: sposta il rischio senza
   eliminarlo e renderebbe falso il claim “zero unsafe nel codice
   distribuibile”.
3. **Invocazione di utility GDAL in sottoprocesso.** Respinta: modifica errori,
   cancellazione, deployment e costo del percorso hot; non è un equivalente
   del reader in-process.
4. **API safe upstream o fork governato della crate `gdal`.** Ammessa come
   prerequisito: richiede pin esatto, review del wrapper, CIA della dipendenza,
   test multi-GDAL e benchmark A/B.

## Decisione

Il pushdown nativo resta `design_constraint_open`. Non viene introdotto codice
`unsafe` per chiudere artificialmente il punto. L'ottimizzazione indicizzata
RC2 resta il percorso operativo.

La riapertura è autorizzata quando esiste una API safe che:

- accetta nomi C compatibili senza lifetime non verificabili;
- mantiene visibile la geometria se richiesta;
- restituisce gli errori OGR senza perdita;
- viene applicata prima della prima feature;
- supera projection vuota, ordine invertito, dataset mutato tra open/read e
  benchmark interlacciato full/narrow.

## Claim

Nessun incremento di capability e nessun miglioramento prestazionale vengono
dichiarati da questa decisione. Il fatto che il punto resti aperto è
un'applicazione del profilo safety, non un risultato positivo mascherato.
