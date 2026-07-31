# Change impact analysis — fedeltà DBF e costo di lettura Shapefile

**Data:** 2026-07-31  
**Baseline:** ramo `main` successivo a `v1.0.0-rc.2`, revisione
`adf611fe106eb119641442ed47d92f28bd978e44`  
**Ambito:** `driver-shp`; nessuna modifica al protocollo CLI o al formato Arrow
canonico.

## Innesco

Il confronto con lo stack GeoPandas/GDAL su 3.000 particelle catastali ha
isolato due perdite nel bordo DBF e un divario di tempo:

1. un campo DBF `N(18,0)` veniva decodificato dalla dipendenza come `f64`;
   identificativi distinti oltre `2^53` collassavano;
2. due descrittori DBF con lo stesso nome venivano inseriti in una mappa dalla
   dipendenza e una colonna spariva senza dichiarazione;
3. il reader percorreva tutte le geometrie sia durante `open` sia durante la
   lettura effettiva.

I primi due casi ricadono nell'obbligo R5 già ratificato: una perdita non può
restare silenziosa. La sola dichiarazione di precisione non è sufficiente
quando il valore originale è ancora disponibile nel record DBF.

## Decisione

- I descrittori `N` con zero decimali e larghezza almeno 10 sono classificati
  come `Int64`. Il valore viene analizzato direttamente dai byte ASCII del
  record e non dal `f64` prodotto da `dbase 0.5.0`; null, segno e limite `i64`
  sono trattati esplicitamente. Un valore incompatibile con il descrittore
  fallisce chiuso.
- I nomi dei descrittori sono controllati senza distinzione fra maiuscole e
  minuscole prima di costruire `dbase::Record`. Una collisione rifiuta il file
  con errore tipizzato, anziché scegliere o rinominare una colonna.
- L'ordine delle colonne viene dall'header DBF, non dall'ordine non garantito
  della mappa dei record.
- Per Shape XY e M il contratto geometrico deriva dal tipo unico dichiarato
  nell'header `.shp`; la validazione dei singoli record resta nel reader. I
  tipi Z conservano una scansione mirata perché la presenza effettiva della
  misura M non è determinabile dal solo tag nativo.
- `driver_version` Shapefile passa da 8 a 9; `descriptor_version` resta 7.

La lettura raw replica il comportamento della dipendenza sui record cancellati
e sulla variante che omette il deletion flag dalla lunghezza dichiarata. Il
backlink Visual FoxPro viene escluso dal conteggio dei descrittori.

## Verifica

Le regressioni committate verificano che:

- `9007199254740992` e `9007199254740993` escano come due valori `Int64`
  distinti e non producano più il precedente rapporto di precisione;
- due descrittori con lo stesso nome siano rifiutati prima del collasso nella
  mappa;
- i test dimensionali, di streaming, publish e round-trip Shapefile restino
  verdi.

Il test mirato `cargo test --locked -p driver-shp` passa con 19 test. Passano
anche `cargo test --workspace --locked`, Clippy workspace/all-targets con
warning negati, il gate fallback e i gate statici di identità pubblica,
contratto di release, dipendenze e action pin.

## Diagnosi prestazionale

La misura seguente è diagnostica, non evidenza di release: usa una variante
temporanea del generatore patrimoniale esterno con nomi univoci e sole
geometrie regolari. Sullo stesso binario release e sullo stesso bind mount
Windows→Docker:

| Percorso | Prima | Dopo |
|---|---:|---:|
| `inspect` (open/schema) | 0,41 s | 0,05 s |
| `read` (open + 3.000 record) | 0,80 s | 0,45 s |

Gli stessi file copiati prima della misura sul filesystem Linux del container
richiedono 0,02 s per `read`. Il residuo osservato sul mount Windows è quindi
dominato dall'I/O del mount, non dalla validazione WKB o dal contratto. I
numeri non sono promossi a benchmark di rilascio perché il dataset derivato
non appartiene a questo repository; servono a localizzare il costo e a evitare
di attribuirlo al lavoro semantico del reader.

## Residui

- Il writer Shapefile continua a richiedere una verifica separata della
  conservazione di `Int64` oltre `2^53`: il percorso di scrittura della
  dipendenza accetta `Numeric` come `f64`. Questo change chiude il bordo di
  lettura segnalato, non estende implicitamente il claim del writer.
- La qualifica esterna resta legata a un tag immutabile; questo incremento su
  `main` non modifica le evidenze attribuite a `v1.0.0-rc.2`.
