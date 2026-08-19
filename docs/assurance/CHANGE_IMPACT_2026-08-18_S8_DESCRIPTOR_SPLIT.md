# Change impact analysis — split del descrittore in tre assi ortogonali (INV-7)

Data: 2026-08-18. Sigla: **S8**.
Baseline: `6ea4f00`.

## Problema

`FormatDescriptor` dichiarava un solo `read_mode`, e quel campo conflava tre
cose diverse: cosa fa il parser grezzo, cosa osserva il consumatore, e come è
bounded la memoria. Un consumatore che leggeva `StreamingSequential` non poteva
sapere se il consumo effettivo fosse streaming, spooled o in memoria — è il
corollario del finding L0.4.

## Cosa cambia

Tre campi nuovi, **obbligatori** come `format_options`: un driver che non li
dichiara non compila.

| Asse | Domanda a cui risponde |
|---|---|
| `native_read_mode` | cosa fa il **parser grezzo**, prima di ogni adapter |
| `effective_delivery` | quando il primo batch è visibile, e cosa succede a un errore dopo |
| `buffering` | come è bounded la memoria interna |

`read_mode` resta, **preservato driver per driver, byte per byte**. Non è
derivato dai nuovi campi e non va riallineato a essi: `plenora-io-catalog-v1` lo
emette da sempre, e cambiarlo per farlo «tornare» con `native_read_mode`
romperebbe i consumatori senza aggiungere verità.

### La matrice, con l'evidenza che l'ha determinata

| Driver | `read_mode` (legacy) | `native_read_mode` | Evidenza nel codice |
|---|---|---|---|
| csv | `streaming_sequential` | `streaming_sequential` | `read_record` riga per riga, canale a profondità 2 |
| geojson | `streaming_sequential` | `streaming_sequential` | deserializer serde streaming direttamente nei builder |
| shp | `streaming_sequential` | `streaming_sequential` | una sola `seek` per saltare l'header, poi sequenziale |
| **filegdb** | `materializing` | `streaming_sequential` | `for feature in layer.features()`, iteratore GDAL in avanti |
| **dxf** | `streaming_sequential` | `materialized` | il parser riversa l'intera sorgente in uno spool all'apertura |
| **kml** | `streaming_sequential` | `materialized` | idem |
| **xls** | `streaming_sequential` | `materialized` | vedi sotto |
| **geoparquet** | `streaming_columnar` | `streaming_random` | row group indirizzabili, `SeekFrom::Start` sugli offset del footer |
| **gpkg** | `streaming_sequential` | `streaming_random` | cursore keyset su rowid |
| **ipc** | `streaming_sequential` | `streaming_random` | footer IPC con gli offset dei blocchi |

**Sette driver su dieci divergono** fra legacy e nativo. Non è un difetto da
sanare: è l'informazione che lo split esiste per esporre, e prima non era
osservabile da nessuna parte.

Per **XLS** il riferimento preciso, perché è il caso meno evidente:
`XlsDriver::open` chiama `infer_layout`, la cui firma è

```rust
) -> Result<(XlsxLayout, DataContract, Arc<tempfile::NamedTempFile>)>
```

e il chiamante fa `let (layout, contract, spool) = inferenza?;` **prima** di
costruire `XlsDataset`. Lo spool è quindi completo — l'intero foglio è già stato
consumato da `for_each_dense_row` — quando `open` restituisce l'handle; il
reader che nascerà da `open_layer_reader` legge da quel file temporaneo, non
dalla sorgente. Il parser grezzo ha bisogno di tutto l'input: è `Materialized`.

### Gli altri due assi non differenziano, e il codice lo conferma

Tutti e dieci dichiarano `operation_atomic` e `adaptive_memory_then_disk`, e non
per convenzione: `BudgetedReader` — l'adapter comune attraversato da ogni
lettura — esegue `drain_operation` durante la **prima** chiamata di `next_batch`
e tiene i batch verificati in uno `StagedSpool`. Un driver che dichiarasse altro
starebbe descrivendo un comportamento che l'adapter non gli lascia avere, ed è
il caso che lo snapshot prende.

