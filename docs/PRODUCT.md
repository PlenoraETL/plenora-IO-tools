# Prodotto — che cosa offre e che cosa promette

Questo documento descrive la superficie che un consumatore vede: driver,
opzioni, contratti pubblici e limiti. Come è costruita e come è verificata sta
in [ENGINEERING.md](ENGINEERING.md); dove siamo nel percorso di rilascio sta in
[RELEASE.md](RELEASE.md).

---

## I dieci driver

| Driver | Direzione | Lettura nativa | Projection | Pruning | Reader concorrenti | Fedeltà | CRS | Feature |
|---|---|---|---|---|---|---|---|---|
| `csv` | R/W | sequenziale | esatta | — | multipli | condizionale | nessuno | — |
| `geojson` | R/W | sequenziale | esatta | — | multipli | condizionale | WGS84 fisso | — |
| `kml` | R/W | materializzata | — | — | uno attivo | condizionale | WGS84 fisso | — |
| `shp` | R/W | sequenziale | esatta | — | multipli | condizionale | incorporato | — |
| `gpkg` | R/W | ad accesso casuale | esatta | — | multipli | condizionale | incorporato | — |
| `geoparquet` | R/W | ad accesso casuale | esatta | statistiche min/max numeriche | multipli | **lossless** | incorporato | — |
| `ipc` | R/W | ad accesso casuale | esatta | — | multipli | **lossless** | incorporato | — |
| `xls` | R/W | materializzata | — | — | uno attivo | condizionale | nessuno | — |
| `dxf` | R/W | materializzata | — | — | uno attivo | **approssimante** | incorporato | — |
| `filegdb` | R/W | sequenziale | esatta | — | uno attivo | condizionale | incorporato | `gdal-backend` |

**Consegna e buffering sono uguali per tutti**: `operation_atomic` con buffering
adattivo — memoria finché il budget lo consente, poi spool su disco. Un errore a
metà lettura non consegna righe parziali.

