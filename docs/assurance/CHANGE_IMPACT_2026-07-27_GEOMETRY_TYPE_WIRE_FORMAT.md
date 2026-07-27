# Change impact analysis — wire-format dei tipi geometrici

Data: 2026-07-27

## Baseline e requisito

Fonte autorevole: `plenora-contracts` tag annotato `v2.0-rc2`, oggetto tag
`6c93d5458e7e4fd216116840732aa0488fef9535`, commit
`0faeadbcd34b924430b39647e78e31b34b11bd24`.

La sezione §3.1 ratificata richiede nomi geometrici minuscoli senza separatore.
`GeometryType`, tramite `serde(rename_all = "snake_case")`, emetteva invece
valori come `line_string` e `multi_polygon`. I metadati Arrow impiegavano
inoltre la rappresentazione `Debug`, per esempio `LineString`. Entrambe le
forme erano incompatibili con il consumatore database-tools.

## Modifica e impatto

- `GeometryType` usa `serde(rename_all = "lowercase")`;
- `CoordinateDimensions` usa esplicitamente la stessa convenzione `lowercase`,
  evitando che la conformità dipenda accidentalmente dai nomi semplici delle
  varianti correnti;
- `GeometryType::canonical_name` e `from_canonical_name` centralizzano la forma
  wire dei sette tipi oggi supportati;
- `plenora.geometry.types` emette e accetta soltanto i nomi canonici, quindi
  `linestring`, `multipolygon` e `geometrycollection` senza separatori;
- le forme pregresse `line_string` e `LineString` sono rifiutate come metadato
  canonico malformato, invece di essere reinterpretate.

L’enum non viene ampliato: restano supportati 7 dei 16 tipi di §3.1. In accordo
con §3.2, gli altri nomi non vengono degradati né accettati implicitamente.

La modifica cambia il wire-format serializzato ed è quindi incompatibile con
consumer che dipendano dalla forma non conforme precedente. I crate sono
`0.0.0`, `publish = false`; il cambio avviene prima di una release pubblica.

## Hazard e verifica

- requisito: §3.1 e §3.3 dell’ICD; PLN-ASR-007;
- hazard: H-01, reinterpretazione o perdita silenziosa;
- piattaforme: tutte, senza rami specifici di sistema operativo;
- golden test: tutti i sette tipi e tutte le cinque dimensioni;
- test metadata: emissione e lettura di
  `linestring,multipolygon,geometrycollection`;
- test negativi: tipo futuro, duplicato, vecchia forma snake_case e vecchia
  forma PascalCase.

Risultati locali con Rust `1.92.0` in container Linux x86_64:

- 16 test `plenora-core` superati;
- test workspace completi superati;
- Clippy workspace completo superato con warning negati;
- safety gate sui crate `lib` superato;
- formattazione verificata.

Evidenza remota acquisita per il commit di implementazione
`0d1b0623490947af1e4a907521280cadaed8e50b`: GitHub Actions run
`30280993732`, conclusione `success` per i job `rust`, `windows`,
`macos-publish` e `coverage`.

## Residui

- nove tipi canonici non sono ancora rappresentati dall’enum;
- il modello `types_declaration` di §3.4.1 resta proposta e non viene anticipato;
- non sono disponibili revisione indipendente o qualificazione degli strumenti.

Questa evidenza non costituisce certificazione né dichiarazione di conformità
DO-178C.
