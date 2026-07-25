# ADR-IO 3 — Capability-check di scrittura per formato

**Stato:** Accettato (Fase 0). Vincola §4.1 di `Architetture.md`.

## Contesto

Ogni formato rappresenta un sottoinsieme diverso del contratto RecordBatch. Lo
Shapefile ha nomi campo ≤10 caratteri, tipi DBF limitati, **un solo tipo
geometrico per file**; il DXF non ha tabella attributi generica; KML/GeoJSON
sono WGS84 per specifica; CSV/XLSX non hanno CRS né geometria nativa. La
scrittura deve fallire **in anticipo** e in modo tipizzato, non a metà file.

## Decisione

**1. `FormatWriteCapabilities` dichiarate dal driver** e verificate in `create`,
prima di scrivere qualsiasi byte:

```rust
struct FormatWriteCapabilities {
    field_names: FieldNamePolicy,          // NON un semplice limite numerico (vedi sotto)
    allowed_types: TypeSet,                // sottoinsieme del set chiuso
    geometry: GeometryWriteSupport,        // single-type-per-file? multi? nessuna?
    crs: CrsWriteSupport,                  // Embedded | FixedWgs84 | None
    nullability: NullabilitySupport,
    multi_layer: bool,
}
```

**1b. La policy dei nomi è più ampia di un limite di byte.** I formati vincolano
i nomi campo su assi diversi (byte vs caratteri, encoding, normalizzazione
Unicode, case folding, parole riservate, unicità case-insensitive, collisioni
dopo troncamento). La policy li dichiara; la v1 non deve implementarli tutti, ma
il modello li ammette:

```rust
struct FieldNamePolicy {
    max_bytes: Option<usize>,          // shp DBF: 10
    max_chars: Option<usize>,
    encoding: Option<TextEncoding>,
    case_sensitive: bool,
    normalization: NameNormalization,  // NFC/NFD/none
    reserved_names: NameSet,
}
```

**2. `validate_write(contract) -> Result<()>`** confronta il `DataContract` in
ingresso con le capability. Ogni violazione è un errore **tipizzato** con il
campo e il motivo specifici (mai valori di cella):

- nome campo troppo lungo → `Unsupported { field, reason: FieldNameTooLong }`;
- tipo non rappresentabile → `Unsupported { field, reason: TypeNotRepresentable }`;
- geometrie miste su formato single-geometry → `Unsupported { reason: MixedGeometry }`;
- CRS ≠ WGS84 su formato `FixedWgs84` → `Crs { reason: ReprojectionRequired }`.

**3. Nessun rimescolamento silenzioso dei nomi campo.** Un formato che tronca
(shp ≤10) **non tronca in silenzio**: o il contratto entra così com'è, o è un
errore che elenca i campi in conflitto. Un'eventuale troncatura/normalizzazione è
possibile solo tramite **opzione esplicita** (`--truncate-field-names`),
documentata; nella v1 il default è **fail-closed**. Ogni mapping esplicito
produce un **report macchina-leggibile**, mai una modifica opaca:

```rust
struct FieldRenameReport {
    original: String,
    written: String,
    reason: RenameReason,   // TooLong | Reserved | Collision | Normalized | Encoded
}
```

**4. Attributi non rappresentabili** (DXF senza tabella attributi): mappatura
dichiarata (timbri INSERT) o **report `dropped_properties`**, mai perdita
silenziosa (rimanda a ADR-IO 5).

## Conseguenze

- Un `write` che parte **completa** o non parte affatto: nessun file a metà per
  incompatibilità di schema.
- Il chiamante riceve in `create` l'elenco esatto di ciò che il formato non
  regge, prima di impegnare I/O.
- Test obbligatori: per ogni driver, un contratto che viola ciascuna capability
  → errore tipizzato corretto; il caso valido passa.

## Alternative scartate

- **Troncamento/mangling automatico e silenzioso dei nomi**: perdita di
  informazione non dichiarata, contraria al principio fail-closed.
- **Validazione a metà scrittura**: lascerebbe file parziali.
