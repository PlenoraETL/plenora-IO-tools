# Plenora — Contratti trasversali

**Documento normativo di interfaccia (ICD) · versione 1.1-draft · 27 luglio 2026**

> **Owner: Marco Bonamente.** Nominato il 27 luglio 2026.
>
> **Stato: parzialmente ratificato.**
>
> **Registro di ratifica**
>
> | Sezione | Oggetto | Stato |
> |---|---|---|
> | §2 | Chiavi metadata canoniche, una per proprietà | **Ratificata** 27 lug 2026 |
> | §3.1 | Sedici tipi geometrici, forma `linestring` | **Ratificata** 27 lug 2026 |
> | §3.2 | Rifiuto esplicito dei tipi non supportati | *non ratificata* |
> | §3.3 | Cinque dimensioni rappresentabili e propagabili | *non ratificata* |
> | §3.4 | `unknown` non degradabile a `xy` | *non ratificata* |
> | §3.5 | Encoding come enumerazione chiusa | *non ratificata* |
> | §4 | CRS a tre stati, axis order non canonicalizzato | **Ratificata** 27 lug 2026 |
> | §9 | Diciannove categorie e dieci fasi d'errore | **Ratificata** 27 lug 2026 |
> | §11.5 | `CancellationToken` concreto e clonabile | **Ratificata** 27 lug 2026 |
> | §15 | Crate `plenora-contracts`, repository proprio, tag git | **Ratificata** 27 lug 2026 |
>
> Le regole non ratificate non sono vincolanti. Restano comunque in vigore per
> ciascun componente le regole che si è dato internamente: in particolare la
> correzione di `plenora-io-core/src/driver.rs:347` è dovuta per PLN-ASR-007 e
> H-01 **a prescindere** dallo stato di §3.4 (vedi §16-bis).
>
> Le sezioni §9 e §11 recepiscono i rilievi del team IO-tools del 27 luglio;
> §5, §9 e §12 recepiscono la fotografia del team data-tools; l'Appendice A
> §R13.3 corregge un dato errato della versione 1.0.
>
> Le sezioni §9 e §11 recepiscono i rilievi del team IO-tools del 27 luglio;
> §5, §9 e §12 recepiscono la fotografia del team data-tools; l'Appendice A
> §R13.3 corregge un dato errato della versione 1.0.

Governa i confini fra i tre componenti Plenora sviluppati separatamente. Le regole
qui contenute prevalgono sulla documentazione locale dei singoli repository.

| Componente | Ruolo nella catena | Repository |
|---|---|---|
| **plenora-IO-tools** | Bordo verso i formati file | `plenora-IO-tools` |
| **plenora-data-tools** | Motore di trasformazione (centro) | `plenora-data-tools` |
| **plenora-database-tools** | Bordo verso i database | `plenora-database-tools` |

---

## Indice

- §0 Come si usa questo documento
- §1 R1 — Versione Arrow
- §2 R2 — Chiavi dei metadati Arrow
- §3 R3 — Modello geometrico
- §4 R4 — Sistema di riferimento (CRS)
- §5 R5 — Perdita di informazione
- §6 R6 — Fallimento invece di panic
- §7 R7 — Limiti di risorsa
- §8 R8 — Identità dei crate e delle colonne
- §9 R9 — Modello di errore attraverso i confini
- §10 R10 — Capability e negoziazione
- §11 R11 — Cancellazione
- §12 R12 — Determinismo e riproducibilità dei risultati
- §13 R13 — Toolchain e baseline
- §14 R14 — Esiti di scrittura e pubblicazione
- §15 Migrazione verso il crate condiviso
- §16 Deroghe e modifiche
- §17 Definizione di componente conforme
- Appendice A — Riepilogo di conformità
- Appendice B — Hazard di riferimento
- Appendice C — Tabelle di rinomina
- Appendice D — Glossario

---

## §0 Come si usa questo documento

**Stato normativo.** Le regole usano `DEVE` / `NON DEVE` / `DOVREBBE` con il
significato consueto: `DEVE` è vincolante e la sua violazione blocca il merge;
`DOVREBBE` ammette deroga motivata secondo §16.

**Autorità.** In caso di conflitto fra questo documento e la documentazione di un
singolo repository (`Architetture.md`, ADR, `IMPLEMENTATION_STATUS.md`), prevale
questo documento. Gli ADR locali possono restringere una regola, mai allargarla.

**Verificabilità.** Ogni regola dichiara come si verifica. Una regola che non si
può verificare meccanicamente è marcata `[ispezione]` e richiede evidenza scritta
nella change impact analysis.

**Aggancio all'assurance.** Le regole sono collegate agli hazard definiti in
`plenora-IO-tools/docs/assurance/TRACEABILITY.md` (H-01…H-09). Un componente che
viola una regola ha un hazard non controllato, non solo un difetto di stile.

**Lettura minima per un team che parte.** §0, la regola che riguarda il proprio
confine, la riga del proprio componente nell'Appendice A, e la tabella di
rinomina in Appendice C.

---

## §1 R1 — Versione Arrow

> **R1.1** I tre componenti **DEVONO** dipendere dalla stessa versione esatta di
> Arrow, dichiarata con pin esatto (`=x.y.z`), non con caret.
>
> **R1.2** La versione corrente della baseline è **`=59.1.0`** per tutti i crate
> `arrow-*` e `parquet`.
>
> **R1.3** Un cambio di versione Arrow **DEVE** essere coordinato: i tre
> componenti cambiano nella stessa finestra, con una change impact analysis unica.

**Perché.** I `RecordBatch` attraversano i confini senza conversione solo se il
layout in memoria coincide. Una divergenza di versione non produce un errore di
compilazione al confine: produce due tipi `RecordBatch` incompatibili che il
sistema dei tipi tratta come estranei.

**Verifica.** `grep -h 'arrow-' */Cargo.toml` — tutte le occorrenze pinnate e uguali.

**Stato oggi: conforme in tutti e tre.**

---

## §2 R2 — Chiavi dei metadati Arrow

Il trasporto delle proprietà geometriche fra componenti avviene tramite i
metadati di campo (`Field::metadata`) dello schema Arrow.