Una scrittura rifiutata non lascia una destinazione, con **una** eccezione
dichiarata: il set di file sciolti dello Shapefile, che si ottiene solo
accettandolo esplicitamente e il cui contratto è più sotto, in
[§ Publish](#publish-che-cosa-diventa-visibile-e-quando).

**Fin dove arriva il supporto di una specifica**: il catalogo lo dichiara nel
campo `spec_version_supported` di ciascun driver. `geoparquet` dichiara
`1.1.0`: legge per intero i metadati `geo` delle versioni 1.0.0 e 1.1.0 — i due
valori che gli schemi ufficiali fissano — e scrive 1.1.0. Una versione oltre
viene rifiutata come **funzionalità non supportata**, che è cosa diversa da
metadati non conformi: il primo errore dice che il file va bene e noi no, il
secondo che il file è sbagliato. Con lo stesso criterio sono rifiutate le
codifiche native GeoArrow e i bordi sferici, entrambi validi in GeoParquet 1.1
e non implementati qui. Gli altri driver dichiarano `null`: i loro formati non
si versionano in un modo che il driver possa dichiarare per intero.

### Determinismo

I driver garantiscono **determinismo semantico**, non byte per byte: a parità di
input, opzioni e versione dell'implementazione, stessi valori e stesso insieme
di righe, **senza** assumere un ordine o una rappresentazione fisica identici.

È la garanzia minima della scala dichiarata dal modello — `semantic`, `ordered`,
`byte_for_byte`, `unordered` — e i dieci driver dichiarano `semantic` sia in
lettura sia in scrittura.

L'unica superficie con determinismo **byte per byte** è la busta di
`plenora-io catalog`, che serializzata due volte produce gli stessi byte. È una
proprietà di quel comando, non dei driver, e vale la pena non confonderle:
riconvertire due volte lo stesso file può produrre due output diversi byte per
byte e ugualmente corretti.

`gpkg` e `filegdb` sono multi-layer.

### Publish: che cosa diventa visibile, e quando

Il publish ha tre forme. **Due sono crash-atomic e una no**, e la differenza non
è un dettaglio di implementazione: decide che cosa un altro processo può vedere
sul disco se il nostro muore a metà.

| forma | chi la usa | garanzia |
|---|---|---|
| file singolo | `csv`, `geojson`, `kml`, `xlsx`, `dxf`, `gpkg`, `geoparquet`, `arrow` | un rename: il file c'è per intero o non c'è |
| directory-dataset | `filegdb`, e `shp` con destinazione `*.shp.d` | un rename della directory: tutti i file insieme, o nessuno |
| set di file sciolti | `shp` con destinazione `*.shp` | **nessuna**: quattro rename in sequenza |

La forma raccomandata in produzione per lo Shapefile è la **directory-dataset**,
e si ottiene chiedendo una destinazione che finisce in `*.shp.d`. Dentro ci sono
`data.shp`, `data.shx`, `data.dbf` e `data.prj`, e la directory diventa visibile
con un rename solo.

#### Il set sciolto va accettato, non subìto

La forma compatibile — `dati.shp` accanto a `dati.shx`, `dati.dbf`, `dati.prj` —
**non ha quella garanzia e non può averla**: quattro file separati non si rendono
visibili in un atto solo su nessuno dei filesystem supportati. Per questo non si
ottiene più deducendola dall'estensione: una destinazione `*.shp` è **rifiutata**
finché non la si accetta con l'opzione di formato
`publish_mode=loose_shapefile_set`. Il rifiuto è `InvalidConfiguration` — la
richiesta è incompleta, non il prodotto incapace.

Che cosa quell'accettazione comporta, per intero:

| | |
|---|---|
| **ordine** | i companion prima, il `.shp` per ultimo: chi cerca il file marker non lo trova finché il resto non c'è |
| **errore durante il publish** | i companion già spostati vengono riportati nello staging, best-effort |
| **rollback riuscito** | l'errore porta `remote_effect: none`: nessuna destinazione è rimasta visibile |
| **rollback fallito** | l'errore porta `remote_effect: partial` e `retry: requires_recovery`: sul disco **possono** esserci companion senza il `.shp` |
| **processo ucciso a metà** | nessun rollback avviene: lo stesso stato parziale, e nessun errore che lo dichiari |

L'ultima riga è la ragione per cui questa forma non è raccomandata: le prime
quattro descrivono ciò che il codice fa, la quinta ciò che nessun codice in
spazio utente può fare.

#### Recovery di un set sciolto interrotto

Vale per `remote_effect: partial` e per un processo ucciso durante il publish. È
la stessa procedura, perché è lo stesso stato: un insieme incompleto di file
accanto alla destinazione.

1. **Non ripetere la scrittura senza guardare.** Il publish è no-clobber su ogni
   file del set: un companion sopravvissuto fa fallire il tentativo successivo
   con `OutputExists`, e quel fallimento è corretto — non è il segno che si può
   riprovare.
2. **Riconoscere lo stato.** Un set completo ha `.shp`, `.shx`, `.dbf` e, se il
   layer porta un CRS, `.prj`. Un set senza `.shp` è incompleto per costruzione:
   il marker è l'ultimo a essere pubblicato.
3. **Rimuovere i companion rimasti**, cioè i file con lo stesso stem della
   destinazione che il publish avrebbe scritto. Nessuno di essi è un dato
   valido: appartengono a una scrittura che non si è conclusa.
4. **Ripetere la conversione**, preferibilmente verso `*.shp.d`, dove questa
   procedura non serve.

Lo staging non va cercato: è una directory temporanea che il processo rimuove al
termine, anche in errore. Ciò che il crollo può lasciare è **solo** accanto alla
destinazione.

### Che cosa significano le classi di fedeltà

| | |
|---|---|
| **lossless** | il formato rappresenta il contratto senza perdita |
| **condizionale** | la perdita dipende dal contratto, non dal formato: un CSV senza colonne esotiche è lossless, con un tipo temporale non lo è |
| **approssimante** | il formato approssima per costruzione. DXF tassella archi ed ellissi, esplode le geometrie multipart, rappresenta il testo come punto |

Ciò che si perde è dichiarato nel report di perdita, non taciuto, e il
**contratto** di quel report è ratificato: vedi
[LossReport](#lossreport--ratificato-con-il-protocollo-2).

### Modalità di lettura nativa

La modalità nativa è ciò che il formato consente; la modalità effettiva è
sempre `streaming_sequential`, perché l'adapter comune impone l'atomicità
operativa.

* **sequenziale** — il file si legge in avanti una volta sola;
* **ad accesso casuale** — il formato indicizza e consente di saltare;
* **materializzata** — il parser ha bisogno di **tutto l'input** prima di
  consegnare la prima riga. Non vuol dire «tutto in RAM»: dove stia quell'input
  lo descrive il buffering, che è un asse separato e adattivo — RAM sotto una
  soglia, poi spool su file. La coppia dice esattamente che cosa succede: serve
  tutto l'input, non serve tutta la memoria, e confondere i due assi è il
  difetto che il modello tiene distinto per costruzione. Per questi driver i
  limiti di input sono l'unica difesa contro un file ostile, e sono applicati
  **prima** che il parser veda il contenuto.

## Opzioni di formato

Le opzioni sono dichiarate in uno **schema**: chiave, fase (`read`, `write`,
`both`), tipo del valore e default. Un valore fuori schema è rifiutato prima di
raggiungere il driver, con il token dell'opzione rifiutata e l'elenco degli
ammessi.

`plenora-io catalog` emette lo schema completo. Le opzioni con un default
diverso da «assente»:

| Driver | Chiave | Fase | Valore | Default |
|---|---|---|---|---|
| `csv` | `delimiter` | entrambe | un carattere ASCII | `,` |
| `csv` | `geometry_encoding` | scrittura | `wkt` \| `xy` | `wkt` |
| `csv` | `wkt_column` | lettura | testo | assente |
| `xls` | `geometry_encoding` | scrittura | `wkt` \| `xy` | `wkt` |
| `xls` | `wkt_column` | lettura | testo | assente |
| `xls` | `sheet` | lettura | testo | primo foglio del workbook |
| `shp` | `publish_mode` | scrittura | `shapefile_directory_dataset` \| `loose_shapefile_set` | nessuno: `*.shp.d` si deduce, `*.shp` va accettata |
| `shp` | diagnostica di riga e nomi DBF | | | vedi catalogo |
| `geoparquet` | opzioni di scrittura | | | vedi catalogo |

I driver senza opzioni dichiarate non ne accettano: passarne una è un errore di
configurazione, non un valore ignorato.

### CRS

| | |
|---|---|
| **incorporato** | il formato porta il CRS, e viene letto da lì |
| **WGS84 fisso** | il formato lo impone; un CRS diverso è rifiutato |
| **nessuno** | il formato non lo porta. Se il contratto ha una geometria, `--assume-crs` è **obbligatorio**: senza, la lettura fallisce in fase CRS invece di indovinare |

Nessun driver riproietta. Il CRS viene letto, scritto o dichiarato — mai
trasformato.

---

## Contratti pubblici

La superficie con garanzia di compatibilità è **il JSON della CLI**. L'API Rust
è interna e instabile: non porta garanzia semver e i crate non sono pubblicati.

### Le sei buste

| Comando | Contratto |
|---|---|
| errori | `plenora-io-error-v1` |
| `catalog` | `plenora-io-catalog-v1` |
| `inspect` | `plenora-io-inspect-v1` |
| `layers` | `plenora-io-layers-v1` |
| `read` | `plenora-io-read-v1` |
| `convert` | `plenora-io-convert-v1` |

### `plenora-io-error-v1`

L'oggetto `error` porta **esattamente sei chiavi**:

```json
{
  "status": "error",
  "protocol_version": 1,
  "contract": "plenora-io-error-v1",
  "error": {
    "category": "...",
    "phase": "...",
    "remote_effect": "...",
    "retry": { "kind": "..." },
    "code": "...",
    "message": "..."
  }
}
```

`row_diagnostics` è l'unico campo opzionale, presente quando si osservano
rifiuti per riga.

I campi `driver`, `field` e `capability_reason` esistono nel tipo Rust e **non
sul wire**.

### Il quartetto è la chiave di compatibilità

```
(category, phase, code, retry)
```

Un consumatore deve decidere su questi quattro. **`message` non è una chiave di
compatibilità**: il suo testo può cambiare senza preavviso, ed è cambiato.

Il quartetto di ogni sito di costruzione è fissato da uno snapshot e verificato
a ogni checkpoint.

### `PublicMessage` — il testo è scelto a compile time

Nessun costruttore pubblico di errore accetta testo libero. Il messaggio si
costruisce da varianti che accettano solo `&'static str` e numeri strutturali:

| Variante | Contenuto |
|---|---|
| `Curated` | un testo nostro |
| `CuratedPair` | due testi nostri |
| `CuratedWith` | un testo e un numero strutturale (indice, conteggio, limite) |
| `CuratedBetween` | due testi e due numeri |
| `Capability` | una ragione tipizzata |
| `OpzioneRifiutata` | l'unica variante con testo runtime: un token **bounded e scappato**, coniabile solo dal validatore delle opzioni |

Il valore **decodificato** di `message` non supera **2048 byte UTF-8**. Il tetto
non è promesso sul JSON serializzato, dove l'escaping espande: una virgoletta
diventa due byte, un carattere di controllo sei.

#### Che cosa questa garanzia non è

`&'static str` garantisce la **durata**, non la **provenienza**. Un chiamante
deliberato può promuovere testo runtime a `'static` con `Box::leak` e infilarlo
in un messaggio curato senza che il compilatore obietti.

La promessa realistica è quindi:

> impedire la propagazione **accidentale** di testo runtime nel workspace, non
> rendere crittograficamente inconiabile un messaggio dinamico da codice ostile.

I crate sono interni e `publish = false`: l'avversario di questo invariante è la
distrazione, non un aggressore. La documentazione del tipo porta un esempio
eseguibile che dimostra il limite invece di ammetterlo a parole.

### `ContractIdentifier` ed `ErrorContext`

Un nome di campo o di layer non entra nel testo dell'errore: entra in un campo
**tipizzato**. `ContractIdentifier` è costruibile solo da un contratto validato —
non esiste una conversione da `String` — e i nomi non attestabili sono
**rifiutati**, non troncati.

`ErrorContext` porta driver, campo e ragione di capability accanto all'errore,
per il chiamante Rust. Nessuno dei tre raggiunge il wire v1.

### Budget e limiti

Un'operazione riceve un `PipelineBudget` che porta i limiti e un **permit di
input**, consumato per `move`: la stessa sorgente non può essere osservata due
volte.

| Limite | Governa |
|---|---|
| `max_rows`, `max_columns` | cardinalità del contratto e dello stream |
| `max_input_bytes`, `max_input_entries` | il footprint della sorgente, prima di aprirla |
| `max_output_bytes` | la destinazione |
| `memory_bytes` | la memoria della pipeline. **Non** è la soglia dello spool: quella è la metà della capacità effettiva, e l'altra metà resta al batch in materializzazione |
| `spill_bytes` | i byte che lo spool può scrivere sul file temporaneo |
| `duration_ms` | la deadline dell'operazione, dalla costruzione del budget |
| `max_wkb_cell_bytes`, `max_wkb_components`, `max_wkb_depth` | ogni geometria, in lettura e in scrittura |
| `decompression_ratio` | il rapporto fra byte dichiarati e byte compressi negli archivi |

I limiti si applicano **prima** dell'allocazione che dovrebbero impedire. Un
tetto verificato dopo aver materializzato la cella non è un tetto.

Il superamento è sempre `ResourceLimit`, e il messaggio porta il numero
strutturale — conteggio e limite — non il valore letto dal file.

### Compatibilità

| | |
|---|---|
| **vincolante** | il contratto e la `protocol_version` di ciascuna busta; l'insieme delle chiavi di `plenora-io-error-v1`; il quartetto degli errori |
| **non vincolante** | il testo di `message`; l'ordine delle chiavi; l'API Rust |
| **cambio breaking** | rimuovere o rinominare una chiave, cambiare un valore del quartetto per un sito esistente, cambiare la semantica di un codice |

Un cambio breaking richiede una nuova versione di contratto, non una nota.

---

## Limitazioni di prodotto

| | |
|---|---|
| DXF | approssima per costruzione: archi ed ellissi tassellati, multipart esplose, testo come punto. La perdita è dichiarata, non silenziosa |
| KML, XLSX, DXF | lettura materializzata: la libreria sottostante ha bisogno di **tutto l'input** prima della prima riga. Dove stia quell'input lo decide il buffering, che è un asse separato: non è una promessa che stia tutto in RAM. I limiti di input sono l'unica difesa, e vengono applicati prima del parser |
| FileGDB | richiede `gdal-backend`. Senza, ogni chiamata fallisce come capability mancante |
| CSV, XLSX | non portano CRS: con una geometria, `--assume-crs` è obbligatorio |
| tutti | nessuna riproiezione |

---

## LossReport — ratificato con il protocollo 2

Il contratto del report di perdita è **ratificato**, ed è il
[protocollo 2](../release/cli-protocol-v2.json). Le cinque decisioni che lo
tenevano aperto — struttura, limiti, redazione, comportamento al limite,
versionamento — sono chiuse, e ciascuna è applicata dal codice e verificata da
un gate.

Che cosa esce oggi, nel v2:

```json
"read_loss": {
  "lossless": false,
  "troncato": false,
  "omesse_esatte": true,
  "omesse": { "categorie_omesse": 0, "ragioni_omesse": 0,
              "esempi_omessi": 0, "omesse_per_byte": 0 },
  "counts": [ { "categoria": "coercion tipo attributo", "conteggio": 3 } ],
  "esempi": [ { "category": "coercion tipo attributo",
                "layer_index": 0, "field_index": 4, "type_class": "decimal",
                "context": "il tipo dell'attributo richiede una coercizione" } ]
}
```

### I limiti, e chi li decide

| Grandezza | Tetto |
|---|---|
| categorie in `counts` | **64** |
| byte di un identificatore di categoria | **128**, ovunque compaia |
| ragioni in `reasons` | **64** |
| esempi in `esempi` | **64** |
| byte di un `detail` o di un `context` | **512** |
| byte di una sezione | **12 KiB**, sulla serializzazione effettiva |
| byte della diagnostica di una busta | **64 KiB** |

Nessuno di questi numeri lo decide chi fornisce il file, ed è il cambiamento
che il v2 porta. Il budget non speso da una sezione **non** passa a un'altra:
con un consumo sequenziale una sezione grande affamerebbe quelle che la
seguono, e la stessa sezione produrrebbe un output diverso a seconda di quanto
ha occupato un'altra.

### Nessun nome preso dal file

`reasons[].detail` e `esempi[].context` portano testo **curato**: stabile,
descrittivo, scritto da noi. Dove si è persa una cosa lo dicono `layer_index` e
`field_index`, che sono indici e non nomi — e nemmeno un hash dei nomi, che
resterebbe un identificatore controllato da chi fornisce il file. Il tipo di un
attributo passa da `type_class`, un vocabolario chiuso nostro, mai dalla forma
`Debug` di un tipo di dipendenza.

L'unica eccezione è dichiarata: i **codici numerici** di un registro di
autorità CRS possono comparire in un `context`, perché senza di loro
un'incoerenza fra `crs_definition`, `crs_id` e `srid` non è leggibile. Le
stringhe libere che li accompagnano restano vietate.

### Il troncamento è sempre dichiarato

Quattro cause separate, perché «sono più di sessantaquattro» e «lo spazio è
finito» portano a decisioni diverse. I conteggi pubblicati restano esatti: si
omette una voce intera, mai si riscrive un numero. Se nemmeno la dichiarazione
di troncamento entra nel budget, la CLI **fallisce** invece di pubblicare una
sezione che tace.

`omesse_esatte` qualifica i quattro contatori: vale `false` quando una perdita
di esattezza interna li rende limiti inferiori.

### Il v1 resta quello che era

`--legacy-protocol-v1-unsafe` seleziona il protocollo congelato, difetti
compresi, e lo dice nel nome. I suoi `detail` sono conservati **alla lettera** —
non ricostruiti — in un campo privato che un solo adattatore legge, e a
pretendere che il lettore resti uno solo è un gate: la visibilità di Rust non sa
dire «questo modulo e nessun altro».
