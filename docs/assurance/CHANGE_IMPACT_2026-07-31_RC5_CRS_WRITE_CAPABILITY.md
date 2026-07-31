# Change impact — capability delle rappresentazioni CRS in scrittura

**Data:** 2026-07-31
**Baseline:** `v0.1.0-rc.4`
**Sviluppo:** `0.1.0-rc.5`
**Claim:** implementazione candidata di R4.6.5 (`2.0-rc12`); nessuna ratifica
implicita

## Problema

`CrsWriteSupport::{Embedded, EmbeddedOptional, Fixed, None}` descriveva il
vincolo operativo del formato, ma non quali rappresentazioni del contratto
fossero conservate. Shapefile, GeoParquet, GeoPackage, FileGDB e DXF
dichiaravano tutti `Embedded` pur trattando diversamente `crs_id`, `srid` e
`crs_definition`. Il validatore non poteva quindi distinguere la propagazione
di rappresentazioni discordanti da una scelta o derivazione silenziosa.

## Decisione

`FormatWriteCapabilities` espone ora una capability separata per ciascuna
rappresentazione:

- `preserved`: il valore sorgente sopravvive indipendentemente;
- `absent`: il formato o il writer non lo emette;
- `derived`: il valore in uscita è ricostruito da un'altra rappresentazione o
  da una regola del formato.

`derived` non equivale a conservazione: può rendere coerente in uscita una
discordanza presente in ingresso e cancellarne così l'evidenza.

La matrice dichiarata è conservativa e descrive l'implementazione corrente:

| Writer | `crs_id` | `srid` | `crs_definition` |
|---|---|---|---|
| Arrow IPC | preserved | preserved | preserved |
| GeoParquet | preserved | absent | absent |
| GeoPackage | preserved | derived | derived |
| Shapefile | derived | absent | preserved |
| DXF | derived | absent | preserved |
| FileGDB | derived | absent | derived |
| GeoJSON / KML | derived | absent | absent |
| CSV / XLSX | absent | absent | absent |

`CrsWriteSupport` resta invariato e continua a esprimere se il CRS è
incorporato, opzionale, fisso o assente. I due assi non sono intercambiabili.

## Preflight R4.6.5

Quando il contratto contiene `crs_id` EPSG e `srid` discordanti, il writer può
partire soltanto se entrambe le rappresentazioni sono `preserved`. Arrow IPC
le propaga entrambe; Shapefile e gli altri writer che dovrebbero scartarne o
derivarne almeno una falliscono durante `Validate`, con effetto remoto `none`
e retry `never`, prima di creare output visibile.

Il confronto semantico fra `crs_definition` e le altre rappresentazioni resta
fuori scope: richiede un resolver ed è già dichiarato come residuo R4.3.1.

## LossReport

Per contratti coerenti, una rappresentazione presente ma non `preserved` non
sparisce più dietro la categoria generica «metadata CRS non rappresentati».
Il report distingue rappresentazione e stato, per esempio:

- `crs_id_not_preserved_derived`;
- `srid_not_preserved_absent`;
- `crs_definition_not_preserved_absent`.

Gli esempi bounded registrano layer, campo, stato e lunghezza del valore, senza
pubblicare definizioni CRS o altri valori potenzialmente sensibili.

## Correzione GeoParquet

Il writer GeoParquet prende ora `crs_id` dal `GeometryColumnContract`, usando il
vecchio metadato di campo `crs` soltanto come fallback. In questo modo la
capability `crs_id: preserved` è vera anche per un contratto costruito
programmaticamente.

## Proposte non implementate

Questo intervento non modifica il riconoscimento delle colonne Arrow:

- R2.8 (`2.0-rc12`), riconoscimento geometrico dalle sole chiavi canoniche,
  resta proposta;
- R4.1.1 (`2.0-rc13`), `DeclaredButUnresolved` con `crs_definition` assente,
  resta proposta;
  `RawCrs::definition` rimane obbligatoria.

La matrice esterna a 28 casi separa ora conservazione opaca e comprensione
semantica. Sulle teste precedenti all'intervento ha misurato per IO-tools
`14/14` sulle varianti canoniche e `13/14` sulle varianti GeoArrow; l'unico
fallimento è il caso R4.1.1 proposto. Questi numeri non sono evidenza prodotta
da questo repository e non vengono promossi a gate locale.

## Compatibilità e versioni

La nuova proprietà cambia lo schema del catalogo, quindi
`descriptor_version` aumenta di uno per tutti i driver. GeoParquet aumenta
anche `driver_version` perché cambia la sorgente autoritativa del `crs_id` in
scrittura. Le API Rust restano interne e non sono superficie SemVer 1.x; la
forma JSON della CLI resta l'unica superficie candidata al freeze.

## Verifica richiesta

- test unitario del predicato con `EPSG:4326` e `srid=3003`;
- matrice sui descrittori reali: IPC accetta, Shapefile rifiuta;
- conversione reale IPC → IPC e riapertura con entrambe le rappresentazioni
  invariate;
- tentativo IPC → Shapefile rifiutato prima della creazione dell'output;
- test delle categorie `LossReport` per `absent`, `derived` e `preserved`;
- test GeoParquet senza metadato legacy `crs`;
- rustfmt, Clippy, test workspace e gate di assurance.
