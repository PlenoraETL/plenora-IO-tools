# ADR-IO 1 — `trait FormatDriver`, ciclo di vita e `WritePlan`

**Stato:** Accettato (Fase 0). Vincola D1, D10, D11 di `Architetture.md`.

## Contesto

I driver hanno stati molto eterogenei (mmap, parser XML pull, connessione
SQLite, handle GDAL, indici, staging dir). Servono: un ciclo di vita univoco per
lettura e scrittura, la collocazione dello stato mutabile, la semantica di più
stream sullo stesso dataset, il comportamento alla cancellazione a metà, e le
regole del `WritePlan` per i contenitori multi-layer.

## Decisione

**1. Ciclo di vita.**

```
Lettura:   open ─→ open_layer_reader ─→ (next_batch)* ─→ drop
Scrittura: create ─→ (write)* ─→ finish            (publish)
                                └─ drop senza finish (abort, nessun publish)
```

- `open(source, opts) -> Box<dyn OpenDatasetHandle>`: **solo statico** (firma,
  struttura, schema, CRS, capability). Nessuna riga letta. L'handle è
  **immutabile e condivisibile** (`&self`).
- `open_layer_reader(&self, request) -> Box<dyn LayerReader>`: apre uno stream
  per un layer. **Lo stato mutabile della lettura vive nel `LayerReader`**
  (cursore, parser, connessione), non nell'handle.
- `LayerReader::next_batch(&mut self) -> Option<RecordBatch>`: pull-based, con
  validazione dinamica per batch; `None` = fine.

**2. Stream concorrenti.** Il descrittore dichiara la concorrenza dei reader con
un enum, più espressivo di un `bool`:

```rust
enum ReaderConcurrency {
    SingleActiveReader,        // cursore/parser unico non-seekable
    MultipleIndependentReaders // seekable: più stream indipendenti
}
```

Un formato **seekable** (file random-access, SQLite) dichiara
`MultipleIndependentReaders`; un formato a **cursore unico non-seekable** (parser
XML/stream su sorgente non riavvolgibile) dichiara `SingleActiveReader`: un
secondo `open_layer_reader` sullo stesso layer/handle, prima che il primo sia
esaurito, è un errore tipizzato, **non** un comportamento indefinito. Default v1:
`SingleActiveReader` (conservativo), tranne dove la seekability è certa.

**3. Cancellazione.** In lettura, **droppare** un `LayerReader` prima
dell'esaurimento è sempre sicuro: nessun side effect osservabile, solo rilascio
degli handle. In scrittura, **droppare un `FormatWriter` senza chiamare
`finish`** è un **abort**: la staging viene ripulita, la destinazione non è mai
toccata. Il publish avviene **esclusivamente** in `finish`, e solo a successo.
Qualunque errore restituito da `write`/`write_to_layer` invalida definitivamente
il writer: ulteriori write e `finish` falliscono, così un batch parzialmente
elaborato non può essere pubblicato.

**4. `WritePlan` (contenitori multi-layer).**

- L'**ordine dei layer nel `WritePlan` è l'ordine canonico** di pubblicazione:
  deterministico, riproducibile.
- **Nomi di layer unici e obbligatori**: due layer con lo stesso nome nel piano
  → errore in `create` (fail-closed), mai silenzioso "ultimo vince".
- **CRS multipli nello stesso contenitore**: ammessi **solo** se il formato
  supporta un CRS per layer (GeoPackage sì); un formato a CRS singolo con layer
  a CRS discordanti → errore in `create`.
- **Un solo dataset-writer coordina tutti i layer** verso **un unico commit
  atomico** (D11): non writer per-layer che pubblicano indipendentemente.
  `finish` scrive tutti i layer e pubblica in blocco, o fallisce lasciando la
  destinazione intatta.

## Conseguenze

- La firma è dinamica ai bordi (`Box<dyn ...>`): c'è **un solo dispatch dinamico
  per batch** (`next_batch`), **mai per riga o per cella**. Lo stato e il parsing
  interni del driver restano monomorfizzati. L'overhead per-batch è trascurabile
  ma non nullo — non lo dichiariamo zero.
- L'atomicità multi-layer è garantita dal dataset-writer unico + `finish`
  (rimanda a ADR-IO 2 per il meccanismo di publish).
- Test obbligatori: apertura di due reader su formato seekable e su formato a
  cursore unico; drop del writer senza `finish` che non lascia residui; ordine
  dei layer deterministico; nomi duplicati respinti.

**Nota di implementazione corrente.** Il gate di conformità centrale esercita
tutti i descrittori reali e, per i nove writer pure-Rust, verifica con una
creazione effettiva sia il no-clobber sia il drop senza `finish`. Il backend
FileGDB feature-on applica la stessa garanzia con una guardia RAII che chiude
prima il dataset GDAL e poi rimuove la directory `.gdb` di staging; copre drop,
batch fallito e limite output prima del publish. Staging univoci e sidecar
lockati distinguono writer attivi e orfani anche tra processi: la suite termina
realmente sottoprocessi durante write e finish, recupera gli orfani al tentativo
successivo e verifica che un writer concorrente attivo non venga cancellato.
Il writer GeoPackage rilascia esplicitamente la connessione SQLite prima del
tempfile, preservando l'abort senza residui anche con i lock file di Windows.
Il wrapper comune invalida ogni writer dopo il primo errore di scrittura e
vieta `finish`. Valida inoltre
piani vuoti, multi-layer non supportati e nomi duplicati. Gli handle sono `Send + Sync` e il
lease atomico comune di `SingleActiveReader` restituisce `ReaderBusy` a un
secondo reader sullo stesso handle, rilasciandosi a EOF, errore o drop
anticipato. La matrice apre due reader reali sui driver pure-Rust single
(KML/DXF/XLSX) e sul caso independent IPC; un test feature-on con GDAL copre lo
stesso rilascio del gate anche su FileGDB. Restano da eliminare le
materializzazioni anticipate in alcuni `open`.

## Alternative scartate

- **Stato di lettura nell'handle** (`read(&self)`): impedirebbe stream
  indipendenti e mal si adatta a cursori/parser mutabili.
- **Writer indipendenti per layer che pubblicano da soli**: romperebbe
  l'atomicità del contenitore multi-layer.