Aggiungere tre campi al catalogo è un cambio di schema: **`descriptor_version`
aumenta di uno per tutti e dieci** (filegdb 10 → 11, geoparquet 7 → 8, gli altri
otto 8 → 9).

## Due errata al pacchetto decisionale

Entrambe trovate implementando, non prevedendo. Registrate in
`DECISION-PACKAGE-Lotto-0.md`.

**`Materialized` non significa «carica tutto in RAM».** La definizione
originale legava la variante al supporto fisico, e quel legame contraddice
l'ortogonalità che INV-7 esiste per stabilire. La definizione normativa è ora
*consuma o materializza l'intero input prima dell'emissione nativa*; il supporto
fisico lo descrive `buffering` e nessun altro campo. Nessuna quarta variante:
`WholeInputSpooled` reintrodurrebbe dentro un solo valore la conflazione appena
separata.

**FileGDB è `StreamingSequential`.** Il pacchetto lo dava per materializzante,
coerentemente col suo `read_mode`. Era fattualmente errato.

## Verifica

### Tre test, e sono separati apposta

| Test | Cosa prova |
|---|---|
| `il_read_mode_legacy_e_preservato_driver_per_driver` | i dieci valori legacy, uno per uno, invariati |
| `la_tripla_di_inv7_e_quella_dichiarata_da_ogni_driver` | i dieci `native_read_mode`, più `operation_atomic` e `adaptive_memory_then_disk` per tutti |
| `ogni_driver_dichiara_la_tripla_e_il_legacy_puo_divergere` | i tre campi ci sono per ogni driver, e i divergenti sono **sette** |

I primi due sono snapshot **separati** perché provano cose diverse: che S8 non
abbia toccato un campo emesso da sempre, e che la tripla sia quella dichiarata.
Se fossero uno, una modifica al legacy mascherata da aggiornamento della tripla
passerebbe in una diff sola.

Il terzo fissa il numero dei divergenti. Se qualcuno un giorno derivasse
`read_mode` da `native_read_mode`, i sette tornerebbero a coincidere e il campo
tornerebbe a non dire niente — esattamente il difetto L0.4. Il test lo
impedisce contando, non commentando.

Vale la pena dire che **il conteggio ha preso un errore mio**: avevo scritto
«sei» in due posti, incluso il messaggio con cui ho presentato la matrice. Il
test è fallito con `left: 7, right: 6`. È il tipo di errore che un numero
scritto a mano nasconde e un'asserzione no.

## Perimetro e rischi residui

Toccati: `plenora-io-core/src/descriptor.rs` (tre enum, tre campi),
`plenora-io-core/src/lib.rs` (re-export), i dieci driver,
`plenora-io-core/src/{capabilities,driver}.rs` (due descrittori di prova),
`plenora-io-cli/src/main.rs` (tre test), il pacchetto decisionale.

Non toccati: comportamento a runtime — **nessuno**. S8 dichiara ciò che il
codice già faceva. `BudgetedReader`, i parser dei driver e lo spool sono
invariati.

Residui dichiarati:

* **`DeliverySemantics::Streaming` è dichiarabile e non implementata.** Esiste
  perché l'asse la prevede; selezionarla richiederebbe una categoria d'errore
  nuova e un bump del protocollo, non ratificati nel Lotto 0. Nessun driver la
  dichiara, e la variante è `#[non_exhaustive]` come le altre.
* **`BufferingStrategy::Passthrough` e `InMemoryBounded` sono dichiarabili e non
  usate.** La seconda era il comportamento pre-M2 (la `VecDeque`), non più
  selezionabile.
* **`native_read_mode` è una dichiarazione, non una misura.** Nessun gate
  verifica che un driver che dichiara `StreamingSequential` non materializzi di
  nascosto. Un test del genere richiederebbe di osservare il picco di memoria
  del parser isolato dall'adapter, che oggi non è separabile: il residuo è
  dichiarato, non chiuso.
* **INV-14 non è in S8.** Il pacchetto prevede campi privati e `const_new` per
  `FormatDescriptor`; qui i campi restano pubblici come gli altri. È un passo a
  sé, non trascinato dentro questo.
