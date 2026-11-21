# plenora-IO-tools

<!-- generato da codice/versione: inizio -->

Versione del prodotto: **10.1.0**.

| Artefatto Python | Nome |
| --- | --- |
| wheel | plenora_io-10.1.0-py3-none-any.whl |
| sdist | plenora_io-10.1.0.tar.gz |

<!-- generato da codice/versione: fine -->

Le modifiche per versione sono nelle [note di rilascio](docs/RELEASE.md).

Libreria e CLI per leggere e scrivere dati geospaziali in dieci formati,
attraverso un modello semantico unico basato su Arrow.

Ogni formato è un driver dietro la stessa interfaccia. Chi legge non deve sapere
se dietro c'è un CSV o un GeoPackage: ottiene `RecordBatch` Arrow, un contratto
di layer che dichiara schema, geometria e CRS, e un report di ciò che il formato
non ha saputo rappresentare.

```
release_authorized: false
```

Il componente **non è autorizzato al rilascio**. Le condizioni ancora aperte, e
che cosa serve per chiuderle, sono in [docs/RELEASE.md](docs/RELEASE.md).

## Formati

<!-- generato da codice/formati: inizio -->

| Id | Estensioni | Direzione | Feature | Lettura nativa | Lettura effettiva | Pruning spaziale / predicati | Fedeltà | Tipi scrivibili |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| csv | ["csv"] | bidirectional | assente | streaming_sequential | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |
| dxf | ["dxf"] | bidirectional | assente | materialized | streaming_sequential | none / none | approximating | ["boolean","signed_integer","unsigned_integer","floating","utf8","binary","temporal","decimal","nested","other"] |
| filegdb | ["gdb"] | bidirectional | gdal-backend | streaming_sequential | materializing | none / none | conditional | ["signed_integer","floating","utf8"] |
| geojson | ["geojson","json"] | bidirectional | assente | streaming_sequential | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |
| geoparquet | ["parquet"] | bidirectional | assente | streaming_random | streaming_columnar | bounding_box_statistics / numeric_min_max_statistics | lossless | ["boolean","signed_integer","unsigned_integer","floating","utf8","binary","temporal","decimal","nested","other"] |
| gpkg | ["gpkg"] | bidirectional | assente | streaming_random | streaming_sequential | optional_rtree_index / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","binary"] |
| ipc | ["arrow","arrows"] | bidirectional | assente | streaming_random | streaming_sequential | none / none | lossless | ["boolean","signed_integer","unsigned_integer","floating","utf8","binary","temporal","decimal","nested","other"] |
| kml | ["kml","kmz"] | bidirectional | assente | materialized | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |
| shp | ["shp","shp.d"] | bidirectional | assente | streaming_sequential | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |
| xls | ["xlsx"] | bidirectional | assente | materialized | streaming_sequential | none / none | conditional | ["boolean","signed_integer","unsigned_integer","floating","utf8","temporal"] |

<!-- generato da codice/formati: fine -->

Senza la feature `gdal-backend` il driver FileGDB resta uno stub tipizzato: le
sue chiamate falliscono con una capability mancante, non con un errore di
ambiente.

## Uso come libreria

Il consumatore esterno risolve le dipendenze senza copiare il `Cargo.lock`
del workspace. Il gate della superficie Rust verifica questa costruzione
dall'archivio sorgente.

```rust
use plenora_io_core::driver::{FormatDriver, ReadOptions, Source};
use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadRequest, ReadScope};
use plenora_io_model::budget::{PipelineBudget, PipelineLimits};
use plenora_io_model::contract::LayerId;

let bundle = PipelineBudget::builder().limits(PipelineLimits::default()).build()?;
let opzioni = ReadOptions::from_read_parts(bundle.into_read_parts());

let dataset = driver_geojson::GeoJsonDriver.open(Source::Path(percorso), opzioni)?;
let contratto = &dataset.layers()[0];

let mut reader = dataset.open_layer_reader(&ReadRequest {
    layer: LayerId(0),
    projected_fields: None,
    projection_mode: ProjectionMode::BestEffort,
    pruning_predicate: None,
    spatial_pruning_hint: None,
    scope: ReadScope::Complete,
    batch_target: BatchTarget::default(),
    cancellation: Default::default(),
})?;

while let Some(batch) = reader.next_batch()? {
    // RecordBatch Arrow, con la geometria nella colonna dichiarata dal contratto
}
```

