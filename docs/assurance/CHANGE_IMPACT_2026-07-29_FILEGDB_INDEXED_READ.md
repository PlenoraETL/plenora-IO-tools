# Change impact analysis — lettura FileGDB indicizzata

Data: 2026-07-29

## Decisione

È mantenuta l'ottimizzazione che risolve gli indici OGR una volta per reader e
usa gli accessor tipizzati per indice. È respinta l'alternativa OGRSQL e resta
aperto il pushdown nativo `OGR_L_SetIgnoredFields`.

Il lavoro è successivo al tag immutabile `v0.1.0-rc.1` e costituisce materiale
per un eventuale `v0.1.0-rc.2`; non cambia le evidenze o il claim della RC già
congelata.

## Problema osservato

`Feature::field(name)` di `gdal 0.17.1` chiama
`field_idx_from_name` per ogni cella prima di estrarre il valore. Su dataset
larghi questo ripete ricerca del nome e conversione C per righe × colonne,
benché posizione e tipo del campo siano già noti dal contratto del layer.

La sostituzione diretta con un indice introduce un rischio TOCTOU: il worker
riapre il FileGDB dopo che `open` ha costruito il contratto, quindi un altro
processo potrebbe avere modificato lo schema. Il candidato conserva nome, tipo
Arrow, tipo OGR e indice; sul dataset riaperto convalida nome e tipo di ogni
campo selezionato prima di leggere la prima feature. Qualunque divergenza
produce errore fail-closed.

Il primo prototipo convertiva due errori di conversione indice in assenza con
`.ok()`. Il gate del registro fallback ha rilevato entrambe le nuove
occorrenze (`3` attese, `5` osservate): sono state sostituite con errori
espliciti e il registro è rimasto a 88. Nessun campo richiesto viene quindi
omesso per fallback.

## Alternative respinte

- `OGR_L_SetIgnoredFields` è la primitiva corretta per il pushdown nativo, ma
  non è esposta dall'API safe del pin `gdal 0.17.1`. Chiamare `gdal-sys`
  introdurrebbe `unsafe` nel crate distribuibile e viola il profilo.
- `Dataset::execute_sql` è safe, ma un result-set OGRSQL non è lo stesso layer:
  può modificare FID, identità, gestione della geometria, ordine e piano del
  driver. Non viene adottato come scorciatoia prestazionale.
- L'accesso per indice senza validazione è respinto perché potrebbe associare
  silenziosamente un valore al campo sbagliato dopo una modifica concorrente
  dello schema.

## Protocollo A/B

- baseline di prodotto:
  `ca39d6272b06e290f727b62200ea36cc25d6f826`;
- harness identico:
  `crates/driver-filegdb/examples/projection_bench.rs`;
- fixture: OpenFileGDB, 50.000 righe, geometria Point e 64 attributi `Int32`;
- casi: lettura completa e proiezione di 3 attributi non contigui;
- build: release;
- host: Linux x86_64, kernel WSL2 6.18.33.2;
- toolchain: Rust 1.92.0;
- runtime: GDAL 3.10.3; entrambi i binari usano gli stessi binding C prebuilt
  3.6 di `gdal-sys 0.10.0`;
- campionamento: un warm-up e sette coppie baseline/candidato alternate;
- oracolo: conteggio righe e checksum di tutti gli array letti;
- veto: rimozione se il throughput peggiora in una coppia o se cambia
  l'oracolo.

### Risultati alternati

| Caso | Baseline mediana | Candidato mediana | Tempo | Throughput |
|---|---:|---:|---:|---:|
| geometria + 64 attributi | 787,125 ms | 140,861 ms | **−82,10%** | **+458,8%** |
| 3 attributi non contigui | 114,510 ms | 76,117 ms | **−33,53%** | **+50,4%** |

Il checksum è rispettivamente `80100250000` e `3754675000` per tutti i
campioni, prima e dopo. Nessuna delle quattordici coppie è più lenta nel
candidato. L'intervento supera il veto.

Il benchmark misura throughput su una fixture sintetica larga. Non è una prova
di WCET, latenza di coda, schedulabilità real-time o qualifica di GDAL.

## Impatto contrattuale

- API pubblica: invariata;
- schema e ordinamento delle colonne: invariati;
- valori, nullabilità, geometria e CRS: invariati;
- capability dichiarate: invariate; il pushdown nativo resta non dichiarato;
- dipendenze e formato su disco: invariati;
- memoria bounded: invariata; viene aggiunto soltanto uno snapshot O(numero
  campi) all'apertura del reader;
- safety: nessun nuovo `unsafe`, panic o fallback semantico.

## Hazard

- H-01: accessor tipizzati e test round-trip proteggono valori, null e tipi;
  la convalida dello schema impedisce associazioni silenziose al campo errato.
- H-03: il costo per cella non dipende più dalla ricerca del nome; la memoria
  aggiuntiva è limitata dalla larghezza dello schema.
- H-08: la suite FileGDB feature-on, Clippy all-targets e il benchmark
  ripetibile verificano il cambiamento.
- H-09: baseline, ambiente, veto, risultati e alternative respinte sono
  registrati qui.

## Verifica

Superati sul candidato:

- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`;
- gate safety Clippy su tutti i target `lib`, incluso il divieto di `unsafe`,
  `unwrap`, `expect`, `panic`, `unreachable`, `todo` e `unimplemented`;
- `cargo test -p driver-filegdb --features gdal-backend --locked`
  (22 superati, 2 helper ignorati ed eseguiti dai test padre);
- `cargo test --workspace --all-targets --all-features --locked`;
- `cargo build --workspace --release --all-features --locked`;
- gate release, corpus condiviso, pin delle dipendenze e registro dei fallback
  invariato a 88;
- smoke fuzz strutturato di 15 secondi, seed `20260729`: 29.180.000
  iterazioni, zero finding;
- confronto A/B alternato sopra.

La CI
[`30442548998`](https://github.com/PlenoraETL/plenora-IO-tools/actions/runs/30442548998)
è verde su Linux, Windows, macOS e coverage e copre la revisione di
implementazione `179ad037aad18c3c92ff3c703315a7033ff43773`. Un'eventuale
`rc.2` richiede ancora una decisione di candidato e una nuova baseline
congelata; questa campagna non modifica `v0.1.0-rc.1`.

Questa evidenza non costituisce certificazione né dichiarazione di conformità
DO-178C/ED-12C.
