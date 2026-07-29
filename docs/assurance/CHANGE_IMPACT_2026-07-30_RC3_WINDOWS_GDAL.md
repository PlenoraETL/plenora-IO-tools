# Change impact analysis — GDAL/OpenFileGDB nativo Windows RC3

Data: 2026-07-30

Stato: **candidato respinto; rollback completato**

## Obiettivo

Eliminare la dipendenza da una installazione GDAL Windows implicita e rendere
riproducibile il backend OpenFileGDB nativo nel job Windows. La modifica non
promuove da sola il componente a qualificato su Windows: serve l'esecuzione
verde della matrice e la registrazione dell'ambiente risultante.

## Dipendenze candidate

- `gdal = 0.19.0`;
- `gdal-sys = 0.12.0`;
- `gdal-src = 0.3.0+3.12.1`, con il solo driver
  `driver_openfilegdb` abilitato esplicitamente nel solo harness Windows.

Tutte le versioni dirette restano pin esatti. `gdal-src` vive in un manifest
detached del solo componente FileGDB. Linux e il workspace principale
continuano a collegarsi alla libreria GDAL installata dal runner e non
compilano il bundle durante i normali gate `--all-features`.

Il manifest abilita esplicitamente `gdal-sys/bundled`: la feature opzionale
`gdal/gdal-src` da sola non propaga il bundle al build script low-level.
`gdal-sys` è quindi anche una dipendenza diretta di assurance, pin esatto
`0.12.0`; non viene usato dal codice IO e non introduce chiamate FFI locali.

Un primo tentativo di inserire `gdal-src` nel workspace principale è stato
respinto dal resolver: `proj-sys 0.27.0` richiede `libsqlite3-sys <0.36`,
mentre `rusqlite 0.39.0` del driver GeoPackage usa `libsqlite3-sys 0.37.0`.
Cargo non consente due crate con `links = "sqlite3"` nello stesso grafo. Non si
downgrada GeoPackage e non si altera il suo percorso hot per un requisito
Windows di FileGDB. Il manifest detached evita il conflitto senza separare il
codice sottoposto a test: dipende dal crate `driver-filegdb` per path.

## Motivazione

Il pin precedente `gdal 0.17.1` non contiene binding pre-generati per GDAL
3.10 e non offre un percorso Windows autosufficiente. `gdal 0.19.0` include
binding per le versioni GDAL correnti; `gdal-src` costruisce la libreria nativa
da sorgente e permette di selezionare OpenFileGDB senza affidarsi a un
installer binario non identificato.

La modifica non risolve il pushdown dei campi: l'API safe continua a non
esporre `OGR_L_SetIgnoredFields`. OGR SQL resta respinto perché crea un
result-set, include la geometria nel dialetto OGR e non dimostra il pushdown
fisico nel driver sorgente.

## Hazard

- **H-01 — perdita o conversione semantica:** test round-trip su tipi, null,
  CRS, axis order, Z/M, geometrie e projection devono passare con il nuovo
  binding.
- **H-03 — risorse:** il bundle deve limitarsi ai driver necessari; tempo di
  build, dimensione cache e picco memoria vengono osservati.
- **H-05 — differenze di piattaforma:** la stessa suite FileGDB deve essere
  eseguita nativamente su Windows, non tramite stub.
- **H-08 — dipendenza nativa:** versione, feature, lockfile e provenienza
  devono essere verificabili; nessun download runtime non pinnato.
- **H-09 — evidenza:** un job verde senza `gdal-backend` non soddisfa il gate.

## Criteri di accettazione

1. `cargo check`, Clippy safety e test workspace verdi su Linux.
2. Suite `driver-filegdb --features gdal-backend` verde su Linux.
3. Build e suite FileGDB native verdi su Windows con OpenFileGDB disponibile.
4. Nessun nuovo `unsafe`, panic, fallback semantico o dipendenza diretta non
   pinnata.
5. Benchmark FileGDB RC2 ripetuto; regressione di throughput oltre il 5%:
   rollback.
6. Matrice crash/recovery e filesystem mantenuta separata dai claim di
   certificazione.

## Verifica eseguita

Il manifest detached ha costruito GDAL 3.12.1, PROJ e OpenFileGDB da sorgente
in 7 minuti e 6 secondi su Linux. Il test ha verificato la versione nativa,
l'effettiva disponibilità del driver OpenFileGDB e un round-trip Point
EPSG:3857 attraverso l'API pubblica di `driver-filegdb`; esito: 1 superato,
zero fallimenti. Il Clippy detached con `-D warnings -D unsafe-code` è
risultato verde. La suite FileGDB con GDAL Rust 0.19.0 ha riportato 22 test
superati e 2 helper ignorati/eseguiti dai test padre.

Questa prova dimostra la fattibilità del bundle, non la sua accettabilità
prestazionale.

## Benchmark e veto

Fixture e oracolo sono quelli del benchmark RC2:

- OpenFileGDB con 50.000 righe, Point e 64 attributi `Int32`;
- projection narrow di 3 attributi non contigui;
- stesso runtime di sistema GDAL 3.10.3;
- binari release distinti per `gdal 0.17.1` e `gdal 0.19.0`;
- sette coppie interlacciate, alternando l'ordine;
- checksum atteso e osservato: `3754675000`.

| Variante | Mediana narrow | Delta tempo | Esito |
| --- | ---: | ---: | --- |
| `gdal 0.17.1` | 74,310 ms | baseline | mantenuta |
| `gdal 0.19.0` | 97,918 ms | **+31,77%** | respinta |

Il percorso full aveva mostrato +2,16%, entro il limite, ma il percorso narrow
è quello direttamente interessato dalla projection ed eccede ampiamente il
veto del 5%. Gli oracoli sono invariati: il rollback è prestazionale, non
funzionale.

## Decisione e rollback

Il candidato `gdal 0.19.0`/`gdal-src` è stato rimosso integralmente dal codice,
dal workflow e dai lockfile. Il pin operativo resta `gdal 0.17.1`; nessun
claim Windows viene promosso. Il gate resta aperto nello stato
`bundled_candidate_rejected_performance_veto_environment_open`.

Una futura riapertura richiede un bundle che conservi il percorso narrow entro
il 5%, oppure un aggiornamento upstream accompagnato da un nuovo benchmark
interlacciato. Il test detached sperimentale non resta nel repository perché
dipendeva dalla versione respinta.