L'API Rust è **interna e instabile**: non porta garanzia semver e i crate non
sono pubblicati. La superficie con garanzia di compatibilità è il JSON della
CLI.

## Uso come binding runtime

`plenora_io_tools::runtime` è la superficie Runtime Binding 1.0 delle sei
operazioni (`plenora.io-tools#io.*@1`, discovery
`plenora.io-tools#capabilities@1`). L'applicazione che ospita il binding —
runtime-tools o un'altra — consegna un'`Invocazione` serializzata e riceve un
`Risultato` con la correlazione dell'invocazione:

```rust
use plenora_io_tools::runtime::{BindingRuntime, CancellationToken};

let risultato = BindingRuntime::new(&risolutore)
    .invoca_json(&messaggio, CancellationToken::new());
```

- i payload sono quelli degli schemi `*-input-v1`, letti a campi chiusi;
  `source` e `destination` sono **riferimenti opachi** (`artifact://…`), che
  l'applicazione risolve in percorsi locali con un `RisolutoreArtefatti`: un
  percorso locale nel payload è rifiutato prima di raggiungerla, e il file
  materializzato deve portare il nome dell'ultimo segmento del riferimento
  (diversi formati ne derivano il nome del layer, che esce nei risultati);
- `io.read` consegna il flusso Arrow IPC (`application/vnd.apache.arrow.stream`)
  **nel risultato**, con il documento `plenora-io-read-result-v2` accanto
  (`delivered` a `null`: nessuna destinazione è stata chiesta, e la
  serializzazione la dice il content type del risultato), e su questa
  superficie non scrive file del chiamante;
- `io.write` con un artefatto Arrow stream restituisce
  `input.content_type: application/vnd.apache.arrow.stream`: il risultato
  non valida contro `plenora-io-write-result-v2`, che ammette solo il
  contenitore. La deviazione runtime è dichiarata nel manifesto di adozione
  e nel registro dei limiti di `docs/PRODUCT.md`; rientra con `io.write@2`;
- la discovery passa dalla stessa ammissione delle operazioni: selettore
  `capabilities`, versione `1`, contratto d'ingresso
  `plenora-io-catalog-query-v1`, payload `{}`; rende il documento
  `plenora-capabilities-v2` della superficie runtime;
- `plenora.execution.deadline` (ogni grafia RFC 3339 di UTC, frazioni di
  qualunque lunghezza e `23:59:60` compresi, ricondotti per eccesso al
  nanosecondo; uno scostamento diverso da zero o `-00:00` è rifiutato)
  si misura dall'ammissione -- il tempo del risolutore la consuma -- e scade
  come `timeout`, già passata o durante l'esecuzione; insieme a `deadline_ms`
  nel payload è rifiutata. L'orologio si inietta con `con_orologio`. La chiave
  d'idempotenza è rifiutata, perché nessuna operazione la ammette;
- il risultato ha sempre un `plenora.message.id` nuovo, con la richiesta come
  causa; i selettori della richiesta vi si riflettono solo se canonici;
- un errore è un risultato `application/vnd.plenora.error+json` con i quattro
  assi di `plenora-error-v1`, mai un panico.

I vettori `io-read-*` del repository dei contratti girano in
`crates/plenora-io-tools/tests/runtime_vettori.rs`; dove differiscono dal
prodotto, la differenza è dichiarata nel manifesto di adozione.

## Uso come CLI

Gli esempi seguenti usano `esempio.geojson`, una FeatureCollection con un
Point `[1, 2]` e proprietà `nome="esempio"`. Eseguirli in una directory vuota;
le destinazioni devono essere assenti. Il controllo docset prepara questa fixture.