> **R2.1** Le chiavi **DEVONO** appartenere al namespace `plenora.` e usare la
> forma canonica della tabella sottostante. Nessun componente può introdurre un
> nome alternativo per una nozione già presente in tabella.
>
> **R2.2** Le proprietà **NON DEVONO** essere veicolate come blob serializzato in
> una chiave unica: ogni proprietà è una chiave separata, così un consumatore che
> ne ignora una non perde le altre.
>
> **R2.3** Le estensioni specifiche di un formato o di un provider **DEVONO** usare
> un sotto-namespace proprio (`plenora.filegdb.*`, `plenora.postgres.*`) e non
> possono duplicare una nozione canonica.
>
> **R2.4** Un componente che riceve una chiave canonica che non sa interpretare
> **DEVE** propagarla invariata all'uscita. **NON DEVE** eliminarla.

### Chiavi canoniche

| Chiave | Valore | Obbligatoria |
|---|---|---|
| `plenora.geometry.encoding` | `wkb` \| `ewkb` | sì per colonne geometriche |
| `plenora.geometry.dimensions` | `xy` \| `xyz` \| `xym` \| `xyzm` \| `unknown` | sì |
| `plenora.geometry.types` | lista separata da `,` di tipi canonici (§3) | no |
| `plenora.geometry.srid` | intero decimale senza segno | no |
| `plenora.geometry.crs_id` | identificatore d'autorità, es. `EPSG:4326` | no |
| `plenora.geometry.crs_resolution` | `resolved` \| `declared_unresolved` \| `missing` | sì |
| `plenora.geometry.crs_definition` | WKT o PROJJSON, testuale | no |
| `plenora.geometry.axis_order` | `lon_lat` \| `lat_lon` \| `easting_northing` \| `unknown` | sì se `crs_id` presente |
| `plenora.geometry.spatial_semantics` | `geometry` \| `geography` \| `feature_service` | no |
| `plenora.geometry.precision` | `float64` \| `float32` \| `native` | no |
| `plenora.field_id` | intero decimale senza segno | no |

**Perché R2.2.** Un blob unico è opaco: un componente che non sa deserializzarlo
perde tutte le proprietà insieme, e la perdita è silenziosa (H-01).

**Perché R2.4.** Il componente centrale non conosce le estensioni dei due bordi.
Se le elimina, un round-trip file → trasformazione → file perde i metadati nativi
che il driver aveva preservato con cura.

**Verifica.** `grep -rhoE '"plenora\.[a-z_.]+"' crates/ | sort -u` confrontato con
la tabella; test di round-trip dei metadati attraverso il componente centrale.

**Stato oggi: nessuno dei tre conforme.** Vedi Appendice C per le rinomine.

---

## §3 R3 — Modello geometrico

> **R3.1** I tipi geometrici canonici sono i sedici seguenti, serializzati in
> **minuscolo senza separatore**:
>
> `point` · `linestring` · `polygon` · `multipoint` · `multilinestring` ·
> `multipolygon` · `geometrycollection` · `circularstring` · `compoundcurve` ·
> `curvepolygon` · `multicurve` · `multisurface` · `polyhedralsurface` · `tin` ·
> `triangle` · `unknown`
>
> **R3.2** Un componente **PUÒ** supportare un sottoinsieme dei tipi, ma **DEVE**
> rifiutare esplicitamente quelli che non supporta. **NON DEVE** degradarli,
> approssimarli o ignorarli in silenzio.
>
> **R3.3** Le dimensioni canoniche sono `xy`, `xyz`, `xym`, `xyzm`, `unknown`.
> Ogni componente **DEVE** essere in grado di *rappresentare e propagare* tutte e
> cinque, anche quando non sa *elaborare* Z o M.
>
> **R3.4** `unknown` significa «byte preservati, dimensionalità non risolta». Un
> componente **NON DEVE** convertire `unknown` in `xy` per convenienza: è una
> reinterpretazione silenziosa dei dati (H-01).
>
> **R3.5** L'encoding canonico è `wkb` o `ewkb`, come enumerazione chiusa. **NON
> DEVE** essere modellato come stringa libera.

**Perché la forma senza separatore (R3.1).** I valori serializzati vengono
confrontati con i tipi dichiarati dai sistemi esterni — PostGIS, GeoPackage, OGC
WKT — che usano tutti `LINESTRING`, `MULTIPOLYGON`. La forma `line_string`
richiederebbe una traduzione a ogni confine, e ogni traduzione è un punto in cui
si perde informazione.

**Perché R3.3 è la regola più costosa.** Oggi `data-tools` ammette solo `xy`. La
regola non impone di *calcolare* in 3D: impone di non distruggere e di non
descrivere in modo errato ciò che non si elabora.

Il comportamento attuale di data-tools va descritto con precisione, perché non è
una perdita silenziosa e la versione 1.0 di questo documento lo diceva a torto:

1. **Operazioni geometriche su Z/M: rifiuto esplicito.** `geometry_from_wkb`
   respinge con `Unsupported` ogni WKB che porti Z, M o SRID — «non preservabili
   nel protocollo 2D» — e c'è un test dedicato anche al caso annidato in una
   `GeometryCollection`. È fail-closed, quindi conforme a R5.1 e R3.2.
2. **Operazioni tabellari su dati Z/M: i byte transitano.** I kernel tabellari
   non decodificano la colonna geometrica, che resta un buffer opaco. Un filtro,
   un join o un ordinamento su attributi non attivano il rifiuto.
3. **Ma il contratto emesso dichiara comunque `xy`**, perché è l'unica variante
   rappresentabile.

Il punto 3 è il difetto reale, ed è di natura diversa da quella ipotizzata: non
è perdita di dati, è un **metadato che contraddice i byte che accompagna**. Un
consumatore a valle — per esempio database-tools, che decide su
`plenora.geometry.dimensions` come dichiarare la colonna PostGIS — riceve dati
XYZM etichettati come XY. È H-01 nella forma «reinterpretazione», non in quella
«perdita».

**Verifica.** `[ispezione]` sulle definizioni dei tipi; test di round-trip
end-to-end XYZM attraverso i tre componenti; grep di assegnazioni che portano a
`Xy` un valore letto dai metadati.

**Stato oggi:**

| | Tipi | Dimensioni | Encoding |
|---|---|---|---|
| IO-tools | 7, `snake_case` | 5 ✅ | enum ✅ |
| database-tools | 16 ✅, forma ✅ | 4 (manca `unknown`) | `String` ❌ |
| data-tools | assenti ❌ | 1 (`xy`) ❌ | assente ❌ |

**Violazione nota di R3.4** — `plenora-io-core/src/driver.rs:347`. Segnalata dal
team IO-tools e confermata:

