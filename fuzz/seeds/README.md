# Provenienza dei semi di fuzzing

I semi sono l'ingresso minimo perché un target su formato contenitore superi il
controllo del magic e raggiunga il parser vero; `scripts/fuzz-smoke.sh` copia
`fuzz/seeds/<target>/` dentro `fuzz/corpus/<target>/` prima di ogni corsa.

Questo file sta nella **radice** di `fuzz/seeds/`, non dentro una cartella di
target: la copia è per-target, quindi la documentazione non finisce mai nel
corpus come se fosse un input.

La maggior parte dei semi è costruita a mano ed è autoesplicativa. Quelli che
arrivano da un finding no: valgono come riproduttore di un difetto, e un
riproduttore senza provenienza né digest è un file che nessuno sa più perché
c'è. Sono elencati qui.

| Target | File | SHA-256 | Origine | Difetto |
|---|---|---|---|---|
| `geoparquet_reader` | `arrow-schema-che-fa-panicare.parquet` | `e375abe906204651cd5d20b09ba4df5c276b7706ae688d322a6263a83ddfc3d5` | campagna fuzz 2026-07 | `arrow-ipc` `convert.rs:354`, panic su `Precision` sconosciuta; apache/arrow-rs#10575 |
| `geoparquet_reader` | `arrow-schema-che-fa-panicare-2.parquet` | `062e16f5dba2b90eeb35d2ff98c478debd2414a6b43784bfb5c009c5b8f00519` | campagna fuzz 2026-07 | come sopra, secondo percorso |
| `xlsx_reader` | `riferimento-cella-oltre-u32.xlsx` | `cc7be666dd512c6a34208c7a84bc5c8617ea9bec914c08a9aefb603a9567295a` | smoke fuzz 2026-08-17, mutazione di `minimal.xlsx` | `calamine` 0.36.1 `xlsx/mod.rs:2838`, overflow di `u32` sull'accumulo base-26 della colonna (XLSX-HARDENING) |
| `geoparquet_reader` | `pagina-oltre-il-chunk.parquet` | `91f3e9c17af6c740ebde6c36d717fe83f8969cc30caa1e3fb082729418a4347e` | costruito 2026-08-18 da un GeoParquet valido modificandone **un solo campo** a lunghezza netta invariata; lo script sta per intero in `docs/assurance/UPSTREAM_PARQUET_PAGE_ALLOCATION.md` | `parquet` 59.1.0 `serialized_reader.rs:447`: l'allocazione della decompressione segue l'header di pagina, non il totale del chunk. Sotto `RLIMIT_AS` stretto l'esito e' un **abort**, non un errore (FZ-0.2) |
| `xlsx_reader` | `riferimento-cella-nove-lettere.xlsx` | `3786e8fd6d28edcc4ede49ed7a18b063fe685be3ee145b9ab96cd614030ed440` | costruito da zero (FZ-0), CRC corretti | `calamine` 0.36.1, riferimento `AAAAAAAAA1`: serve perché il seme mutato ha il CRC rotto e verrebbe fermato prima del controllo sui limiti |
| `gpkg_reader` | `wkt-multibyte-che-fa-panicare.gpkg` | `581797dae0cd8bcb5dd2359257e92a2875c4d90da15e6440dd8cef9e0b4ccb9b` | smoke fuzz 2026-08-20, checkpoint di livello 2 su `8d6883f` | **difetto nostro**, non di una dipendenza: `plenora-io-model` `crs.rs:180`, `wkt_root_epsg` scorreva indici di byte e affettava la stringa — `upper[index..]` panica quando l'indice cade dentro un carattere multi-byte. Chiuso dallo stesso commit che aggiunge questo seme |
| `geoparquet_reader` | `bit-width-dizionario-fuori-intervallo.parquet` | `962759c9ab3f1f1dc2ccc6eb7fe37a054c5776b7c50e4a7ed7ec8b8a528fb972` | fuzzing attivo 2026-08-17 | `parquet` 59.1.0 `arrow/decoder/dictionary_index.rs:46`, bit width degli indici di dizionario non validato — **aperto**, vedi `fuzz/quarantine.txt` |

## `riferimento-cella-oltre-u32.xlsx`

5.428 byte. Prodotto dallo smoke di `scripts/fuzz-smoke.sh` del 2026-08-17
(37.345 esecuzioni, 268 unità nuove) come mutazione in-place di
`minimal.xlsx` — stessa lunghezza, byte cambiati dentro il flusso deflate di
`xl/worksheets/sheet1.xml`.

`cargo fuzz tmin` **non** riesce a ridurlo: è un archivio ZIP, e togliere byte
rompe il contenitore prima di arrivare al parser, quindi il crash sparisce
invece di restare su un input più piccolo. Le 5.428 byte sono già il minimo che
lo strumento sa produrre.

Il foglio contiene riferimenti di cella come `r="Bncasufw"`: otto lettere, che
`calamine` accumula in base 26 (`col = col * 26 + …`) fino a 20.424.890.639,
oltre `u32::MAX`. Il CRC del membro non torna più — la mutazione è dentro il
flusso compresso — ma `calamine` non verifica il CRC e arriva comunque al
parser.

Il difetto è mitigato nel driver: `driver-xls` avvolge le chiamate a `calamine`
in `leggendo_calamine` e restituisce un errore tipizzato di fase `Read`. La
mitigazione è verificata da
`un_xlsx_che_fa_panicare_calamine_diventa_un_errore_del_driver`, non dal
fuzzing: sotto `libfuzzer-sys` il panico diventa `abort()` prima
dell'unwinding, quindi il target resta rosso anche a barriera funzionante ed è
elencato in `fuzz/quarantine.txt`.
