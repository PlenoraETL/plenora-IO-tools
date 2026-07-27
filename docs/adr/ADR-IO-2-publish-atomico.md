# ADR-IO 2 — Publish atomico (file singolo, multi-file, multi-layer)

**Stato:** Accettato (Fase 0). Vincola D7 di `Architetture.md`; coerente con
ADR 7 dei `plenora-data-tools`.

## Contesto

La famiglia garantisce "nessun output parziale mai visibile". Per un file
singolo è risolto (tempfile + `persist_noclobber`). Ma un **set Shapefile**
(`.shp`+`.shx`+`.dbf`+`.prj`) e un **contenitore multi-layer** (GeoPackage con N
layer) devono apparire **o tutti o niente**, e non esiste un rename atomico
portabile di più file separati.

## Decisione

**1. File singolo.** `NamedTempFile` nella **stessa directory** della
destinazione (requisito same-filesystem), poi `persist_noclobber`. Una
destinazione esistente non viene mai sovrascritta.

**2. Multi-file e multi-layer: due modalità esplicite, non equivalenti.**

- **GeoPackage / contenitori single-file multi-layer**: sono comunque un solo
  file → tempfile + `persist_noclobber` (caso 1), tutti i layer dentro.
- Per lo **Shapefile** (e formati a più file) si dichiarano **due modalità
  distinte**, mai presentate come equivalenti trasparenti:

  - **`ShapefileDirectoryDataset`** — atomicità **forte**: si scrive in una
    **staging directory** sullo stesso filesystem e si fa **un solo `rename` di
    directory** (atomico su POSIX/NTFS locale). La directory è l'unità di
    pubblicazione (convenzione Plenora, es. `stazione.shp.d/`). **Richiede che il
    consumatore conosca la directory-dataset**: NON è lo Shapefile tradizionale a
    file sciolti che le applicazioni GIS si aspettano.
  - **`LooseShapefileSet`** — **compatibilità standard**: i file companion
    (`.shp`/`.shx`/`.dbf`/`.prj`) sciolti nella stessa directory, con **rename
    ordinato** (companion prima, `.shp` per ultimo). Atomicità **ridotta e
    dichiarata**: un set di file sciolti **non ha atomicità portabile piena**
    (rischio circoscritto — un lettore che chiave sul `.shp` vede il set
    completo); `DurableAtomicPublish` raccomandato. Ogni singolo passaggio usa
    comunque una primitiva no-replace autorevole: una destinazione creata dopo
    il preflight non viene sovrascritta. In caso di conflitto può essere già
    visibile il prefisso ordinato del set, coerentemente con la garanzia debole.

  La modalità forte NON è un rimpiazzo trasparente dello Shapefile classico: la
  scelta è del chiamante, con i due nomi sopra.

**3. Due profili e sequenza `fsync` completa** (allineata ad ADR 7 data-tools):

- `AtomicPublish`: nessun output parziale visibile (tempfile/staging + rename).
- `DurableAtomicPublish`: la sequenza robusta è, **in quest'ordine**:
  1. `fsync` di **tutti i file** nella staging;
  2. `fsync` della **staging directory** (per i dataset multi-file);
  3. **rename** della staging/del file sulla destinazione;
  4. `fsync` della **directory padre della destinazione** *dopo* il rename —
     necessario per rendere durevole il nome pubblicato.

  Inteso come "le più forti garanzie offerte da filesystem e piattaforma
  supportati", non universale.

**Default v1 = `AtomicPublish`.** La durabilità è **opt-in**: il costo `fsync`
non si paga di default; chi ne ha bisogno lo richiede esplicitamente.

**3b. Esito tipizzato dopo il rename.** Il passo 4 (`fsync` della directory
padre) può fallire **quando il file è già visibile**. Il publish restituisce
quindi un esito esplicito, non un booleano:

```rust
enum PublishOutcome {
    Published,                          // pubblicato (e durabile, nel profilo durable)
    PublishedButDurabilityUnconfirmed,  // già visibile, ma fsync finale fallito
}
```