```sh
# esito: {"exit":0,"status":"ok"}
plenora-io catalog
# esito: {"exit":0,"status":"ok"}
plenora-io capabilities
# esito: {"exit":0,"status":"ok","result.format.id":"geojson"}
plenora-io inspect esempio.geojson
# esito: {"exit":0,"status":"ok","result.format":"geojson"}
plenora-io layers esempio.geojson
# esito: {"exit":0,"status":"ok","result.rows_read":1,"result.truncated":false}
plenora-io read esempio.geojson
# esito: {"exit":0,"status":"ok","result.rows_read":1,"result.truncated":true}
plenora-io read esempio.geojson --limit 1
# esito: {"exit":0,"status":"ok","result.rows_read":1}
plenora-io read esempio.geojson --output esempio.arrow
# esito: {"exit":0,"status":"ok"}
plenora-io write esempio.arrow scritto.geojson --to geojson
# esito: {"exit":0,"status":"ok"}
plenora-io convert esempio.geojson convertito.arrow --from geojson --to ipc
# esito: {"exit":2,"status":"error","error.category":"invalid_configuration"}
plenora-io read esempio.geojson --limit 0
```

Ogni comando emette un documento JSON con un contratto versionato. Gli errori
escono su `stdout` come `plenora-error-v1`, con un codice stabile e un exit
code dedicato.

**`read` non stampa le righe.** Attraversa la sorgente applicando l'intero
contratto di lettura e riporta `rows_read`, `batches`, `truncated`, il layer e la
fedeltà: è una validazione che conta, non un dump. I campi del documento sono
esattamente quelli fissati da [`release/cli-protocol-v2.json`](release/cli-protocol-v2.json)
del protocollo corrente.

`read --output` consegna i dati in Arrow IPC; `convert` pubblica nel formato scelto.

### Flag

I flag sono ammessi solo dai comandi pertinenti. `--layer` vale per read,
convert e write; `--limit` e `--output` solo per read, e non insieme.
`--from` vale solo per convert; `--to` per convert e write.
`--durable` e `--out-opt` richiedono una destinazione. Flag ripetuti e
posizionali in eccesso sono `invalid_configuration/CLI_USAGE`; le opzioni di
formato possono essere ripetute solo con chiavi distinte.
`--help` (anche `<comando> --help`), `-h`, `--version` e `-V` sono comandi di
scoperta; `--version` non e un flag da aggiungere a read o convert.

<!-- generato da codice/comandi: inizio -->

