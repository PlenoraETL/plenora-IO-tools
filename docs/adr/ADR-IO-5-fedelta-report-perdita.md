# ADR-IO 5 — Fedeltà e report di perdita

**Stato:** Accettato (Fase 0). Vincola D5 di `Architetture.md`.

## Contesto

Alcuni driver sono lossless (GeoPackage, GeoParquet, Arrow IPC, gran parte di
Shapefile); altri sono **approssimanti**: il DXF tassella archi/bulge, esplode i
blocchi INSERT, scarta solidi ACIS. La perdita non deve mai essere silenziosa.

## Decisione

**1. La fedeltà è a tre livelli e dipende dal contratto, non solo dal formato.**
Un driver può essere lossless per certi contratti e approssimante per altri (lo
Shapefile: geometrie potenzialmente lossless, ma nomi campo mappati, precisione
DBF degradata, timestamp non pienamente rappresentabili, nullability con
semantica diversa). Quindi:

```rust
enum Fidelity { Lossless, Conditional, Approximating }
```

- Il **descrittore** dichiara la **capacità generale** (`fidelity_class`).
- `open`/`create` producono una **valutazione specifica** per il dataset/contratto
  concreto:

```rust
struct FidelityAssessment {
    level: Fidelity,
    reasons: Vec<FidelityReason>,  // es. FieldNameMapped, DbfPrecision, TimestampNarrowed
}
```

Nell'API v1 la valutazione preventiva è esposta dall'`OpenDatasetHandle` e dal
`FormatWriter`; il risultato `Published` contiene la valutazione finale. Il
wrapper comune di scrittura analizza il `WritePlan` e incorpora le categorie
del `LossReport`: una perdita osservata promuove sempre l'esito ad
`Approximating`. Motivazioni ed esempi sono bounded.

- **Lossless**: round-trip `file → RecordBatch → file` che preserva la semantica
  — geometria secondo l'uguaglianza di **ADR 1 dei data-tools** (confronto
  geometrico, non byte), tutti i tipi, il CRS.
- **Conditional**: lossless *se* il contratto rientra nelle capacità; la
  valutazione elenca i motivi che lo renderebbero approssimante.
- **Approximating**: dichiara *cosa* altera (tassellazione, esplosione blocchi,
  attributi non rappresentabili).

**2. Un driver `Approximating` DEVE popolare un `LossReport`** nel risultato di
lettura/scrittura, mai perdere in silenzio. Il report è **aggregato per
categoria e bounded**: conteggi per categoria + un numero **limitato** di esempi
diagnostici, **mai** una voce per ogni feature (altrimenti crescerebbe
linearmente con l'input):

```rust
struct LossReport {
    counts: BTreeMap<Category, u64>,   // aggregati: (AcisSolid → 3), (TessellatedArc → 1240)
    examples: BoundedVec<LossExample>, // esempi bounded, separati dai conteggi
}
```

Nessun accumulo illimitato. Il report è parte dell'output macchina-leggibile del
comando (come lo `skipped` di `plenora-dxf-tools` oggi).

**3. Verifica in CI:**

- driver **Lossless** → **round-trip test** che asserisce l'uguaglianza semantica
  (ADR 1);
- driver **Approximating** → **oracolo indipendente** (GDAL `ogrinfo`/`ogr2ogr`,
  pyarrow) su extent/conteggi + **tolleranza documentata**; e verifica che il
  `LossReport` non sia vuoto quando qualcosa è stato approssimato/scartato.

**4. Un `Lossless` che scoprisse di perdere qualcosa è un bug**, non un report:
o si declassa a `Approximating` con report, o si corregge. La fedeltà dichiarata
è un contratto verificato, non un'etichetta.

Il backend FileGDB applica esplicitamente questa regola: il descrittore è
`Conditional`, il writer accetta soltanto il sottoinsieme verificato
round-trip dalla versione GDAL supportata (`Int32`, `Float64`, `Utf8`, WKB
XY/XYZ con un solo tipo geometrico nativo tra `Point`, `MultiPoint`,
`MultiLineString`, `MultiPolygon` e CRS risolto) e rifiuta prima del publish
tipi, dimensioni o semantiche non rappresentabili. `LineString` e `Polygon`
sono rifiutati perché FileGDB li normalizzerebbe nelle rispettive famiglie
multipart. Il backend non converte valori
incompatibili in zero/testo e non elimina feature con geometria nulla. Il
reader ricostruisce tipo e dimensionalità dal geometry field OGR e conserva il
codice nativo nei metadati namespaced; la nullability dello schema resta
`FormatDefined`, mentre i valori nulli sono preservati. M/ZM, EWKB,
`GeometryCollection` e i tipi attributo non verificati restano fail-closed
finché non esiste un round-trip reale che ne dimostri la fedeltà.

## Conseguenze

- Chi consuma l'output sa sempre se e quanto è stato approssimato, con numeri.
- La disciplina degli oracoli (già usata per DXF↔GDAL e GeoParquet↔pyarrow)
  diventa un **criterio di accettazione per driver**.
- Test obbligatori: round-trip lossless; presenza e correttezza del `LossReport`
  sui driver approssimanti; oracolo su corpus reale dove disponibile (RFI DXF).

## Alternative scartate

- **Fedeltà come sola etichetta documentale**: non verificabile, deriva col
  tempo.
- **Perdita implicita "tanto è CAD"**: contraria al principio fail-closed della
  famiglia.
