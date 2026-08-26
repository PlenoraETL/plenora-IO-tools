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
metà lettura non consegna righe parziali, e una scrittura rifiutata non lascia
una destinazione.

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

`gpkg` e `filegdb` sono multi-layer. `shp` e `filegdb` pubblicano più file, e il
publish li rende visibili insieme o per nulla.

### Che cosa significano le classi di fedeltà

| | |
|---|---|
| **lossless** | il formato rappresenta il contratto senza perdita |
| **condizionale** | la perdita dipende dal contratto, non dal formato: un CSV senza colonne esotiche è lossless, con un tipo temporale non lo è |
| **approssimante** | il formato approssima per costruzione. DXF tassella archi ed ellissi, esplode le geometrie multipart, rappresenta il testo come punto |

Ciò che si perde è dichiarato nel report di perdita, non taciuto. Il **contratto**
di quel report non è però ancora ratificato: vedi [LossReport](#lossreport--non-ratificato).

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
| `shp` | opzioni di publish e nomi DBF | | | vedi catalogo |
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
| `memory_bytes` | la soglia oltre cui il buffering passa dalla memoria allo spool |
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

## LossReport — NON RATIFICATO

**Il contratto del report di perdita non è ratificato**, e la sua superficie è
già sul wire: `read_loss`, `write_loss` e `fidelity` compaiono nelle buste di
`convert`, `inspect`, `layers` e `read`.

Che cosa esce oggi:

```json
"read_loss":  { "lossless": false, "counts": { "<categoria>": 3 } },
"read_fidelity": { "level": "...", "reasons": [ { "code": "...", "detail": "..." } ] }
```

Stato misurato della superficie:

| Grandezza | Tetto |
|---|---|
| cardinalità di `counts` | nessuno nel contratto; di fatto delimitata da `max_columns` |
| lunghezza di una chiave di `counts` | **nessuno** |
| numero di `reasons` | 64 |
| lunghezza di un `detail` | **nessuno** |
| byte totali della busta di perdita | **nessuno** |

`FidelityReason.detail` porta nomi di layer e di attributo e la forma `Debug` di
tipi di dipendenza. Il vocabolario di `counts` mescola identificatori macchina e
prosa, quindi un consumatore non può né farne `match` né mostrarlo.

### Le cinque decisioni aperte

1. **Struttura** — `counts` resta indicizzata per stringa, o per un enum chiuso?
2. **Limiti** — tetto alla cardinalità, tetto in byte per stringa, tetto ai byte
   totali della busta.
3. **Redazione** — quali valori possono comparire. La regola degli errori non si
   applica per analogia: un report di perdita ha lo scopo opposto, cioè dire
   *quale* colonna si è persa.
4. **Comportamento al limite** — troncare o rifiutare, che cosa si conserva, e
   come si dichiara ciò che è stato omesso.
5. **Versionamento** — qualunque scelta rompe un consumatore che legga le chiavi
   attuali.

Finché non sono ratificate, **nessuna promessa di compatibilità copre questa
superficie**, e la voce resta bloccante per il rilascio.