```rust
read_geometry_contract_metadata(field, &mut geometry);
if geometry.dimensions == CoordinateDimensions::Unknown {
    geometry.dimensions = CoordinateDimensions::Xy;
}
```

L'intento documentato è il default per i contratti v1 privi di metadati
geometrici, ed è legittimo. Il difetto è la posizione: la conversione avviene
**dopo** la lettura dei metadati, quindi sovrascrive anche un `unknown`
dichiarato esplicitamente. La correzione non è rimuovere il default ma spostarlo
nel ramo in cui i metadati sono assenti, così che i due casi — «nessuna
informazione, si applica il default storico» e «informazione presente, dice
`unknown`» — restino distinti.

Questa correzione **non dipende dalla ratifica del presente documento**: è già
dovuta per PLN-ASR-007 e H-01 nel profilo di assurance di IO-tools.

---

## §4 R4 — Sistema di riferimento (CRS)

> **R4.1** Lo stato di risoluzione del CRS **DEVE** distinguere tre casi:
> `resolved`, `declared_unresolved`, `missing`. Un componente **NON DEVE**
> collassarne due su una stessa rappresentazione, né nel modello interno né nei
> metadati che emette.
>
> **R4.2** L'ordine degli assi **DEVE** essere trasportato esplicitamente e **NON
> DEVE** essere canonicalizzato. `EPSG:4326` (lat/lon) e `OGC:CRS84` (lon/lat)
> **NON DEVONO** essere trattati come equivalenti.
>
> **R4.3** La definizione sorgente (WKT o PROJJSON) **DEVE** essere preservata
> quando disponibile, anche se il componente sa già risolvere l'identificatore.
>
> **R4.4** Nessun componente **DEVE** assumere un CRS di default. In particolare,
> l'assenza di CRS **NON DEVE** essere interpretata come WGS84.
>
> **R4.5** La riproiezione è responsabilità esclusiva del componente centrale. I
> bordi **NON DEVONO** riproiettare: preservano il CRS sorgente e lo dichiarano.

**Perché.** L'inversione lat/lon è il fallimento geospaziale più costoso e più
silenzioso che esista: produce coordinate plausibili in un punto sbagliato del
pianeta, senza alcun errore (H-06).

**Verifica.** `[ispezione]` più matrice di test axis-order per componente.

**Stato oggi:** IO-tools conforme nel modello; `driver-shp` proietta però
`crs.id.unwrap_or("unknown")` nei metadati, collassando *assente* e *irrisolto*
(violazione R4.1 nel dato che esce). database-tools rappresenta il CRS come
`srid: Option<u32>` + `crs: Option<String>`: non può esprimere né i tre stati né
l'axis order. data-tools ha `ResolvedCrs` allineato al modello.

---

## §5 R5 — Perdita di informazione

> **R5.1** Nessuna conversione **DEVE** perdere dati o metadati in silenzio. Ogni
> perdita **DEVE** essere: rifiutata (fail-closed), oppure registrata in un
> report di perdita esplicito restituito al chiamante.
>
> **R5.2** Un valore di default **NON DEVE** sostituire un dato assente quando
> l'assenza è informativa. `unwrap_or(default)` su una proprietà del contratto è
> una violazione salvo motivazione nella CIA.
>
> **R5.3** Un ordinamento, un raggruppamento o un confronto **DEVE** usare la
> semantica nativa del tipo. La conversione a un tipo più povero per comodità di
> confronto è una violazione: `i64` oltre 2⁵³ convertito a `f64` collassa valori
> distinti; `u64` confrontato come stringa ordina `"10"` prima di `"9"`.
>
> **R5.4** Il troncamento numerico implicito attraverso un confine è vietato. Le
> conversioni fra larghezze **DEVONO** usare `try_from` con errore esplicito.

**Perché R5.3 è qui e non nel repository di data-tools.** È la regola che vale
per chiunque manipoli valori attraversando il confine: gli stessi dati devono
ordinarsi allo stesso modo nei tre componenti, altrimenti un risultato dipende da
quale libreria l'ha prodotto.

**Verifica.** Censimento di `unwrap_or`, `unwrap_or_default`, `unwrap_or_else` nei
crate `lib`; test di ordinamento su valori limite (`i64::MAX`, `u64` a più cifre);
grep dei cast `as` su tipi interi.

**Stato oggi:** IO-tools ha 103 `unwrap_or*` nei crate `lib`, non censiti.
data-tools ha corretto R5.3 con `compare_cells_typed`, committato in `14a0a29`;
resta il censimento di `unwrap_or*` e dei cast troncanti (R5.4), che è cosa
distinta dal gate anti-panic e non viene intercettata da esso.
database-tools non ha un report di perdita uniforme.

---

## §6 R6 — Fallimento invece di panic

> **R6.1** Nei crate **libreria**, un input esterno malformato **DEVE** produrre un
> errore tipizzato. **NON DEVE** produrre `panic`, in nessuna forma.
>
> **R6.2** I crate libreria **NON DEVONO** contenere `unsafe`, né le primitive
> `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`.
>
> **R6.3** Il vincolo si applica ai soli target `--lib`. Le asserzioni nei test
> restano un meccanismo di verifica legittimo.
>
> **R6.4** Un'invariante che oggi giustifica un `expect` **DOVREBBE** essere
> codificata nel sistema dei tipi (newtype validato) anziché documentata in un
> messaggio.
>
> **R6.5** La sostituzione di una primitiva di panic **NON DEVE** introdurre un
> default silenzioso. `expect("X")` diventa `ok_or_else(|| errore("X"))?`, non
> `unwrap_or(valore)`: altrimenti si scambia un H-04 rumoroso con un H-01 muto,
> che è peggio.

**Verifica** — identica per i tre componenti, da aggiungere alla CI:

```
cargo clippy --workspace --lib --all-features --locked --
  -D warnings -D unsafe-code
  -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic
  -D clippy::unreachable -D clippy::todo -D clippy::unimplemented
```

**Stato oggi:** IO-tools **0** occorrenze, gate attivo. data-tools **~121** nei
crate libreria, nessun gate. database-tools **26**, nessuna CI.

---

## §7 R7 — Limiti di risorsa

