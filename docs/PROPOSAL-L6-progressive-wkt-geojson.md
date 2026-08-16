# Proposta tecnica L6 — Parser progressivo WKT e GeoJSON

Stato: **proposta**. Documento non normativo destinato a supportare la
decisione tecnica del lotto L6 di `ROADMAP-1.1.0.md`. Non modifica il
codice.

## Contesto

Il fix base della finding #6 della review 2026-08-15 (chiuso) applica un
cap sulla **dimensione in byte** della cella WKT o del payload GeoJSON
prima di invocare il parser dei rispettivi driver:

- `driver-common::wkt_lossless::parse_wkt_bounded(text, max_bytes)` per
  CSV/XLSX;
- intercettazione via `serde_json::value::RawValue` sul field
  `geometry` della singola Feature per GeoJSON.

Questo blocca l'ordine di grandezza peggiore (celle da centinaia di MiB)
ma **non** applica in-parse `max_components` e `max_depth`. Il parser
`wkt 0.14.0` costruisce l'AST completo prima di restituire; il
`serde_json` deserializer materializza l'intera `GjGeometry` prima che
la conversione in WKB veda il risultato. Fra il cap byte e il rifiuto
downstream la libreria alloca comunque un albero completo con
conseguente pressione sul memory budget.

Ambito del lotto L6: **applicare `max_components` e `max_depth` in
streaming, prima di ogni allocazione ricorsiva**.

## Requisiti

R1. Il parser rifiuta la geometria appena il conteggio cumulativo dei
    componenti (coordinate + sub-geometrie) supera `max_components`,
    senza aver mai allocato un contenitore piu' grande della soglia.

R2. Il parser rifiuta la geometria appena la profondita' di annidamento
    supera `max_depth`.

R3. Il parser produce lo stesso AST `WkbGeometry` del percorso corrente
    per input validi che rientrano nei limiti: byte-per-byte identico
    all'attuale `geometry_from_wkt` e `wkb_from_gj_value`.

R4. Gli errori sono tipizzati e mappabili sugli assi del contratto
    `PlenoraIoError` (`LimitExceeded`, `Wkb`, `Json`); nessun errore
    porta il valore parziale del payload.

R5. Il costo prestazionale su input validi resta entro il veto
    ereditato dalla campagna prestazionale (`docs/assurance/`), ovvero
    nessun peggioramento mediano sensibile su fixture rappresentative.

R6. Il perimetro `unsafe_code = "forbid"` resta invariato; il gate
    anti-panic sulle librerie resta invariato.

## Opzioni

### Opzione A — Fork del parser WKT + Visitor GeoJSON custom

Descrizione: sostituire `wkt::Wkt::parse` con un parser progressivo
scritto internamente al workspace (nuovo modulo
`driver-common::wkt_streaming`) che emette eventi tokenizzati e
alimenta direttamente il costruttore `WkbGeometry`. Per GeoJSON:
sostituire l'attuale `let g: GjGeometry = serde_json::from_str(raw)`
con una catena di `Visitor` che consuma `MapAccess`/`SeqAccess` e
scrive nel WKB buffer.

Compatibilita': l'output resta identico per input validi (R3). Diverso
mapping degli errori sintattici: il parser interno userebbe messaggi
proprie invece di quelli di `wkt 0.14.0` — richiede aggiornamento dei
test che ne verificano il testo.

Vantaggi:
- controllo totale sui limiti in-parse;
- niente dipendenze aggiuntive;
- possibilita' di usare `#[forbid(unsafe_code)]` sul modulo.

Costi:
- circa 800-1200 righe di codice nuovo per WKT (grammatica ISO WKT
  supportata dal driver corrente);
- circa 400-600 righe per la catena Visitor GeoJSON;
- doppia manutenzione fra AST autoritativo (`plenora-io-model::wkb`) e
  builder streaming;
- rischio regressioni sintattiche non catturate dai test attuali (le
  fixture non coprono tutto il ventaglio ISO WKT).

Rischio: **medio-alto**. Un parser scritto internamente e' una
superficie di attacco geometrica nuova, va coperto con fuzz
coverage-guided dedicato prima di considerarsi affidabile.

### Opzione B — Wrapper contabile sui parser esistenti

Descrizione: mantenere `wkt 0.14.0` e la deserializzazione
`GjGeometry`, ma inserire un allocatore contabile per il thread di
parsing. In pratica:

- un `Allocator` custom che intercetta le allocazioni oltre una soglia
  e restituisce `None`/errore;
- oppure un `Vec` con capacita' massima dichiarata (via `try_reserve`)
  per ogni contenitore che il parser costruisce.

Compatibilita': l'AST resta identico (R3), gli errori diventano
`AllocationFailed` invece di `LimitExceeded`.

Vantaggi:
- niente rewrite;
- costo iniziale basso.