La documentazione chiarisce che un fallimento **dopo** il rename può lasciare un
output **completo e visibile** ma senza conferma di durabilità: è un'informazione
per il chiamante, non un rollback (l'output è già lì).

**4. Same-filesystem obbligatorio** tra staging/tempfile e destinazione: se
divergono, il rename degenera in copia (non atomico) → errore in `create`
(fail-closed), non un fallback silenzioso.

**5. No-clobber autorevole e staging regolare.** Il controllo anticipato della
destinazione serve solo per restituire un errore tempestivo; l'operazione che
pubblica deve essere essa stessa atomica e no-replace. Linux usa
`renameat2(RENAME_NOREPLACE)` e Windows un move senza
`MOVEFILE_REPLACE_EXISTING`; una piattaforma priva di una primitiva equivalente
fallisce chiusa per le directory. File, directory e loose set rifiutano inoltre
symlink e oggetti non regolari nella staging indipendentemente dal profilo
`durable`.

**6. Destinazioni remote / network fs: fuori scope v1.**

## Conseguenze

- I formati multi-file guadagnano atomicità reale scegliendo la **directory**
  come unità di pubblicazione; i file sciolti restano supportati ma con garanzia
  documentata e più debole.
- **Windows**: il move no-replace fallisce se la destinazione esiste; share-lock
  di antivirus/indexer possono farlo fallire in modo transitorio → errore
  `IO_ERROR` retryable, mai sovrascrittura. Poiché Rust non espone un `fsync`
  portabile della directory padre, un publish richiesto come durable restituisce
  `PublishedButDurabilityUnconfirmed` anche quando i file staged sono stati
  sincronizzati.
- Test obbligatori: publish su Linux e Windows; staging su fs diverso →
  respinto; crash simulato tra scrittura e rename → nessun file parziale;
  `DurableAtomicPublish` con verifica di durabilità.

**Nota di implementazione corrente.** Il driver Shapefile distingue le modalità
anche nel percorso: `*.shp.d` seleziona `ShapefileDirectoryDataset`, contiene
`data.shp/.shx/.dbf/.prj` ed è leggibile direttamente dal driver; `*.shp`
seleziona il `LooseShapefileSet` interoperabile. L'opzione
`publish_mode=shapefile_directory_dataset|loose_shapefile_set`, se fornita,
deve essere coerente con il suffisso e fallisce chiuso altrimenti. Il publish
durable delle directory sincronizza ricorsivamente file e directory prima
dell'unico rename; nessun errore pre-publish di `fsync` viene ignorato. La
validazione ricorsiva rifiuta symlink e oggetti non regolari anche quando
`durable=false`. Il rename finale di directory e ciascun rename del loose set
sono no-replace atomici su Linux e Windows, quindi il preflight non è la barriera
contro il clobber; test deterministici creano file e directory concorrenti dopo
il preflight e verificano che restino intatti. Sono inoltre coperti abort senza
residui e round-trip della directory dataset. FileGDB usa la stessa unità
directory e una guardia
RAII: chiude il dataset GDAL prima della cancellazione e rimuove lo staging su
drop, errore di write o limite output, senza rendere visibile la destinazione.
Ogni writer FileGDB possiede uno staging univoco sul filesystem di destinazione
e un sidecar lockato per tutta la sua vita. Prima di creare un nuovo writer
vengono rimossi soltanto gli staging il cui lock è acquisibile; un lock detenuto
da un altro processo identifica uno staging attivo e non viene toccato. Test in
sottoprocesso terminano forzatamente il writer dopo una scrittura, prima del
rename e subito dopo il rename: nei primi due casi la destinazione resta
assente e lo staging orfano è recuperato al tentativo successivo; nel terzo la
destinazione è completa e leggibile e resta da recuperare soltanto il sidecar.
Un test concorrente cross-process verifica inoltre che il recupero non cancelli
un writer attivo. Il publish condiviso esegue ora un preflight esplicito del
filesystem per file singoli, directory-dataset e loose set: su Unix confronta
il device id, su Windows il prefisso volume/UNC dei percorsi canonici; il rename
resta in ogni caso la seconda barriera fail-closed. La CI Linux esercita davvero
il rifiuto tra il filesystem del runner e `/dev/shm`, verificando che nessuna
destinazione diventi visibile; la CI Windows compila ed esegue lo stesso
contratto e prova un secondo volume scrivibile quando il runner lo espone.
La sincronizzazione durable apre i file staged in lettura/scrittura, requisito
di `FlushFileBuffers` su Windows, prima di rinominarli. Su Windows l'impossibilità
di confermare separatamente la persistenza del nome nella directory padre viene
propagata come `PublishedButDurabilityUnconfirmed`, non assorbita come successo.
Il job Linux installa inoltre GDAL ed esegue la suite FileGDB feature-on.
Resta da validare FileGDB/GDAL nativamente su Windows, estendere la primitiva
directory no-replace oltre Linux/Windows e ampliare la matrice di durabilità a
più filesystem reali.

## Alternative scartate

- **Rename ordinato come default per i set sciolti**: garanzia troppo debole per
  essere il default; relegato al caso in cui il chiamante impone file sciolti.
- **Copy-then-swap**: raddoppia I/O e non è più atomico del rename di dir.