> **R7.1** Ogni operazione che alloca in funzione dell'input **DEVE** applicare un
> limite prima dell'allocazione, non dopo.
>
> **R7.2** I limiti **DEVONO** essere controllati dal chiamante, con un default
> documentato, e coprire almeno: byte totali, righe, colonne, componenti
> geometrici, profondità di annidamento.
>
> **R7.3** Le strutture ricorsive che attraversano un confine (AST, WKB annidato,
> piani) **DEVONO** avere un limite di profondità applicato **prima** della
> ricorsione, e la validazione stessa **DOVREBBE** essere iterativa, per non
> esaurire lo stack nel verificarlo.
>
> **R7.4** I limiti **DEVONO** essere espressi nelle stesse unità nei tre
> componenti: byte per la memoria, righe per il conteggio, millisecondi per il
> tempo. Un limite passato attraverso un confine non richiede conversione.

**Verifica.** `[ispezione]` dei punti di allocazione; fuzzing con input ostili.

**Stato oggi:** IO-tools dichiara R7 «Parziale» (PLN-ASR-004). database-tools
applica profondità e nodi all'AST tramite `validate_query_operation`, iterativa e
quindi conforme a R7.3. data-tools valida il framing IPC prima di delegare ad
arrow-rs.

---

## §8 R8 — Identità dei crate e delle colonne

> **R8.1** Due crate distinti **NON DEVONO** avere lo stesso nome di pacchetto.
>
> **R8.2** L'identità logica di una colonna è `FieldId(u32)`, stabile attraverso
> le rinomine e assegnata in un namespace globale del grafo.
>
> **R8.3** I tipi di confine **DEVONO** vivere in un crate condiviso, non essere
> ridefiniti in ciascun componente.
>
> **R8.4** Due tipi pubblici omonimi con forma diversa **NON DEVONO** esistere nel
> perimetro Plenora.

**Stato oggi: violazioni aperte.** `plenora-IO-tools/crates/plenora-core` e
`plenora-data-tools/crates/plenora-core` sono due pacchetti distinti con lo stesso
nome (R8.1), e definiscono entrambi un `PlenoraError` con varianti diverse (R8.4).
Il crate condiviso previsto da R8.3 non esiste.

---

## §9 R9 — Modello di errore attraverso i confini

> **R9.1** Un errore che attraversa un confine **DEVE** portare almeno:
> categoria, fase in cui si è verificato, indicazione di ritentabilità, messaggio.
>
> **R9.2** La ritentabilità **DEVE** essere esplicita e non dedotta dal messaggio.
> Un chiamante non deve interpretare del testo per decidere se riprovare.
>
> **R9.3** L'esito ignoto **DEVE** essere una categoria di prima classe, distinta
> dal fallimento. Un'operazione che non sa se è andata a buon fine **NON DEVE**
> essere riportata come fallita né come riuscita.
>
> **R9.4** I messaggi d'errore **NON DEVONO** contenere credenziali, stringhe di
> connessione, percorsi assoluti dell'ambiente o contenuto dei dati. Il
> riferimento al dato avviene per posizione (riga, colonna, offset), mai per valore.
>
> **R9.5** Le categorie d'errore **DEVONO** provenire da un'enumerazione condivisa.
> Un componente può usarne un sottoinsieme, non può inventarne di proprie.

**Perché R9.3.** È la distinzione che rende un'API onesta e che oggi solo
database-tools modella (`WriteStatus::OutcomeUnknown`, `ErrorCategory::OutcomeUnknown`,
e in IO-tools `PublishedButDurabilityUnconfirmed`). Se il commit di una
transazione fallisce dopo l'invio, l'operazione può essere andata a buon fine: dire
«fallito» invita a ritentare e a duplicare i dati (H-01, H-02).

**Perché R9.4.** Un messaggio d'errore attraversa i confini e finisce nei log.
È il canale attraverso cui i dati sfuggono al perimetro senza che nessuno se ne accorga.

### Enumerazione canonica delle categorie

Base: il modello di `plenora-database-tools`, esteso con le categorie che servono
ai bordi su filesystem e all'esecuzione delle trasformazioni. Ogni componente ne
usa il sottoinsieme che gli compete (R9.5); nessuno ne aggiunge.

| Categoria | Significato | Ritentabile |
|---|---|---|
| `InvalidPlan` | La richiesta è malformata o incoerente | no |
| `InvalidConfiguration` | La configurazione del componente è invalida | no |
| `Schema` | Schema Arrow o contratto dati incoerente | no |
| `DataMapping` | Un valore non è rappresentabile nella destinazione | no |
| `Crs` | CRS assente, irrisolto o incoerente | no |
| `Unsupported` | Capability non offerta dal componente | no |
| `NotFound` | Risorsa, layer o tabella inesistente | no |
| `Conflict` | Destinazione già esistente o conflitto di scrittura | no |
| `Authentication` | Credenziali assenti o rifiutate | no |
| `Authorization` | Permessi insufficienti | no |
| `Timeout` | Scadenza superata | sì |
| `Cancelled` | Annullato dal chiamante | no |
| `ResourceLimit` | Limite di byte, righe, profondità o quota superato | no |
| `Io` | Errore del filesystem o del dispositivo | dipende |
| `Protocol` | Violazione del protocollo di trasporto o di rete | dipende |
| `Transient` | Condizione temporanea, ritentabile per natura | sì |
| `Execution` | Fallimento di un nodo durante la trasformazione | no |
| `OutcomeUnknown` | Esito non determinabile: né riuscito né fallito | mai automaticamente |
| `Internal` | Invariante interna violata | no |

La colonna «ritentabile» è il valore di default suggerito: il campo di R9.1 resta
esplicito per ogni errore, perché la stessa categoria può essere ritentabile o no
a seconda del contesto.

### Enumerazione canonica delle fasi

`Validate` · `Connect` · `Probe` · `Prepare` · `Read` · `Write` · `Finalize` ·
`Commit` · `Rollback` · `Cleanup`

Per i bordi su filesystem, `Connect` copre l'acquisizione dell'handle e del lease
sulla risorsa, `Probe` l'ispezione preliminare del formato, `Commit` il rename
atomico di publish.

### Mappatura dai modelli attuali

| Oggi | Categoria canonica |
|---|---|
| IO `Contract`, data `Contract` | `InvalidPlan` |
| IO `Schema`, data `Schema` | `Schema` |
| IO `Crs`, data `Crs` | `Crs` |
| IO `Wkb` | `DataMapping` |
| IO `LimitExceeded` | `ResourceLimit` |
| IO `OutputExists` | `Conflict` |
| IO/data `Unsupported`, data `UnsupportedPublishTarget` | `Unsupported` |
| IO/data `Io` | `Io` |
| IO/data `Json`, data `Arrow` | `DataMapping` |
| data `Step` | `Execution` |
| data `Cancelled` | `Cancelled` |

