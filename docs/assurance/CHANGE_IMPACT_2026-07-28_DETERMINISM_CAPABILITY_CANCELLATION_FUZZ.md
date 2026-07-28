# Change impact analysis — determinismo, capability geometriche e cancellazione

Data: 2026-07-28

## Baseline e perimetro

Baseline precedente:
`60151dad391436f1f2b9e242c1b7652ae9ebbb9a`.

L'incremento modifica il modello pubblico dei descrittori, il validatore comune,
i reader materializzanti KML/DXF/XLSX, il codec di uscita GeoJSON, i test di
conformità e la documentazione assurance. Non modifica dipendenze, lockfile,
toolchain, formato WKB/EWKB o chiavi Arrow del contratto dati.

## Modifiche funzionali

### Determinismo dichiarativo

`FormatDescriptor` espone separatamente `read_determinism` e
`write_determinism` usando i quattro valori candidati dell'ICD §12. Tutti i
driver dichiarano conservativamente il livello `semantic`; nessun formato viene
promosso a `byte_for_byte` senza una prova multipiattaforma.

`DriverRegistry::descriptors` ordina per identificatore. Il comando `catalog`
è quindi indipendente dall'ordine di registrazione e dichiara esplicitamente
`byte_for_byte`. Un test serializza due cataloghi e verifica byte e ordine
canonico. Un secondo test scrive e rilegge lo stesso Point due volte attraverso
tutti i nove driver pure-Rust e confronta i `RecordBatch`.

### Tipi geometrici nelle capability

`GeometryWriteSupport` espone il set dei tipi geometrici accettati. Il
validatore comune:

- rifiuta tipi non presenti nel set prima della creazione del writer;
- rifiuta `mixed` sui formati single-type;
- richiede una dichiarazione preventiva dei tipi quando il formato offre un
  sottoinsieme del modello corrente;
- conserva la guardia runtime sui valori WKB come difesa indipendente.

Shapefile dichiara i sei tipi Simple Features rappresentabili, esclusa
`GeometryCollection`. FileGDB dichiara il profilo nativo verificato:
`Point`, `MultiPoint`, `MultiLineString`, `MultiPolygon`. La matrice negativa
deriva i casi dai descrittori reali.

L'aggiunta dei campi cambia lo schema del catalogo: `descriptor_version` passa
da 4 a 5 per i driver pure-Rust e da 6 a 7 per FileGDB.

### Cancellazione bounded dei parser materializzanti

Il core definisce un intervallo comune di 1.024 elementi. KML, DXF e XLSX
osservano il token:

- dopo ogni chiamata sincrona di parsing/apertura della dipendenza;
- all'ingresso della conversione interna;
- ogni 1.024 nodi, entità, geometrie, coordinate KML, righe o celle XLSX nei
  loop controllati da IO-tools.

KML conserva riferimenti ai placemark invece di clonarli prima della
conversione. DXF applica il contatore anche alle entità raggiunte ricorsivamente
dagli `INSERT`, non soltanto a quelle top-level.

Non viene creato un thread abbandonabile: una singola chiamata sincrona della
dipendenza resta non preemptibile, ma il lavoro successivo di IO-tools è
cooperativo e bounded.

### Finding fuzz GeoJSON

Lo smoke con seed `20260728` ha trovato quattro round-trip incoerenti: il writer
emetteva geometrie vuote che il decoder dello stesso driver rifiuta. Il writer
GeoJSON, sia sul modello `geo_types` sia sull'AST WKB lossless, valida ora
ricorsivamente geometrie e coordinate prima di emettere il primo byte.
LineString, Polygon, multipart o collection vuoti sono rifiutati fail-closed;
non vengono trasformati in `null` né pubblicati parzialmente.

I quattro artefatti sono stati riprodotti dopo la correzione. Uno smoke
successivo di 30 secondi ha eseguito 61.860.000 iterazioni senza finding.
Questa è una regressione locale, non la campagna WKB/EWKB coordinata.

## Compatibilità e failure mode

Lo schema JSON del catalogo cambia in modo additivo, ma l'aggiunta di campi a
strutture Rust pubbliche è source-breaking per eventuali consumatori esterni
che costruiscono `FormatDescriptor` o `GeometryWriteSupport` con struct
literal. L'incremento della versione del descrittore rende il cambiamento
osservabile. L'ordine del catalogo diventa canonico; i consumatori che trattano
l'array come insieme non cambiano semantica.