Costi:
- Rust non espone un `Allocator` per-thread stabile (feature
  `allocator_api` nightly). Il fork di `wkt` o del deserializer
  richiederebbe modifiche upstream governate;
- `try_reserve` funziona solo se il codice del parser lo usa;
  `serde_json` NON lo usa, quindi la strada e' bloccata per GeoJSON.

Rischio: **alto**. La feature stabile di Rust non esiste; una feature
nightly renderebbe la libreria non buildabile con la toolchain pinnata
`1.92.0`.

### Opzione C — Pre-scansione + parser esistente

Descrizione: prima di chiamare il parser esistente, eseguire una
pre-scansione lineare del payload che conta componenti e profondita'
massima *senza* costruire l'AST. Se la scansione supera i limiti,
rifiuta prima di allocare.

Per WKT: contare le parentesi aperte per calcolare depth; contare le
virgole per stimare i vertici; contare le keyword `POINT|LINESTRING|
POLYGON|MULTI*|GEOMETRYCOLLECTION` per stimare le sub-geometrie.

Per GeoJSON: scandire il payload JSON contando `{`, `[`, `,` e
riconoscendo `"type"` per distinguere Point/LineString/Polygon/etc.

Compatibilita': completa (R3). Errori nuovi: `LimitExceeded` prima del
parse, distinguibile dal formato malformato.

Vantaggi:
- niente rewrite;
- correttezza garantita: la seconda passata (parser esistente) resta
  quella autoritativa;
- fuzz superficie ridotta (scansione lineare senza stato ricorsivo);
- allineato al pattern gia' usato per il cap byte.

Costi:
- costo lineare aggiuntivo O(N) sul payload per la pre-scansione;
- stima conservativa: se sovrastima leggermente componenti/depth
  rispetto all'AST reale, si rifiuta qualche input che il parser
  avrebbe accettato. Questo va documentato ma e' safe-fail.

Rischio: **basso**. La scansione non alloca, non ricorre, non parsa
semanticamente. Regressioni possibili solo su stime troppo strette;
un test comparativo (scansione vs parse su corpus valido) rileva la
divergenza.

## Impatto prestazionale atteso

| Opzione | Costo su input valido | Costo su input ostile |
|---|---|---|
| A (parser custom) | -5% a +5% (dipende dall'implementazione) | Interruzione precoce, memoria O(soglia) |
| B (allocatore contabile) | +5% a +15% (overhead per-alloc) | Interruzione precoce, memoria O(soglia) |
| C (pre-scansione) | +10% a +25% (una passata extra) | Interruzione precoce, memoria O(soglia) |

Nota: le stime opzione A e C vanno confermate su fixture reali. La
campagna prestazionale del componente ha vietato regressioni sensibili
in passato per KML/DXF/XLSX (vedi
`docs/assurance/STREAMING_READER_DECISION.md`); lo stesso veto si
applica qui.

## Compatibilita' con la CLI e i contratti

Nessun cambio di contratto CLI (`plenora-io-error-v1`,
`plenora-io-inspect-v1`, `plenora-io-read-v1`,
`plenora-io-convert-v1`): l'errore prodotto sarebbe la variante
`LimitExceeded` gia' esistente. Nessun bump `descriptor_version`
richiesto per CSV/XLS/GeoJSON perche' i limiti sono runtime, non
capability dichiarate.

Le nuove categorie di rifiuto (`limits.wkt.components_before_alloc`,
analoghe) andrebbero pero' registrate nel `LossReport` per
osservabilita', se applicabili.

## Raccomandazione

Opzione C. Ratio benefici/rischi migliore per un componente con
`unsafe_code = "forbid"` e gate anti-panic obbligatori. La pre-scansione
si integra col pattern del cap byte gia' presente, non introduce una
nuova superficie di parsing complessa, e la sua correttezza e'
verificabile con un test differenziale su corpus valido.

Se in futuro emergessero corpus reali dove il costo O(N) diventa
critico (payload >100 MiB frequenti), l'opzione A resta la sola strada
per un costo asintotico invariato — ma andrebbe pilotata da misure
concrete, non da speculazione.

## Piano di rollout proposto (se ratificato L6)

1. Implementare pre-scansione per WKT con test differenziale contro
   il parser autoritativo su ~50 fixture note.
2. Implementare pre-scansione per GeoJSON con test analogo.
3. Cablare `max_components` e `max_depth` dei `Limits` (non piu' solo
   `max_cell_bytes`) nei call site di `driver-csv`, `driver-xls`,
   `driver-geojson`.
4. Aggiungere fuzz target dedicato alla pre-scansione (input strutturato
   e input arbitrario).
5. Campagna prestazionale bounded prima del merge; veto scattato =
   revisione o alternativa.

## Fuori scope

- Sostituzione del parser autoritativo `wkt 0.14.0` (non necessaria
  per l'opzione C).
- Sostituzione di `serde_json` (non necessaria).
- Cambio del contratto CLI (nessuna nuova busta JSON richiesta).