| Comando | Flag ammessi |
| --- | --- |
| inspect | --archive-entry, --opt, --in-opt, --assume-crs, --source-stdin, --archive-max-entries, --archive-max-bytes, --archive-max-ratio, --format, --deadline-ms, --max-input-bytes, --max-input-entries, --max-output-bytes, --max-rows, --max-columns, --max-vertices, --max-geometry-components, --max-wkb-cell-bytes, --max-wkb-components, --max-wkb-depth, --memory-bytes, --spill-bytes, --output-expansion-ratio, --decompression-ratio |
| layers | --archive-entry, --opt, --in-opt, --assume-crs, --source-stdin, --archive-max-entries, --archive-max-bytes, --archive-max-ratio, --format, --deadline-ms, --max-input-bytes, --max-input-entries, --max-output-bytes, --max-rows, --max-columns, --max-vertices, --max-geometry-components, --max-wkb-cell-bytes, --max-wkb-components, --max-wkb-depth, --memory-bytes, --spill-bytes, --output-expansion-ratio, --decompression-ratio |
| read | --archive-entry, --opt, --in-opt, --assume-crs, --source-stdin, --archive-max-entries, --archive-max-bytes, --archive-max-ratio, --format, --deadline-ms, --max-input-bytes, --max-input-entries, --max-output-bytes, --max-rows, --max-columns, --max-vertices, --max-geometry-components, --max-wkb-cell-bytes, --max-wkb-components, --max-wkb-depth, --memory-bytes, --spill-bytes, --output-expansion-ratio, --decompression-ratio, --layer, --limit, --output, --durable, --out-opt |
| write | --archive-entry, --opt, --in-opt, --assume-crs, --source-stdin, --archive-max-entries, --archive-max-bytes, --archive-max-ratio, --format, --deadline-ms, --max-input-bytes, --max-input-entries, --max-output-bytes, --max-rows, --max-columns, --max-vertices, --max-geometry-components, --max-wkb-cell-bytes, --max-wkb-components, --max-wkb-depth, --memory-bytes, --spill-bytes, --output-expansion-ratio, --decompression-ratio, --to, --layer, --durable, --out-opt |
| convert | --archive-entry, --opt, --in-opt, --assume-crs, --source-stdin, --archive-max-entries, --archive-max-bytes, --archive-max-ratio, --format, --deadline-ms, --max-input-bytes, --max-input-entries, --max-output-bytes, --max-rows, --max-columns, --max-vertices, --max-geometry-components, --max-wkb-cell-bytes, --max-wkb-components, --max-wkb-depth, --memory-bytes, --spill-bytes, --output-expansion-ratio, --decompression-ratio, --from, --to, --layer, --durable, --out-opt |
| catalog, capabilities | --format json |
| --help, -h; <comando> --help | help testuale |
| --version, -V | busta versione |

<!-- generato da codice/comandi: fine -->

Le quote finiscono tutte in `PipelineLimits` e condividono la stessa disciplina.
Zero e incoerenze sono `invalid_configuration/CLI_USAGE`.
`--memory-bytes` deve essere almeno pari a `--max-wkb-cell-bytes` (64 MiB
per default); per ridurre ulteriormente la memoria ridurre anche quel tetto.
I default sono quelli del modello.

<!-- generato da codice/quote: inizio -->

| Flag / campo | Default | Unità | Ambito | Fasi |
| --- | --- | --- | --- | --- |
| --max-input-bytes | 268435456 | byte | totale sorgente e spool dei decoder | ["read"] |
| --max-input-entries | 10000 | entry | totale sorgente | ["read"] |
| --max-rows | 10000000 | righe | totale operazione | ["read","write"] |
| --max-columns | 4096 | colonne | schema layer e totale del piano di scrittura | ["validate","prepare","read","write"] |
| --max-geometry-components | 16777216 | componenti | totale operazione | ["read","write"] |
| --max-output-bytes | 1073741824 | byte | totale operazione | ["read","write","finalize","commit"] |
| --output-expansion-ratio | 1073741824 | moltiplicatore | uscita / ingresso osservato non vuoto | ["read","write","finalize","commit"] |
| --max-wkb-cell-bytes | 67108864 | byte | per geometria | ["read","write"] |
| --max-wkb-components | 100000 | componenti | per geometria | ["read","write"] |
| --max-wkb-depth | 32 | livelli | per geometria | ["read","write"] |
| --max-vertices | 50000000 | componenti (coordinate e membri) | per geometria; alias storico che stringe max_wkb_components | ["read","write"] |
| --memory-bytes | 536870912 | byte | memoria viva pipeline e pool | ["prepare","read","write","finalize"] |
| --spill-bytes | 4294967296 | byte | spill vivo pipeline e pool | ["read","write","finalize"] |
| --decompression-ratio | 1000 | moltiplicatore | totale archivio compresso | ["read"] |
| concurrent_operations | 64 | operazioni | pool condiviso | ["prepare","read","write"] |
| --deadline-ms | 30000 | ms | operazione; scadenza distinta dalle quote | timeout |

<!-- generato da codice/quote: fine -->

La coerenza della configurazione si verifica prima della lettura:

* `--max-wkb-cell-bytes` non può superare `--memory-bytes`, e la validazione lo
  rifiuta invece di ridurlo in silenzio;
* `--max-wkb-depth` non può superare 32, il limite strutturale del contratto.

