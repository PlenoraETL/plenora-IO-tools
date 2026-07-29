# FileGDB: projection, determinismo e matrice GDAL

Stato al 2026-07-28: **projection Arrow esatta e determinismo semantico
verificati; pushdown nativo delle colonne e GDAL nativo Windows aperti**.

## Risultato tecnico

Il reader costruisce esclusivamente builder e array per i campi richiesti e
non invoca `Feature::field` né `Geometry::wkb` per le colonne escluse. Lo
schema e il `RecordBatch` prodotti coincidono quindi esattamente con la
projection, compreso il caso senza colonne.

Questo non è ancora un pushdown nativo nel driver OpenFileGDB: GDAL può avere
materializzato internamente campi esclusi prima di restituire la `Feature`.
La primitiva corretta è `OGR_L_SetIgnoredFields`. Essa è presente nell'API C
di GDAL e in `gdal-sys`, ma non è esposta dall'API safe di `gdal` usata dal
workspace (pin `0.17.1`). Anche la release upstream `gdal 0.19.0`, verificata
il 2026-07-28, non dichiara un wrapper safe per questa primitiva.

Non sono state adottate due scorciatoie:

- chiamare direttamente `gdal-sys` introdurrebbe `unsafe` nel crate, vietato
  dal profilo;
- sostituire il layer con una query OGRSQL `SELECT` cambia identità del layer,
  FID, comportamento della geometria e piano del driver; non è equivalente a
  ignorare campi e richiederebbe una nuova matrice semantica e prestazionale.

Il pushdown nativo resta quindi bloccato in modo esplicito da una API upstream,
non viene dichiarato come implementato e non giustifica una deroga `unsafe`.

## Determinismo

`read_determinism` e `write_determinism` restano correttamente
`semantic`, non `byte_for_byte`: un directory dataset FileGDB contiene
artefatti e ordinamenti interni controllati da GDAL.

Il test feature-on
`repeated_filegdb_writes_are_semantically_deterministic`:

1. pubblica due `.gdb` indipendenti dallo stesso piano e dallo stesso batch;
2. riapre entrambi con due handle separati;
3. confronta nome layer, dimensioni, tipi, CRS, axis order e geometria
   decodificata;
4. non usa l'uguaglianza byte-per-byte come oracolo improprio.

Esecuzione locale su Linux x86_64, Rust 1.92.0 e GDAL 3.10.3: superata.

## Matrice osservata

| Ambiente | GDAL | Lettura/scrittura | Crash publish | Determinismo semantico | Pushdown nativo |
|---|---:|---|---|---|---|
| Linux locale corrente | 3.10.3 | verificata | verificato | verificato | no, API safe assente |
| Linux evidenza storica | 3.6.2 | verificata | verificato | non rieseguito con il nuovo test | no |
| CI Ubuntu | versione pacchetto runner | verificata a ogni run | verificato | nuovo gate al prossimo run | no |
| Windows runner | nessun GDAL nativo | stub/pure-Rust soltanto | non applicabile | non verificato | non verificato |
| macOS runner | nessun GDAL nativo | publish core soltanto | non applicabile | non verificato | non verificato |

La voce “versione pacchetto runner” non è sufficiente per una baseline
avionica: prima del freeze qualificato l'immagine GDAL Linux deve essere
immutabile e identificata da digest. La matrice minima residua è:

- GDAL 3.8 LTS-1, 3.10.3 corrente e 3.12 corrente;
- Windows x64 con un pacchetto GDAL/OpenFileGDB identificato e ridistribuibile;
- NTFS e almeno un filesystem Linux qualificato;
- medesimo corpus FileGDB, inclusi crash, projection vuota, CRS/axis, Z/M e
  doppia scrittura semantica.

Finché questa matrice non è verde, FileGDB resta capability opzionale e non va
incluso nel perimetro operativo aeronautico congelato.
