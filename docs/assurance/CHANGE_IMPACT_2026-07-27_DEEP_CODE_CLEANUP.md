# Change impact analysis — pulizia profonda dei percorsi I/O

Data: 2026-07-27

## Baseline e incrementi

Baseline precedente: commit
`b6eb21d1263a6f429c5b654842db074438f387b6`.

Incrementi analizzati:

- `3b261a42a988be17e041c4e2df3ad27cb7dfc9df` — inferenza e builder
  attributivi comuni;
- `8dda5d778a92caddf1551da88ad62df8b4e57af2` — infrastruttura reader
  separata per responsabilità;
- `45d8ca6c086cb579fa10c81b7ce085a5920382cf` — codec WKB unificato;
- `0a1108ee593e6e2109b3a5ff66c77c8b59a06706` — staging e fallback
  semantici.

Non cambiano dipendenze, manifest, lockfile, toolchain, descrittori di formato
o versioni dei contratti wire.

## Modifiche funzionali

### Attributi inferiti

CSV, GeoJSON e Shapefile usano lo stesso accumulatore monotono e gli stessi
builder Arrow. Un valore non nullo incompatibile con il tipo inferito è un
errore di schema e non viene più convertito in `null`.

Gli interi JSON/CSV fuori da `i64` sono conservati come testo. Se una colonna
mescola un intero `i64` non rappresentabile esattamente da `f64` e numeri
frazionari, l'intera colonna è testuale: nessuna cifra viene persa durante una
promozione implicita.

### Reader

Il protocollo del worker fail-closed e gli adattatori di batch target e
concorrenza sono stati separati da `driver.rs`. Le API pubbliche e il protocollo
osservabile restano invariati; gli eventi terminali continuano a distinguere
successo, errore tipizzato e terminazione anomala.

### WKB

`wkb_lossless` è l'unico parser/encoder binario. `to_wkb`, `to_wkb_into` e
`from_wkb` sono adattatori XY sull'AST autoritativo. È rimossa la seconda
implementazione che ignorava Z/M durante il parsing.

Il vecchio `from_wkb` accettava una geometria valida seguita da byte residui;
ora usa `decode_wkb` e la rifiuta. Test differenziali verificano che l'encoder
XY produca gli stessi byte dell'encoder autoritativo per tutti i sette tipi WKB
classici.

Anche la conversione GeoJSON → WKB costruisce l'AST e usa l'encoder comune,
invece di mantenere un terzo writer binario locale.

### Staging e fallback

Il core espone helper same-directory per staging file, file con suffisso e
directory. Tutti i writer pure-Rust usano questi helper; il lifecycle speciale
FileGDB con lock e recovery resta separato.

Il registro `unwrap_or*` scende da 95 a 82 occorrenze e il gate CI è aggiornato.
Sono eliminate le seguenti reinterpretazioni:

- geometrie GeoJSON/KML senza coordinate non diventano XY;
- coordinate XLSX non numeriche, non finite, incomplete o oltre la precisione
  intera esatta di `f64` non diventano geometria nulla;
- i driver non risolvono più autonomamente un parent di staging assente.

## Compatibilità e failure mode

Le nuove API (`WkbGeometry::from_geo_xy` e helper di staging) sono additive.
Il formato Arrow e i byte WKB prodotti per geometrie XY valide restano
invariati.

La compatibilità di accettazione è intenzionalmente più stretta:

| Input prima accettato | Esito precedente | Esito corrente |
|---|---|---|
| WKB valido con trailing bytes | geometria iniziale accettata | errore WKB |
| GeoJSON/KML vuoto | dimensioni XY inventate | errore di formato |
| XLSX con una sola coordinata X/Y | geometria nulla | errore di formato |
| XLSX con coordinata invalida/non finita/lossy | geometria nulla | errore di formato |
| intero attributivo fuori `i64` | possibile `null` o passaggio lossy da `f64` | testo esatto |
| valore non nullo diverso fra passata di inferenza e lettura | `null` | errore di schema |

L'adattatore `geo-types` costruisce un AST intermedio; non è usato nei percorsi
caldi dei driver correnti, che lavorano direttamente con `WkbGeometry`.

## Hazard e controlli

- H-01: eliminate conversioni a `null`, XY e `f64` non giustificate;
- H-02: lo staging same-filesystem ha un solo punto di creazione;
- H-03: l'inferenza resta O(numero colonne) e i builder restano bounded dal
  batching esistente;
- H-04: il worker conserva panic/disconnessione fail-closed;
- H-08: regressioni differenziali e negative coprono i nuovi confini;
- H-09: questo record collega baseline, commit, impatto e prove.

## Evidenza locale

Ambiente: Rust 1.92.0, Linux x86_64; per FileGDB immagine locale con GDAL 3.6.2.
Le build GDAL usano una target directory isolata per non riusare artefatti
prodotti contro una diversa versione glibc.

- `cargo fmt --all -- --check`: superato;
- registro fallback: 82/82, superato;
- Clippy workspace `--all-targets --all-features --locked -D warnings`:
  superato;
- safety Clippy workspace `--lib --all-features --locked`: superato con divieti
  `unsafe`, `unwrap`, `expect`, `panic`, `unreachable`, `todo`,
  `unimplemented`;
- test workspace `--all-targets --locked`: 169 superati;
- FileGDB `gdal-backend`: 21 superati, 2 helper ignorati ed eseguiti dai test
  di sottoprocesso;
- test reale cross-filesystem `/dev/shm`: superato;
- build workspace release `--locked`: superata.

## Residui

- KML, DXF e XLSX restano materializzanti durante `open`;
- la cancellazione dei worker è cooperativa al successivo invio;
- il modello attributivo condiviso copre i tipi scalari v1, non ancora
  decimal/temporali/binari nativi;
- l'AST geometrico pubblico resta limitato ai sette tipi WKB classici;
- manca revisione indipendente.

Questa evidenza non costituisce certificazione né dichiarazione di conformità
DO-178C/ED-12C.
