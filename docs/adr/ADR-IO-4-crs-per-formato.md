# ADR-IO 4 — Estrazione e serializzazione del CRS per formato

**Stato:** Accettato (Fase 0). Vincola D3 di `Architetture.md`.

## Contesto

Ogni formato porta il CRS in modo diverso; alcuni non lo portano affatto. Il
driver deve produrre/consumare un `geo.crs` risolto, **senza mai riproiettare**
(la riproiezione è lo step `geo.reproject` di `plenora-data-tools`).

## Decisione

**1. Estrazione (lettura) formato → `ResolvedCrs`:**

| Formato | Fonte CRS |
|---|---|
| Shapefile | `.prj` (WKT) → `resolve_crs` |
| GeoPackage | `gpkg_spatial_ref_sys` per `srs_id` della tabella |
| GeoParquet | `geo.crs` PROJJSON (assente/null ⇒ OGC:CRS84) |
| DXF | WKT ESRI incorporata (es. RDN2008/EPSG:7794) |
| KML / GeoJSON | fisso **OGC:CRS84** (WGS84 lon/lat) per specifica |
| CSV / XLSX | **nessuno**: dichiarato dal chiamante |

**2. CRS assente/non risolvibile con geometria presente → fallimento chiuso
`CRS_UNRESOLVED`.** Nessun default implicito sbagliato.

- Per i formati **senza CRS** (CSV/XLSX) con una colonna geometrica: il CRS è
  **obbligatorio via opzione esplicita** (`--assume-crs EPSG:XXXX`); in assenza,
  errore. Nessun WGS84 implicito.

**3. Serializzazione (scrittura) `geo.crs` → meccanismo del formato:** `.prj`
per shp, riga in `gpkg_spatial_ref_sys` per gpkg, `geo.crs` PROJJSON per
geoparquet, WKT ESRI per dxf.

**4. Formati a CRS fisso (KML/GeoJSON = WGS84):** se il contratto in ingresso ha
un CRS ≠ WGS84 → errore `ReprojectionRequired` (riproiettare a monte con
data-tools), **mai** scrivere le coordinate proiettate spacciandole per WGS84
(è esattamente il difetto che il lavoro DXF ha chiuso).

**5. Nessuna riproiezione nel driver, in nessun verso.** Il CRS letto è il CRS
riportato/scritto.

**6. Ordine assi: nessuna canonicalizzazione implicita.** `OGC:CRS84` (lon, lat)
ed `EPSG:4326` (ordine assi definito formalmente da EPSG) **non** sono trattati
come sinonimi: i driver **preservano l'ordine assi previsto dal formato** e non
convertono silenziosamente l'uno nell'altro. Eventuali equivalenze o
normalizzazioni sono **esplicite** nel contratto CRS condiviso (`plenora-core`),
mai affidate a una canonicalizzazione implicita di libreria.

**7. CRS embedded non pienamente risolvibile: diagnostica senza uso.** La
risoluzione distingue tre casi, così un CRS grezzo dichiarato ma non risolto è
conservato per la diagnostica **senza** essere usato come valido:

```rust
enum CrsResolution {
    Resolved(ResolvedCrs),
    DeclaredButUnresolved(RawCrs),  // presente nel file ma non risolvibile
    Missing,
}
```

La v1 continua a **fallire chiuso** (`CRS_UNRESOLVED`) quando c'è geometria e il
CRS non è `Resolved`; il `RawCrs` finisce nell'errore/diagnostica, non nel dato.

## Conseguenze

- Il `geo.crs` della colonna geometrica è sempre popolato e risolvibile quando
  c'è geometria; a valle data-tools sa sempre in che sistema sono le coordinate.
- L'unico modo per cambiare CRS è lo step esplicito di data-tools: separazione
  netta I/O ↔ trasformazione.
- Test obbligatori: estrazione CRS per ogni formato; CSV con geometria senza
  `--assume-crs` → errore; scrittura KML con CRS ≠ WGS84 → errore; round-trip
  del CRS su gpkg/shp/geoparquet/DXF; DXF senza GEODATA e senza
  `--assume-crs` → errore.

**Nota di implementazione corrente.** Il gate di conformità attraversa ogni
descrittore scrivibile: le capability `Embedded` rifiutano un CRS mancante e le
capability `Fixed` rifiutano un CRS incompatibile con
`ReprojectionRequired`. Restano da uniformare sul lato lettura la conservazione
del `RawCrs` non risolto e i test sull'ordine degli assi.

## Alternative scartate

- **WGS84 implicito per CSV/XLSX**: assunzione silenziosa e spesso errata.
- **Riproiezione automatica verso il CRS del formato**: violerebbe D3 e
  nasconderebbe una trasformazione dentro l'I/O.
