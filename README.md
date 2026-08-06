# plenora-IO-tools

Componente Rust di bordo fra i formati su disco e Arrow: legge file geospaziali
e tabellari producendo `RecordBatch`, e li riscrive applicando un piano di
scrittura validato prima di toccare il filesystem. Il publish e' atomico e
avviene solo a successo.

> Versione workspace: `1.0.1`.
> Versione, evidenze e stato di pubblicazione sono registrati nei manifesti
> sotto `release/` e nelle release GitHub; questo README non sostituisce i gate.

## Ruolo nell'ecosistema Plenora

```text
plenora-IO-tools       file e formati  <-> Arrow
plenora-data-tools     Arrow           <-> Arrow
plenora-database-tools database        <-> Arrow
```

I tre componenti comunicano tramite schema Arrow e metadata canonici definiti da
`plenora-contracts`. `plenora-IO-tools` non trasforma i dati: apre la sorgente,
dichiara il contratto del layer e consegna Arrow: le trasformazioni appartengono
a `plenora-data-tools`. Il pin Arrow `=59.1.0` e' comune ai tre componenti,
cosi' i `RecordBatch` passano senza conversione.

## Formati

Ogni driver implementa lettura e scrittura; la scrittura resta subordinata al
capability check statico di ADR-IO 3, che rifiuta prima di aprire il sink un
contratto non rappresentabile nel formato di destinazione.

| Estensione | Driver | Note |
| --- | --- | --- |
| `parquet` | `driver-geoparquet` | GeoParquet |
| `geojson`, `json` | `driver-geojson` | |
| `csv` | `driver-csv` | |
| `gpkg` | `driver-gpkg` | GeoPackage, multi-layer |
| `shp` | `driver-shp` | Shapefile, DBF e WKT proiettato |
| `kml` | `driver-kml` | |
| `xlsx` | `driver-xls` | `.xls` (BIFF) e' una capability drop esplicita |
| `dxf` | `driver-dxf` | fork governato in `vendor/dxf` |
| `gdb` | `driver-filegdb` | OpenFileGDB via GDAL, feature `gdal-backend` |
| `arrow` | `driver-ipc` | Arrow IPC |

## Requisiti

- Rust `1.92` (fissato in `rust-toolchain.toml`);
- dipendenze bloccate da `Cargo.lock` con pin esatti, verificati in CI da
  `scripts/check_dependency_pins.py`;
- GDAL con driver OpenFileGDB solo per `driver-filegdb` (feature
  `gdal-backend`).

Il workspace usa Arrow `59.1.0` con pin esatti. I fork governati di `gdal` e
`dxf` vivono sotto `vendor/` con provenienza registrata nei rispettivi
`PLENORA_FORK.md` e un gate CI dedicato per ciascuno.

## Build e test

```sh
cargo build --workspace --locked
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Il gate anti-panic gira separatamente sul solo codice di libreria, ed e' piu'
stretto della tabella lint del workspace:

```sh
cargo clippy --workspace --lib --all-features --locked -- \
  -D warnings -D unsafe-code \
  -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic \
  -D clippy::unreachable -D clippy::todo -D clippy::unimplemented
```

`plenora-bench` e' l'unico crate con deroga al `forbid(unsafe_code)` di
workspace: e' un harness di misura che richiede un `GlobalAlloc` contatore e
`getrusage`, non appartiene al perimetro spedito e resta fuori dal gate, che
gira su `--workspace --lib`.

## CLI

```sh
plenora-io <catalog|inspect|layers|read|convert> [args]
plenora-io --version
```

```sh
cargo run -p plenora-io-cli -- catalog
cargo run -p plenora-io-cli -- inspect input.gpkg
cargo run -p plenora-io-cli -- layers input.gpkg
cargo run -p plenora-io-cli -- read input.shp --layer 0 --limit 100
cargo run -p plenora-io-cli -- convert input.shp output.parquet
```

Il formato e' dedotto dall'estensione del percorso. Opzioni principali:

- `--assume-crs <crs>`: dichiara il CRS quando la sorgente non lo porta; non
  esiste CRS predefinito e nessuna riproiezione e' implicita (ADR-IO 4);
- `--layer <n>`, `--limit <n>`: selezione e troncamento in lettura;
- `--opt`, `--in-opt`, `--out-opt` in forma `chiave=valore`: opzioni di formato,
  rispettivamente comuni, di ingresso e di uscita;
- `--durable`: forza la durabilita' del publish;
- `--max-input-bytes`, `--max-output-bytes`, `--max-rows`, `--max-columns`,
  `--max-vertices`, `--max-wkb-cell-bytes`, `--max-wkb-components`,
  `--max-wkb-depth`: limiti di risorsa applicati fail-closed.

Ogni comando emette un singolo documento JSON su stdout; gli errori vanno su
stderr come envelope JSON con categoria, fase, effetto remoto e disposizione di
retry, e con `row_diagnostics` quando sono osservate rifiuti riga-scoped. Gli
output esistenti non vengono sovrascritti silenziosamente.

## Contratti e compatibilita'

La superficie congelata per la 1.x sono le sei buste JSON della CLI, dichiarate
in [`release/cli-protocol-v1.json`](release/cli-protocol-v1.json): `catalog`,
`inspect`, `layers`, `read`, `convert` ed `error`. L'API Rust resta interna e
instabile: i crate non sono pubblicati e non offrono garanzia semver.

- Nessun CRS predefinito e nessuna riproiezione implicita.
- Il contratto del layer e' autoritativo: il consumatore non inferisce lo schema
  e la projection dichiarata riflette quella realmente applicata (ADR-IO 6).
- La perdita di informazione e' dichiarata, non silenziosa: ogni lettura e
  scrittura produce una valutazione di fedelta' e un report di perdita
  (ADR-IO 5), separati fra `read_loss` e `write_loss`.
- Il publish e' atomico e avviene solo a successo (ADR-IO 2); la scrittura su
  filesystem diverso dalla destinazione e' rifiutata prima di rendere visibile
  qualsiasi output.

Per il modello completo vedere:

- [`Architetture.md`](Architetture.md);
- [`Prestazioni.md`](Prestazioni.md);
- [`docs/adr/`](docs/adr/);
- [`docs/IMPLEMENTATION_STATUS.md`](docs/IMPLEMENTATION_STATUS.md);
- [`docs/contracts/README.md`](docs/contracts/README.md);
- [`docs/assurance/`](docs/assurance/).

## Release

Lo stato candidato corrente e' dichiarato in
[`release/1.0.1.json`](release/1.0.1.json). I manifesti precedenti restano
record storici immutabili, e le evidenze di freeze vivono sotto
[`release/evidence/`](release/evidence/).

Una release stabile richiede sulla stessa revisione immutabile:

1. CI funzionale verde su Linux, Windows, macOS e sulla matrice GDAL;
2. Clippy, gate anti-panic, `cargo audit` e soglia di coverage;
3. conformance e roundtrip con `plenora-data-tools` e `plenora-database-tools`;
4. aggiornamento del manifesto `release/` e della baseline normativa;
5. revisione indipendente prima del tag.

Il claim corrente e' `verified_internally`: revisione indipendente, RC di
sistema e certificazione avionica non sono dichiarate. Il bump di versione, il
tag e la pubblicazione non sono impliciti nel semplice superamento della suite
locale.