Il target di batch (8 MiB) non è una soglia minima di memoria: le prenotazioni
si adattano al budget disponibile. Il superamento effettivo nomina la quota
(`memory_bytes`, `max_rows`, `max_output_bytes`, ecc.) ed è `resource_limit`
nella fase in cui avviene: `read`, `write` o `finalize` per la misura dello
staging prima della pubblicazione. Un rapporto di espansione esplicitamente
ristretto può ancora rifiutare un piccolo GPKG: il messaggio nomina
`output_expansion_ratio`, distinto dal tetto assoluto `max_output_bytes`.
La tabella autoritativa in codice è `plenora_io_model::quota::QUOTAS`, con
nomi, flag, campi SDK/runtime, default, unità, ambito e fasi. Il pool runtime
condivide memoria/spill e aggiunge `concurrent_operations` (default 64).

`--deadline-ms` non è una quota come le altre: quando scade l'operazione si
ferma e l'errore è `timeout` (`DEADLINE_EXCEEDED`, exit code `5`), come vuole
PUBLIC-SURFACES-1.0 SURF-010, anche quando nello stesso momento una quota
risulta superata. La scadenza si controlla un'ultima volta subito prima della
pubblicazione: fra quel controllo e il rename atomico resta una finestra breve
che nessun controllo chiude, e una scadenza che cade lì trova l'output già
pubblicato. Per uno Shapefile pubblicato come file sciolti la finestra è
l'intera sequenza dei rename: superato il controllo, la sequenza si completa,
per non lasciare un set pubblicato in parte. Fino alla 4.1.0 usciva
`resource_limit` (`LIMIT_EXCEEDED`, exit code `4`). Un valore a zero è configurazione non valida, `invalid_configuration`.

### Ctrl+C

Il primo `SIGINT` **annulla in modo cooperativo**: la pipeline lo osserva ai
propri punti di verifica, ritorna un errore `CANCELLED` con exit code `130`, e
il rientro ordinato fa cadere staging e spool senza pubblicare nulla. Non è
un'interruzione immediata, e dentro una chiamata nativa che non ritorna — GDAL,
sul percorso FileGDB — nessuno guarda il token. Il **secondo** `SIGINT` esce
subito, e ciò che lo staging aveva in corso resta dov'è.

### Ambiente

| | |
|---|---|
| `PLENORA_SPILL_DIR` | la directory che ospita l'inode dello spool, per metterlo su un volume capiente o veloce. Il file non ha nome — è scollegato dal filesystem appena aperto — quindi non esistono orfani da spazzare. Se la variabile è impostata ma la directory non è utilizzabile, la creazione **fallisce chiuso**: nessun ripiego su un altro volume |

### Piattaforme

Ciò che la CI misura, e nient'altro.

| | build e test | note |
|---|---|---|
| Linux x86-64 | suite completa, fuzz, copertura, tutti i gate | è la piattaforma di riferimento; FileGDB in scrittura richiede GDAL ≥ quella di Ubuntu 24.04 |
| Windows x86-64 | `cargo test --workspace --all-targets`, FileGDB con GDAL nativo, publish cross-volume | il gestore di Ctrl+C c'è; la sonda che invia il segnale no, perché su Windows si applica a un gruppo di console e non a un PID |
| macOS | non supportata | nessuna compilazione, prova CI o distribuzione |

