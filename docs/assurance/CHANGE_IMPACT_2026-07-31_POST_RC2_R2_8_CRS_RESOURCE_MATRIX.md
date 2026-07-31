# Change impact analysis — R2.8, CRS semantico e budget post-RC2

**Data:** 2026-07-31  
**Baseline:** `v1.0.0-rc.2` (`9804d775d0d46df9137d44cf0c6963d66a563753`)  
**ICD di riferimento:** `plenora-contracts v2.0-rc14`, revisione
`65fd2c6418efa7937e3063245913d79a80c6499b`  
**Perimetro:** sviluppo successivo alla RC.2; nessun claim trasferito al tag.

## Obiettivo

Chiudere i residui tecnici registrati per R2.8, confronto
`crs_definition`/`crs_id`/SRID e budget R7.5-R7.7; rendere eseguibile una
matrice Linux multi-GDAL/filesystem. Correggere inoltre il riferimento ICD
stale nel gate di sistema.

## Modifiche

1. Il riconoscimento IPC considera sia `ARROW:extension:name=geoarrow.wkb` sia
   le chiavi `plenora.geometry.*`. La validazione resta più stretta del
   riconoscimento: versione schema e quattro chiavi obbligatorie sono richieste,
   l'estensione eventualmente presente deve essere coerente e il tipo fisico
   deve essere Binary/LargeBinary.
2. Il modello estrae soltanto l'EPSG dichiarato alla radice di WKT/WKT2 o
   PROJJSON. Gli ID annidati del CRS base sono ignorati. Il reader dichiara una
   discordanza fra qualunque coppia confrontabile; il writer applica lo stesso
   confronto alla capability `Preserved/Absent/Derived`.
3. `ResourceBudget` e `ResourceLease` condividono contatori atomici e deadline
   fra clone. `convert` passa lo stesso handle a reader e writer. Sono
   contabilizzati memoria, righe, colonne, componenti geometrici, depth,
   concorrenza, output e spill; cell size, fattore di espansione e rapporto di
   decompressione XLSX falliscono chiuso.
4. KML, DXF e XLSX conservano il disegno a singola scansione più spool; lo spool
   entra ora nella quota condivisa. Una scansione in meno non è possibile senza
   uno schema esplicito del chiamante, perché i tre contratti dipendono
   dall'intero insieme di valori/entità.
5. La CI aggiunge Ubuntu 22.04 e 24.04 per FileGDB/GDAL e registra identità GDAL
   e filesystem prima dei test. I risultati non sono dichiarati finché i job
   remoti non sono eseguiti.

## Decisione FileGDB Int64

L'estensione `Int64 -> OFTInteger64` è stata implementata in prova e ritirata.
Nel runtime GDAL/OpenFileGDB 3.6.2 la colonna è stata riaperta come `OFTReal` e
Arrow `Float64`; un valore `9007199254740993` non avrebbe quindi un round-trip
identitario. La capability resta intenzionalmente limitata a
`Int32`/`Float64`/`Utf8`. Non viene dichiarato supporto sulla sola accettazione
del writer.

## Compatibilità e rischio

- Le sei buste CLI v1 non cambiano forma.
- Le API Rust sono dichiarate interne e `publish = false`; `ReadOptions` e
  `WriteOptions` acquistano il budget condiviso.
- R2.8 è ancora proposta nella revisione esaminata: l'implementazione è
  registrata come allineamento candidato, non come ratifica.
- Il resolver non prova equivalenze semantiche prive di ID EPSG radice; questo
  residuo è esplicito e impedisce un claim di resolver CRS completo.
- La contabilizzazione memoria reader è cumulativa e conservativa. È preferita
  a una falsa quota di picco, perché `RecordBatch` può sopravvivere alla chiamata
  successiva e non espone un hook di rilascio al bordo.

## Verifica prevista

- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`;
- `cargo test --workspace --all-targets --locked`;
- suite FileGDB feature-on con GDAL 3.6.2;
- gate assurance/dependency/action pins e `git diff --check`;
- CI remota Linux/Windows/macOS, inclusa la nuova matrice Ubuntu 22.04/24.04.

La qualifica esterna a tre componenti resta di proprietà di
`plenora-contracts/conformance` e deve usare una futura revisione immutabile,
non questo working tree.
