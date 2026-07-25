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
    completo); `DurableAtomicPublish` raccomandato.

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

**5. Destinazioni remote / network fs: fuori scope v1.**

## Conseguenze

- I formati multi-file guadagnano atomicità reale scegliendo la **directory**
  come unità di pubblicazione; i file sciolti restano supportati ma con garanzia
  documentata e più debole.
- **Windows**: rename-over-existing fallisce (coerente col no-clobber, che è
  voluto); share-lock di antivirus/indexer possono far fallire il rename in modo
  transitorio → errore `IO_ERROR` retryable, mai output parziale. Comportamento
  documentato.
- Test obbligatori: publish su Linux e Windows; staging su fs diverso →
  respinto; crash simulato tra scrittura e rename → nessun file parziale;
  `DurableAtomicPublish` con verifica di durabilità.

## Alternative scartate

- **Rename ordinato come default per i set sciolti**: garanzia troppo debole per
  essere il default; relegato al caso in cui il chiamante impone file sciolti.
- **Copy-then-swap**: raddoppia I/O e non è più atomico del rename di dir.