**Stato oggi: tre modelli distinti e incompatibili.**

| Componente | Tipo | Categoria | Fase | Ritentabilità | Esito ignoto |
|---|---|---|---|---|---|
| IO-tools | `PlenoraError` enum | implicita nella variante | ❌ | ❌ | solo nel publish |
| data-tools | `PlenoraError` enum + `ErrorCategory` (11) | ✅ | ❌ | ✅ `retryable()` | ❌ |
| database-tools | `DatabaseError` struct | ✅ (15) | ✅ (10) | ✅ | ✅ |

Le categorie di data-tools e database-tools condividono oggi soltanto
`Unsupported` e `Cancelled`.

data-tools soddisfa R9.2 dalla milestone M1: `PlenoraError::retryable()` è una
funzione esplicita con test dedicato, e `execution_id` è portato dalle varianti
`Step` e `Cancelled`. Restano da aggiungere la fase (R9.1) e l'esito ignoto
(R9.3).

---

## §10 R10 — Capability e negoziazione

> **R10.1** Un componente che non può eseguire un'operazione richiesta **DEVE**
> fallire prima di iniziarla, non a metà.
>
> **R10.2** Le capability **DEVONO** essere interrogabili in forma dichiarativa e
> leggibile da un programma, prima dell'esecuzione, non desumibili per tentativi.
>
> **R10.3** Il rifiuto per capability mancante **DEVE** indicare quale capability
> manca, in forma tipizzata, non come messaggio libero.
>
> **R10.4** Una capability assente **NON DEVE** essere sostituita da un
> comportamento approssimato. La degradazione silenziosa è una violazione di R5.1.

**Perché.** Il valore di un modello di capability sta nel fallire *presto*: un
fallimento a metà scrittura lascia lo stato parziale che H-02 descrive.

**Stato oggi:** IO-tools ha `FormatWriteCapabilities` e `CapabilityReason`
tipizzato, con validatore statico prima della creazione. database-tools ha
`ProviderCapabilities` (read, write, transaction, spatial, limits). I due modelli
sono paralleli ma non condividono nulla. data-tools, come componente centrale,
non espone capability: **DOVREBBE** dichiarare almeno quali tipi e dimensioni
geometriche è in grado di propagare, per rendere verificabile R3.3.

---

## §11 R11 — Cancellazione

> **R11.1** Ogni operazione potenzialmente lunga **DEVE** accettare un segnale di
> cancellazione dal chiamante.
>
> **R11.2** La cancellazione **DEVE** essere cooperativa e osservata a intervalli
> limitati: fra un batch e il successivo come minimo.
>
> **R11.3** Un'operazione cancellata **NON DEVE** lasciare output parziale
> visibile, né stato intermedio non ripulito.
>
> **R11.4** La cancellazione **DEVE** essere riportata come categoria d'errore
> propria, mai confusa con un fallimento o con un successo parziale.
>
> **R11.5** Il meccanismo canonico è un **token concreto e clonabile** definito nel
> crate condiviso, non un trait per componente. Il token **DEVE** essere `Send`,
> `Sync` e `Clone`; l'osservazione **DEVE** costare quanto una lettura atomica,
> perché va controllata fra un batch e il successivo.
>
> **R11.6** Il token **DEVE** poter essere segnalato da un altro thread, così che
> un chiamante con un proprio meccanismo — Ctrl-C, runtime asincrono, timeout
> dell'host — lo possa collegare senza polling.

**Perché un token concreto e non un trait (R11.5).** Un trait per componente è
esattamente la situazione attuale: data-tools e database-tools ne hanno uno
ciascuno e non sono interoperabili, quindi un'operazione che attraversa i tre
richiede due adattatori. L'obiettivo del contratto è che un token creato dalla CLI
attraversi i tre componenti invariato. La flessibilità del trait serve a chi deve
astrarre *implementazioni* diverse; qui l'implementazione è una sola e la
flessibilità che serve davvero — collegare una sorgente esterna di segnale — la
dà R11.6.

**Stato oggi: due modelli e un'assenza.** data-tools ha un modulo `cancellation`
con token cooperativo; database-tools ha un trait `Cancellation` e sa cancellare
il backend remoto. IO-tools non ha un modello di cancellazione, e lo dichiara
apertamente in `IMPLEMENTATION_STATUS.md` («restano da uniformare cancellazione e
streaming dei parser che materializzano»). Un'operazione lunga che parte da un
driver non è oggi interrompibile.

---

## §12 R12 — Determinismo e riproducibilità dei risultati

> **R12.1** A parità di input, opzioni e versione, un componente **DEVE**
> produrre lo stesso output. Nessun risultato può dipendere dall'ordine di
> schedulazione dei thread, dall'iterazione di una mappa non ordinata o
> dall'orologio.
>
> **R12.2** Un ordinamento **DEVE** essere totale e stabile: a parità di chiave,
> l'ordine è deciso da un criterio esplicito, non dall'algoritmo.
>
> **R12.3** L'esecuzione parallela e quella sequenziale **DEVONO** produrre lo
> stesso risultato. La soglia di parallelizzazione è un dettaglio prestazionale,
> mai semantico.
>
> **R12.4** Un percorso che ricade su disco (spill) **DEVE** produrre lo stesso
> risultato del percorso in memoria. Il superamento di una soglia di memoria non
> può cambiare l'output.

**Perché R12.4.** È la forma più insidiosa di non determinismo: il risultato
cambia in funzione della dimensione dei dati o della memoria disponibile, quindi
non si riproduce sulla macchina di chi lo segnala.

**Verifica.** Test di snapshot canonico; confronto esplicito fra percorso in
memoria e percorso con spill; confronto fra sopra e sotto la soglia di
parallelizzazione.

**Stato oggi:** normato solo in data-tools, che ha test di determinismo canonico
IPC, catalogo canonico e — da `14a0a29` — l'oracolo memoria-contro-spill per
tutti e tre gli operatori spillabili (`spilled_sort_matches_in_memory`,
`spilled_distinct_matches_in_memory`, `spilled_aggregate_matches_in_memory`) più
un property test sul contratto d'insieme. R12.4 è quindi soddisfatta, non
parzialmente ma per l'intera superficie di spill. Gli altri due componenti non
hanno test equivalenti: IO-tools e database-tools non verificano oggi che
percorso parallelo e sequenziale coincidano.

