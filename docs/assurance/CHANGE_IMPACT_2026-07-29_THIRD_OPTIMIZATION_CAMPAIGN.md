# Change impact analysis — terza campagna di ottimizzazione

Data: 2026-07-29

## Scopo e baseline

La campagna interviene soltanto su tre hot path già coperti da contratti e test:
inferenza dello schema GeoJSON, serializzazione WKT del writer CSV e raccolta
dei tipi geometrici del writer GeoParquet.

- baseline di prodotto: `662339d2d20f1a87d0b2465fb95633144ad3ff62`;
- baseline A/B: sorgenti estratti dalla revisione sopra e compilati in una
  directory target separata e immutabile;
- correzione applicata simmetricamente all'harness di baseline: GeoJSON, come
  KML, usa `OGC:CRS84` e non `EPSG:4326`;
- ambiente: Rust 1.92.0, Linux x86_64 in container su kernel WSL2
  6.18.33.2, GDAL 3.10.3;
- carico: 250.000 geometrie Polygon, build `release`, filesystem Linux nativa;
- protocollo: sette coppie prima/dopo intercalate sullo stesso host e sulla
  stessa fixture;
- veto: rimozione dell'intervento se una coppia perde oltre il 5% di throughput
  o se la memoria peggiora materialmente.

La prima esecuzione aggregata aveva mostrato rumore elevato anche su percorsi
non modificati. Non viene usata come prova causale: le decisioni sotto derivano
esclusivamente dal confronto A/B intercalato contro il binario immutabile.

## Difetto rilevato nell'harness

Il benchmark costruiva per GeoJSON un contratto `EPSG:4326`, mentre il formato
ha il CRS fisso `OGC:CRS84`. Il driver di produzione ha rifiutato correttamente
la richiesta prima di scrivere: il finding riguarda il benchmark, non il
driver. `bench_crs` e `bench_contract` sono stati allineati e un test impedisce
la regressione per GeoJSON e KML.

## Interventi mantenuti

### Inferenza GeoJSON

Il precedente `BTreeMap<String, TypeAccumulator>` materializzava una nuova
`String` per ogni chiave di ogni feature prima di scoprire che la chiave era già
nota. `SchemaAccumulators` interna ora assegna un indice stabile a ciascun nome:
la chiave viene allocata una sola volta, gli accumulatori restano contigui e
l'ordinamento lessicografico dello schema viene applicato una volta al termine.
Gli indici incoerenti producono errore fail-closed. Un test con proprietà
presentate in ordine inverso fissa esplicitamente che entrambi gli input
producano lo stesso schema `geometry, a, z`.

| Metrica | Prima | Dopo |
|---|---:|---:|
| Throughput mediano | 483.644 righe/s | 515.404 righe/s |
| Delta mediano accoppiato | — | **+6,59%** |
| Peggior coppia | — | **+5,77%** |
| Allocazioni | 6.750.763 | 5.250.764 |
| Byte allocati | 634,58 MiB | 627,19 MiB |
| RSS di picco | 15,89 MiB | 15,79 MiB |

### Writer CSV

`format_wkt_into` appende la rappresentazione WKT a un buffer riusabile dopo
aver validato la conversione. Il writer CSV riusa il buffer già posseduto per
la riga, evitando la `String` temporanea per ogni geometria. In caso di errore
il contenuto precedente del buffer non viene modificato.

| Metrica | Prima | Dopo |
|---|---:|---:|
| Throughput mediano | 937.987 righe/s | 1.056.098 righe/s |
| Delta mediano accoppiato | — | **+12,59%** |
| Peggior coppia | — | **+8,44%** |
| Allocazioni | 2.504.472 | 1.250.345 |
| Byte allocati | 370,33 MiB | 310,16 MiB |
| RSS di picco | 20,11 MiB | 20,01 MiB |

### Writer GeoParquet

Per produrre il solo metadato `geometry_types`, il writer decodificava
l'intero AST WKB e costruiva una `String` per ogni riga. Ora usa il visitor
strutturale bounded `inspect_wkb`, già confrontato in modo differenziale con il
decoder autoritativo, e conserva nel batch soltanto la coppia
`(GeometryType, CoordinateDimensions)`. Le etichette vengono costruite una volta
per tipo unico e ordinate lessicograficamente, preservando l'output precedente.

| Metrica | Prima | Dopo |
|---|---:|---:|
| Throughput mediano | 2.190.456 righe/s | 3.196.552 righe/s |
| Delta mediano accoppiato | — | **+48,64%** |
| Peggior coppia | — | **+28,53%** |
| Allocazioni | 755.558 | 5.559 |
| Byte allocati | 262,55 MiB | 197,70 MiB |
| RSS di picco | 25,61 MiB | 25,55 MiB |

## Invarianti e impatto

- nessuna modifica all'API pubblica distribuibile, al wire contract, alle
  dipendenze, alla toolchain, alle capability o al formato su disco;
- GeoJSON continua a produrre colonne in ordine lessicografico e usa la stessa
  inferenza monotona;
- CSV conserva WKT dimensionale e precisione `f64`; la validazione precede la
  mutazione del buffer;
- GeoParquet continua a validare l'intero payload WKB, limiti e trailing byte e
  conserva l'ordine lessicografico di `geometry_types`;
- nessun nuovo `unsafe` o panic nel codice distribuibile;
- le misure sono di microbenchmark su un carico Polygon e non costituiscono un
  worst-case execution time né una prova di schedulabilità real-time.

## Hazard

- H-01: test round-trip dimensionali, schema eterogeneo e metadato GeoParquet
  verificano che l'eliminazione delle allocazioni non modifichi i dati.
- H-03: allocazioni e byte allocati diminuiscono in tutti e tre i percorsi; RSS
  non peggiora.
- H-08: ogni modifica ha un test di regressione e la workspace è verificata
  all-features.
- H-09: baseline, protocollo, soglia di veto e risultati sono registrati in
  questa CIA.

## Verifica locale

Superati sul working tree della campagna:

- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`;
- safety Clippy sui target `lib`, incluso il divieto di `unsafe`, `unwrap`,
  `expect`, `panic`, `unreachable`, `todo` e `unimplemented`;
- `cargo test --workspace --all-targets --all-features --locked`;
- `cargo build --workspace --release --all-features --locked`;
- gate pin delle Action, dipendenze, identità pubblica, contratto di release e
  registro dei fallback, invariato a 88;
- smoke fuzz strutturato con seed `20260729`: 28.900.000 iterazioni in 15
  secondi, zero finding;
- confronti A/B intercalati descritti sopra.

La CI candidata precedente non copre questo working tree. Una nuova evidenza CI
potrà essere attribuita alla campagna soltanto dopo commit e run sul nuovo SHA.
La revisione indipendente resta separata e non viene soddisfatta da questa
autoverifica.

## Margini residui

- KML, DXF e XLSX restano materializzanti in lettura per limiti delle API
  upstream; i prototipi KML file-backed già respinti non vanno riaperti senza
  un nuovo disegno e nuove misure.
- Il pushdown nativo delle sole colonne FileGDB richiede una matrice
  multi-versione GDAL.
- Una successiva campagna dovrebbe usare dataset larghi/misti e percentile di
  coda, senza riaprire i contratti durante il freeze.

Questa evidenza non costituisce certificazione né dichiarazione di conformità
DO-178C/ED-12C.
