# Catena eseguibile IO → data → database

Questo harness verifica una tratta reale, non tre fixture indipendenti:

1. il writer Arrow IPC di IO-tools pubblica tre `Point Z`, CRS `EPSG:4326`,
   SRID e metadati canonici/GeoArrow coerenti;
2. `plenora-data-tools` filtra una riga mediante un DAG v4 e ripubblica IPC;
3. l'oracolo compilato contro `plenora-database-core` valida il contratto
   finale e scandisce ogni cella WKB con il parser EWKB del bordo database.

Sono richiesti i tre checkout fratelli alla stessa revisione dichiarata
dall'evidenza di release:

```text
python scripts/run_three_component_chain.py \
  --data-repo ../plenora-data-tools \
  --database-repo ../plenora-database-tools
```

Il test pretende due righe finali, due geometrie XYZ valide, tipo `point`
coerente con `types_declaration=exact`, `plenora.field_id`, le chiavi CRS e
`plenora.contract.version=1`. Qualunque chiave assente, valore inventato,
degrado XYZ→XY o WKB non accettato dal consumatore database termina con errore.