---

## §13 R13 — Toolchain e baseline

> **R13.1** I tre componenti **DEVONO** compilare con la stessa versione esatta
> del compilatore, fissata da un file `rust-toolchain.toml` versionato.
>
> **R13.2** La CI **DEVE** usare `--locked`. Un `Cargo.lock` modificato dalla
> build è un fallimento, non un aggiornamento.
>
> **R13.3** Le dipendenze **DEVONO** essere dichiarate con pin esatto e in un
> punto unico per workspace (`[workspace.dependencies]`).
>
> **R13.4** L'edition è `2021` e la versione minima dichiarata è `1.92` in tutti i
> crate.
>
> **R13.5** Ogni variazione di dipendenza o toolchain **DEVE** essere accompagnata
> da change impact analysis.

**Perché R13.1.** Una CI che installa «stable» compila con una versione diversa
ogni settimana: la baseline non è riproducibile e un difetto introdotto da un
cambio di compilatore è indistinguibile da uno di codice (H-07).

**Stato oggi:** tutti e tre dichiarano `edition = 2021` e `rust-version = 1.92`,
quindi R13.4 è soddisfatta. Ma soltanto **database-tools** ha un
`rust-toolchain.toml` che fissa il canale a `1.92.0`: gli altri due compilano con
lo stable corrente, in violazione di R13.1.

Su R13.3 la versione 1.0 di questo documento riportava «due dipendenze caret» in
IO-tools: la fotografia era errata, perché contava solo
`plenora-io-core/Cargo.toml` senza il manifest di workspace. Il conteggio reale è
il seguente, e la segnalazione del team IO-tools è corretta:

| Componente | Dipendenze non pinnate |
|---|---|
| IO-tools | `geo-types "0.7"`, `shapefile "0.6"`, `kml "0.8"`, `calamine "0.26"`, `rust_xlsxwriter "0.79"`, `gdal "0.17"`, `serde "1"`, `serde_json "1"`, `sha2 "0.10"`, `tempfile "3"` (workspace) · `rustix "1"`, `atomicwrites "0.4.4"` (crate, anche fuori da `[workspace.dependencies]`) · `libc "0.2"` |
| data-tools | nessuna |
| database-tools | nessuna |

Le due dichiarate nel crate anziché nel workspace violano R13.3 due volte: per il
caret e per la collocazione. Le altre solo per il caret. Anche questa correzione
**non dipende dalla ratifica del presente documento**: la regola 6 del profilo
`AERONAUTICAL_PROFILE.md` di IO-tools chiede già change impact analysis per ogni
variazione di dipendenza, e un caret la rende impossibile.

---

## §14 R14 — Esiti di scrittura e pubblicazione

> **R14.1** Un output **NON DEVE** sovrascrivere una destinazione esistente, salvo
> richiesta esplicita del chiamante. Il controllo **DEVE** essere atomico, non
> una verifica seguita da una scrittura.
>
> **R14.2** Un output **DEVE** diventare visibile solo quando è completo. Nessun
> consumatore deve poter osservare uno stato intermedio.
>
> **R14.3** Una garanzia di durabilità **NON DEVE** essere dichiarata se la
> piattaforma non la conferma. L'esito **DEVE** distinguere «pubblicato e durevole»
> da «pubblicato, durabilità non confermata».
>
> **R14.4** Un fallimento **DEVE** lasciare la destinazione nello stato
> precedente e ripulire lo staging.
>
> **R14.5** Quando l'atomicità non è tecnicamente ottenibile, il componente
> **DEVE** dichiararlo nel proprio contratto anziché simularla.

**Perché.** H-02 e H-05 sono gli hazard che riguardano l'uscita, e valgono
identici su un file e su una tabella: sostituire un dataset pubblicato con uno
parziale ha lo stesso effetto sul consumatore, che si tratti di uno Shapefile o
di una tabella PostGIS.

**Stato oggi:** IO-tools è il riferimento — rename no-clobber autorevole,
sequenza durable con esito tipizzato, staging ripulito con guardia RAII — ma solo
su Linux e Windows: sulle altre piattaforme `publish_dir_atomic` fallisce sempre,
e i test sono gated, quindi la CI non lo rileva. database-tools soddisfa R14.3 e
R14.4 in transazione, con `OutcomeUnknown` sul commit incerto. data-tools ha un
`TempStore` con scavenging degli orfani.

---

## §15 Migrazione verso il crate condiviso

La convergenza avviene in quattro passi, in quest'ordine. Ogni passo è
verificabile e non richiede il successivo per essere utile.

**Passo 1 — Chiavi metadata (R2).** Non dipende da nessun refactoring: sono
costanti stringa. Ogni componente allinea i propri nomi alla tabella §2, con un
periodo di doppia lettura (accettare vecchio e nuovo, emettere solo il nuovo) se
serve compatibilità all'indietro. Vedi Appendice C.

**Passo 2 — Estrazione a semantica zero.** Nasce il crate condiviso, come pura
estrazione dei tipi di confine oggi in `plenora-IO-tools/crates/plenora-core`.
IO-tools ci dipende e re-esporta; nessun cambiamento di comportamento, nessuno
stato di tracciabilità si muove.

Il crate **DEVE** essere definito nella sua interezza prima di essere creato:

| Attributo | Valore |
|---|---|
| Nome | `plenora-contracts` |
| Repository | proprio, condiviso fra i tre team — **non** dentro uno dei tre componenti |
| Versione iniziale | `0.1.0`, `publish = false` |
| Distribuzione | dipendenza git per tag, non per path locale |
| Contenuto | `FieldId`, `CoordinateDimensions`, `GeometryType`, `GeometryEncoding`, `SpatialSemantics`, `CrsResolution`, `ResolvedCrs`, `AxisOrder`, `CancellationToken` (R11.5), `ErrorCategory` e `ErrorPhase` (§9), costanti delle chiavi §2 |
| Vincoli | nessuna dipendenza oltre `serde` e `arrow-schema`; nessun `unsafe`; nessuna primitiva di panic |

**Perché un repository proprio e non `docs/` di uno dei tre.** Un contratto che
vincola tre team non può essere ospitato da uno di essi: chi lo ospita ne
controlla di fatto il merge, e §15 assegna già a IO-tools il ruolo di riferimento
per i tipi. Ospitare anche il documento normativo concentrerebbe troppo. Il
documento e il crate **DOVREBBERO** vivere nello stesso repository e condividere
la versione: quando cambia il contratto, cambiano insieme, e una CIA può citare
un tag unico.