La validazione geometrica restringe soltanto casi che il writer avrebbe
comunque rifiutato più tardi. Il fallimento si sposta prima dell'I/O ed espone
`CapabilityReason::GeometryNotSupported`.

Le geometrie GeoJSON vuote erano già rifiutate in lettura. La scrittura ora
applica la stessa regola prima di produrre byte, eliminando un output non
riacquisibile.

## Prestazioni

Ambiente: stesso container Linux, Rust 1.92.0 release, geometria Point,
100.000 righe, tre ripetizioni. Baseline locale:
`target/cancellation-periodic-before.json`; post:
`target/cancellation-periodic-after.json`. Veto: 5% su throughput e RSS.

| Driver/operazione | Throughput | Picco RSS | Esito |
|---|---:|---:|---|
| DXF read | -2,33% | +0,10% | OK |
| DXF write | -0,80% | -0,06% | OK |
| KML read | +3,54% | -0,03% | OK |
| KML write | +5,02% | -0,11% | OK |
| XLSX read | +7,37% | +0,17% | OK |
| XLSX write | -2,25% | -0,04% | OK |

Il confronto automatico ha superato il veto.

Una seconda revisione ha esteso i controlli ai loop annidati senza modificare
alcun writer. Il campione post-finale è
`target/cancellation-periodic-after-deep-repeat.json` (cinque ripetizioni):

| Driver/read | Throughput vs baseline pre-change | Picco RSS | Esito |
|---|---:|---:|---|
| DXF | -4,89% | +0,09% | OK |
| KML | +24,73% | -29,23% | OK |
| XLSX | -4,51% | -0,18% | OK |

Nello stesso campione i writer KML/DXF, il cui codice non è cambiato nella
seconda revisione, hanno oscillato oltre il 5% rispetto alla prima misura.
Questo segnala rumore dell'host e impedisce di usare quei run per attribuire una
regressione al reader; il gate dei percorsi modificati resta superato, mentre
la ripetibilità assoluta del benchmark richiede un runner isolato.

## Evidenze di verifica

- `cargo test --workspace --all-targets --all-features --locked`: superato,
  incluso FileGDB con GDAL 3.10.3 (21 test più i 2 helper invocati dai test di
  crash e ownership);
- `cargo clippy --workspace --all-targets --all-features --locked -- -D
  warnings`: superato;
- gate Clippy delle librerie con divieto di `unsafe`, `unwrap`, `expect`,
  `panic`, `unreachable`, `todo` e `unimplemented`: superato;
- gate CI per pin delle Action, identità pubbliche, provenienza RC, dipendenze
  esatte, grafi Cargo locked e registro dei fallback: superati;
- registro H-01 invariato a 88 fallback revisionati;
- replay dei quattro finding: superato; secondo smoke da 61.860.000 iterazioni:
  zero finding.

La prova FileGDB su GDAL 3.10.3 integra, ma non sostituisce, la matrice CI che
usa la propria versione pinned del backend.

## Hazard e controlli

- H-01: capability geometriche complete e rifiuto delle geometrie vuote
  impediscono perdita o reinterpretazione tardiva;
- H-02: la validazione GeoJSON precede l'emissione e il publish resta a
  `finish`;
- H-03: i controlli cooperativi limitano il lavoro proprio fra due osservazioni
  del token;
- H-07: il catalogo ha ordine canonico e livello di determinismo esplicito;
- H-08: test trasversali, replay dei finding e smoke con seed fisso;
- H-09: baseline, impatto, prestazioni e residui sono registrati qui.

## Residui

- FileGDB dichiara il determinismo semantico ma richiede verifica
  multipiattaforma con backend GDAL;
- la singola chiamata sincrona interna a KML/DXF/XLSX non è preemptibile;
- la conversione interna di una singola primitiva DXF con un numero avversario
  di vertici non osserva ancora il token all'interno della primitiva;
- i tipi geometrici del modello locale restano 7 dei 16 canonici;
- la campagna WKB/EWKB condivisa, la revisione indipendente, MC/DC e la matrice
  di filesystem reali restano fuori da questo incremento.

Questa evidenza non costituisce certificazione né dichiarazione di conformità
DO-178C/ED-12C.