Il **recovery** di un publish Shapefile interrotto è in
[docs/PRODUCT.md § Publish](docs/PRODUCT.md#publish-che-cosa-diventa-visibile-e-quando).

## Documentazione

| | |
|---|---|
| [docs/PRODUCT.md](docs/PRODUCT.md) | che cosa offre e che cosa promette: driver, opzioni, contratti pubblici, limiti |
| [docs/ENGINEERING.md](docs/ENGINEERING.md) | come funziona e come viene verificato: architettura, pipeline, checkpoint, fuzzing |
| [docs/RELEASE.md](docs/RELEASE.md) | dove siamo e dove andiamo: stato misurato, blocchi aperti, ordine di lavoro |
| [docs/INSTALL.md](docs/INSTALL.md) | installare e migrare: artefatti, profili, scoperta del binario, protocollo corrente |
| [docs/PIANO-4.0.0.md](docs/PIANO-4.0.0.md) | dove andiamo dopo la 3.0.0: scostamenti dai contratti pubblici, priorità, decisioni aperte |
| [docs/PIANO-4.1.0.md](docs/PIANO-4.1.0.md) | maturità del codice a confronto con database-tools, residui, dipendenze e proposte d'interoperabilità |
| [docs/CENSIMENTO-LIBRERIE-2026-09-15.md](docs/CENSIMENTO-LIBRERIE-2026-09-15.md) | che librerie ci sono e perché: uso effettivo, catena di introduzione, condizioni di rimozione |

Lo stato in forma strutturata è
[`assurance/current-state.json`](assurance/current-state.json); `docs/RELEASE.md`
ne riporta i numeri e un gate verifica che coincidano.

## Licenza

Il codice di questo repository -- il binario, i crate del workspace e lo SDK
Python -- è proprietario: [`LICENSE`](LICENSE), «Copyright (c) Plenora ETL. All
rights reserved.», la stessa licenza di plenora-database-tools. Nessun diritto
d'uso, copia, modifica o distribuzione è concesso se non per accordo scritto con
Plenora ETL. I crate dichiarano `license-file`, lo SDK `License: Proprietary`.

I tre crate vendorizzati sotto `vendor/` — `gdal`, `dxf` e `shapefile` —
conservano la propria licenza upstream (MIT) e il proprio file di licenza. La
provenienza di ciascuno è registrata in `assurance/registries/vendor-<nome>-fork.json`,
un gate la confronta con il lockfile, e `scripts/audit-fork.py` verifica che il
testo della licenza sia quello pubblicato da chi l'ha scritta — non che lo
diciamo noi.

Le modifiche e la migrazione della versione corrente sono nel [CHANGELOG](CHANGELOG.md).

### Selezione ed elenco degli oggetti

`layers` e `inspect` elencano anche gli oggetti non leggibili: `status.readable`
vale `false` e `status.error` contiene il motivo tipizzato, senza dati del file.
Senza `--layer`, `read`, `write` e `convert` verso un formato a layer singolo
usano automaticamente solo un dataset con un unico oggetto leggibile; con zero
o piu' di un oggetto totale, anche se uno solo e' leggibile, restituiscono
`LAYER_SELECTION_REQUIRED`, con indici e stati, e richiedono `--layer`.
`convert` verso GPKG o FileGDB copia tutti gli oggetti leggibili e dichiara gli
esclusi con `source_object_not_readable`, conteggio, indice e motivo tipizzato.
`--layer` seleziona l'indice mostrato dall'elenco, inclusi gli oggetti rifiutati.
`xls.sheet` seleziona il foglio per nome esatto anche nella conversione verso
contenitori; un `--layer` diverso e' un errore di configurazione.
KML sceglie automaticamente il documento aggregato: Document, Folder e Placemark
sono viste annidate selezionabili esplicitamente, non copie aggiuntive da esportare.
I nomi delle viste sono univoci anche ignorando le maiuscole ASCII.

Gli indici sono a base zero: GPKG ordina i nomi con confronto binario SQLite;
FileGDB segue gli identificatori del catalogo interno; XLSX l'ordine del workbook.
KML ha all'indice zero il documento aggregato, poi Document, Folder e Placemark
in ordine di apertura XML; DXF ha il model space all'indice zero, poi i blocchi
nell'ordine del file. Un errore locale non elimina l'oggetto e non scala gli indici.
Gli indici descrivono quel contenitore: aggiunte o riordinamenti possono cambiarli.

Su Windows `--durable` pubblica con `published_durability_unconfirmed`: la sincronizzazione della directory non è confermata. Il file è pubblicato; non ripetere alla cieca.