Se serve partire prima che il repository esista, la collocazione provvisoria
**DEVE** essere dichiarata tale, con la condizione di rientro, secondo §16.

**Distribuzione per path locale: vietata.** Una dipendenza `path = "../..."`
riproduce il problema che il crate deve risolvere: ogni team avrebbe una copia
non verificabile, cioè un terzo modello anziché un contratto comune.

**Passo 3 — Adozione.** data-tools e database-tools adottano il crate condiviso,
uno per volta, ciascuno con la propria change impact analysis.

**Passo 4 — Allargamento del modello.** Solo quando i tre dipendono dallo stesso
crate si estende il modello geometrico (curve, XYZM ovunque) con una CIA che
copre i tre insieme.

**Perché IO-tools è il punto di partenza del passo 2.** Non per maturità, ma
perché i suoi vincoli non sono negoziabili: i formati file esistono e dettano
cosa deve essere rappresentabile. Gli stati `unknown`, `declared_unresolved` e
l'axis order non canonicalizzato sono scoperte fatte contro dati reali, non
scelte di design che si possano rifare a tavolino. I sedici tipi geometrici di
database-tools, al contrario, si aggiungono in un pomeriggio. L'eccezione è il
modello d'errore (§9), dove il punto di partenza è database-tools.

---

## §16 Deroghe e modifiche

**Deroga.** Un componente che non può rispettare una regola **DEVE** dichiararlo
esplicitamente: regola, motivo, impatto sugli hazard, condizione di rientro. Una
deroga dichiarata è un gap noto; una regola aggirata in silenzio è un difetto.

**Registro.** Le deroghe attive **DEVONO** essere elencate in un punto solo per
componente, così che si possano contare. Una deroga senza condizione di rientro
è permanente: va scritto.

**Modifica del documento.** Una proposta di modifica **DEVE** indicare: regole
toccate, impatto sui tre componenti, piano di migrazione, retrocompatibilità. La
modifica entra in vigore quando i tre team l'hanno recepita — fino ad allora la
versione precedente resta vincolante.

**Versionamento.** Questo documento **DEVE** essere versionato in un repository,
non distribuito come file sciolto: senza storia non esiste baseline, i team non
sanno a quale versione si stanno conformando e nessuna change impact analysis è
possibile. La versione è dichiarata in testa al documento e citata nelle CIA.

**Proprietà.** Questo documento **DEVE** avere un owner nominato. Un contratto
trasversale senza proprietario diventa tre interpretazioni divergenti: è già
successo con i due `plenora-core`, ricopiati consapevolmente e poi lasciati
derivare fino a definire due `PlenoraError` incompatibili.

---

## §16-bis Lavoro non subordinato alla ratifica

La ratifica è necessaria per tutto ciò che tocca **tipi condivisi, nomi di chiavi
metadata e firme pubbliche**: cambiarli prima significa produrre un terzo modello
da rifare. Non è invece necessaria per ciò che ciascun componente già deve alle
proprie regole interne, oggi in vigore.

**Eseguibile subito, senza attendere:**

| Intervento | Perché non dipende da questo documento |
|---|---|
| Correzione di `driver.rs:347` (`unknown` → `xy`) | Violazione di PLN-ASR-007 e H-01, già ratificati in `AERONAUTICAL_PROFILE.md` |
| Pin esatto delle 13 dipendenze caret | Regola 6 del profilo: un caret rende impossibile la CIA che il profilo richiede |
| `rust-toolchain.toml` in IO-tools e data-tools | PLN-ASR-010 è già dichiarato «Parziale» per questo motivo |
| Censimento dei 103 `unwrap_or*` nei crate `lib` | H-01: il gate anti-panic non li intercetta, sono l'altra metà della stessa regola |
| Gate anti-panic `--lib` su data-tools e database-tools | Replica di un gate già in produzione, nessuna decisione di contratto |
| Doppia **lettura** delle chiavi metadata (accettare canoniche e legacy, emettere solo legacy) | Retrocompatibile e reversibile; dimezza il lavoro del passo 1 senza anticiparne le scelte |

**Subordinato alla ratifica:** rinomina delle chiavi in emissione (R2), forma dei
valori dei tipi geometrici (R3.1), estensione delle dimensioni propagabili (R3.3),
enumerazioni d'errore condivise (§9), token di cancellazione comune (R11.5),
creazione del crate `plenora-contracts` (§15 passo 2).

---

## §17 Definizione di componente conforme

Un componente è **conforme alla v1.0** quando, verificabile in CI:

1. Arrow pinnato alla versione di baseline (R1).
2. Emette e accetta le chiavi canoniche §2, e propaga invariate quelle che non
   interpreta (R2).
3. Rappresenta e propaga le cinque dimensioni e i sedici tipi, rifiutando
   esplicitamente ciò che non supporta (R3).
4. Distingue i tre stati del CRS e trasporta l'axis order (R4).
5. Non perde dati in silenzio: fail-closed o report esplicito (R5).
6. Gate anti-panic attivo sui target `--lib`, zero occorrenze (R6).
7. Applica limiti prima dell'allocazione, con profondità bounded (R7).
8. Nessuna collisione di nomi; dipende dal crate condiviso (R8).
9. Errori con categoria, fase e ritentabilità; esito ignoto distinto (R9).
10. Capability dichiarative interrogabili prima dell'esecuzione (R10).
11. Cancellazione cooperativa senza residui (R11).
12. Risultati deterministici, indipendenti da parallelismo e spill (R12).
13. Toolchain fissata, `--locked` in CI, dipendenze pinnate (R13).
14. Output atomico, no-clobber, durabilità dichiarata onestamente (R14).

**Nessuno dei tre componenti è oggi conforme alla v1.0.** Il più vicino è
IO-tools (R1, R3.3, R6, R14 su Linux/Windows); il più distante è database-tools,
che non ha CI e quindi non può dimostrare alcuna regola in modo automatico.

---

## Appendice A — Riepilogo di conformità

Rilevato per ispezione del codice al 27 luglio 2026. `—` = nozione non modellata.

| Regola | IO-tools | data-tools | database-tools |
|---|---|---|---|
| R1 Arrow pinnato `=59.1.0` | ✅ | ✅ | ✅ |
| R2 Chiavi canoniche | parziale | ❌ blob | ❌ namespace |
| R3.1 Tipi geometrici | 7/16 | — | 16/16, forma ✅ |
| R3.3 Dimensioni propagabili | ✅ 5/5 | ❌ solo `xy` | 4/5 |
| R3.4 `unknown` non degradato | ❌ `driver.rs:347` | n/a | n/a |
| R3.5 Encoding come enum | ✅ | — | ❌ `String` |
| R4 Modello CRS | ✅ modello, ⚠ shp:223 | ✅ | ❌ piatto |
| R5 Perdita non silenziosa | ⚠ 103 `unwrap_or*` | ✅ R5.3, censimento aperto | parziale |
| R6 Nessun panic nei `lib` | ✅ 0, gate attivo | ❌ ~121 | ❌ 26 |
| R7 Limiti pre-allocazione | parziale | parziale | ✅ AST |
| R8.1 Nomi crate unici | ❌ collisione | ❌ collisione | ✅ |
| R8.3 Crate condiviso | ❌ non esiste | ❌ | ❌ |
| R8.4 Tipi omonimi | ❌ `PlenoraError` | ❌ `PlenoraError` | ✅ |
| R9 Modello d'errore | ❌ no fase/retry | ⚠ manca fase ed esito ignoto | ✅ riferimento |
| R10 Capability dichiarative | ✅ | ❌ assenti | ✅ |
| R11 Cancellazione | ❌ assente | ✅ | ✅ |
| R12 Determinismo | ❌ non testato | ✅ | ❌ non testato |
| R13.1 Toolchain fissata | ❌ stable | ❌ stable | ✅ 1.92.0 |
| R13.3 Dipendenze pinnate | ❌ 13 caret | ✅ | ✅ |
| R14 Output atomico | ✅ Linux/Win | n/a | ✅ transazione |

---

## Appendice B — Hazard di riferimento

Da `plenora-IO-tools/docs/assurance/TRACEABILITY.md`. Le regole di questo
documento controllano gli hazard indicati.

| Hazard | Descrizione | Regole |
|---|---|---|
| H-01 | Corruzione, perdita o reinterpretazione silenziosa dei dati | R2.2, R2.4, R3.2, R3.4, R5, R6.5, R9.3, R10.4 |
| H-02 | Sovrascrittura o pubblicazione parziale di un dataset | R11.3, R14 |
| H-03 | Esaurimento non controllato di memoria, CPU o storage | R7 |
| H-04 | Panic, comportamento indefinito o arresto del processo | R6 |
| H-05 | Dichiarazione di durabilità non realmente confermata | R14.3 |
| H-06 | CRS o ordine assi interpretato in modo errato | R4 |
| H-07 | Artefatto non riproducibile o dipendenza non controllata | R1, R12, R13 |
| H-08 | Difetto non rilevato dalla strategia di verifica | R12.4, §17 |
| H-09 | Regressione introdotta senza change impact analysis | R13.5, §16 |

---

## Appendice C — Tabelle di rinomina

Migrazione delle chiavi metadata al namespace canonico (§2, passo 1 di §15).

### plenora-IO-tools

| Attuale | Canonica | Nota |
|---|---|---|
| `plenora.geometry.dimensions` | invariata | ✅ |
| `plenora.geometry.srid` | invariata | ✅ |
| `plenora.geometry.encoding` | invariata | ✅ |
| `plenora.geometry.types` | invariata | valori da riformattare (R3.1) |
| `plenora.geometry.spatial_semantics` | invariata | ✅ |
| `plenora.geometry.precision` | invariata | ✅ |
| `plenora.geometry.native.*` | `plenora.<formato>.*` | sotto-namespace per formato (R2.3) |
| `plenora.filegdb.*` | invariata | ✅ conforme a R2.3 |
| — | `plenora.geometry.crs_id` | da aggiungere |
| — | `plenora.geometry.crs_resolution` | da aggiungere |
| — | `plenora.geometry.crs_definition` | da aggiungere |
| — | `plenora.geometry.axis_order` | da aggiungere |

### plenora-database-tools

| Attuale | Canonica |
|---|---|
| `plenora.dimensions` | `plenora.geometry.dimensions` |
| `plenora.srid` | `plenora.geometry.srid` |
| `plenora.geometry_type` | `plenora.geometry.types` |
| `plenora.spatial_semantics` | `plenora.geometry.spatial_semantics` |
| `plenora.native_type` | `plenora.postgres.native_type` |
| `plenora.native_declaration` | `plenora.postgres.native_declaration` |
| `plenora.postgres_type_kind` | `plenora.postgres.type_kind` |
| — | `plenora.geometry.encoding` (oggi `String` interna) |
| — | `plenora.geometry.crs_resolution`, `crs_id`, `axis_order` |

### plenora-data-tools

| Attuale | Canonica |
|---|---|
| `plenora.contract` (blob unico) | esplodere nelle chiavi §2, una per proprietà |

---

## Appendice D — Glossario

**Componente** — uno dei tre workspace Plenora.

**Bordo** — componente che scambia dati con il mondo esterno: IO-tools verso i
formati file, database-tools verso i database. I bordi hanno vincoli non
negoziabili, imposti dai sistemi esterni.

**Centro** — il componente di trasformazione, data-tools. I suoi vincoli sono
scelte di design e quindi negoziabili.

**Confine** — il punto in cui un `RecordBatch` e il suo contratto passano da un
componente a un altro.

**Contratto** — l'insieme dello schema Arrow e dei metadati canonici §2 che
descrivono un flusso di dati.

**Fail-closed** — rifiutare l'operazione quando non se ne può garantire la
correttezza, invece di procedere in modo approssimato.

**Deroga** — dichiarazione esplicita che un componente non rispetta una regola,
con motivo e condizione di rientro (§16).

**CIA** — change impact analysis: l'analisi che accompagna ogni modifica secondo
il profilo di assurance.

**Baseline** — lo stato versionato e citabile di codice, dipendenze, toolchain e
documenti normativi a cui una CIA si riferisce.

---

*Documento redatto come revisione tecnica indipendente sullo stato del 27 luglio
2026. Le scelte canoniche di §2, §3, §4 e §9 sono proposte motivate e richiedono
ratifica dell'owner prima di diventare vincolanti per i tre team. Gli stati di
conformità in Appendice A sono rilevati per ispezione del codice, non per
esecuzione dei test.*
