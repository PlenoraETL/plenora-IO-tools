> Documento append-only: correzioni e seguiti vanno in coda, non nel corpo.

# Debito — contratto dei report di perdita

Aperto il 2026-08-21, emerso dalla tranche 13 di S9.
**Sigla da assegnare alla ratifica**: qui non ne viene inventata una, perché una
sigla è una promessa di gating e il gate non esiste ancora.

## Perché esiste questo documento

S9 ha chiuso il testo runtime nei **messaggi d'errore**. Il sito che lo ha fatto
notare non era un errore:

```rust
self.loss.record(&format!("attributo non rappresentato in DXF: {c}"), self.rows);
```

È fuori dal perimetro di S9, e correttamente: `LossReport` non è
`PlenoraIoError`, ha un'altra struttura e un altro contratto di wire.

**Ma «non è un `PlenoraIoError`» non è una ragione per non decidere.** È testo
serializzato che nasce dal dominio ed esce verso i consumatori, e la decisione su
struttura, limiti e redazione non è stata presa — non è stata nemmeno posta.

## Che cosa esce, e da dove

`crates/plenora-io-cli/src/main.rs:141` — `loss_doc` serializza `loss.counts`
nel JSON prodotto dalla CLI. **È una superficie verso i consumatori distinta da
`plenora-io-error-v1`**, e nessuno dei gate di S9 la guarda.

### Le tre superfici, con i tetti che hanno davvero

| Superficie | Tipo | Tetto sul **numero** | Tetto sulla **lunghezza** |
|---|---|---|---|
| `LossReport.counts` | `BTreeMap<String, u64>` | **nessuno** | **nessuno** |
| `LossReport.examples` | `Vec<LossExample>` | `MAX_LOSS_EXAMPLES = 64` | **nessuno** |
| `FidelityAssessment.reasons` | `Vec<FidelityReason>` | `MAX_FIDELITY_REASONS = 64` | **nessuno** |

I due tetti esistenti limitano **quante voci**, non **quanto lunga** ciascuna.
Nessuna stringa del modulo ha un limite in byte — mentre i messaggi d'errore,
dopo S9, ce l'hanno (`MAX_MESSAGE_BYTES`).

### La cardinalità di `counts` è la proprietà più esposta

```rust
pub fn record(&mut self, category: &str, n: u64) {
    let count = self.counts.entry(category.to_owned()).or_default();
    *count = count.saturating_add(n);
}
```

Ogni stringa distinta crea **una chiave nuova**. Una categoria costruita
interpolando un nome fa crescere la mappa con il numero di nomi distinti: la
struttura pensata per essere un istogramma di poche categorie note diventa un
elenco per elemento.

`saturating_add` protegge il valore. **Nulla protegge la chiave.**

## Censimento

### Scrittori di `counts` con testo costruito a runtime

| Sito | Testo | Provenienza |
|---|---|---|
| `driver-dxf/src/lib.rs:536` | `format!("attributo non rappresentato in DXF: {c}")` | nome di colonna dal contratto d'ingresso |
| `plenora-io-core/src/driver.rs:1026` | `format!("{representation}_not_preserved_{category_suffix}")` | **due vocabolari chiusi**: cardinalità limitata |

I due casi **non sono la stessa cosa**, e vanno decisi separatamente: il secondo
compone statici — è già di fatto una `CuratedPair` scritta a mano — mentre il
primo fa entrare un nome nella chiave.

Tutti gli altri ~20 siti passano `&'static str` o costanti: `driver-dxf` ne ha
15 così, `driver-gpkg` usa `coercion.category() -> &'static str`, `driver-shp`
una costante.

### Scrittori di `context` e `detail`

| Sito | Testo |
|---|---|
| `plenora-io-core/src/driver.rs:999` | `format!("layer={} field={}", layer.name, field.name())` |
| `plenora-io-core/src/driver.rs:1029` | `format!("layer={layer} field={field} representation={representation} …")` |
| `driver-gpkg/src/lib.rs:716` | `format!("field={name}: {}", coercion.detail())` |
| `driver-shp/src/lib.rs:2264` | `format!(…)` |
| `plenora-io-core/src/loss.rs:76, 85` | `format!("{format}: …")` |

`context` porta nomi di layer e di campo. Sono **nomi di contratto**, non byte
del payload — la stessa distinzione che in S9 separa `ContractIdentifier` da un
`&str` qualunque. Il che suggerisce la forma della soluzione, non la sostituisce.

## Il precedente è dentro lo stesso modulo

```rust
pub struct FidelityReason {
    pub code: FidelityReasonCode,   // tipizzato
    pub detail: String,             // libero
}
```

`FidelityReason` ha **già** la forma che S9 ha scelto per gli errori: un codice
tipizzato accanto a un testo. Metà del lavoro è fatta e nessuno l'ha chiamata
così. `counts` invece non ha nulla di tipizzato: la chiave *è* il testo.

## Le tre decisioni dovute

Sono distinte, e prenderne una non implica le altre:

1. **Struttura** — `counts` resta indicizzata per stringa, o per un enum di
   categorie (con una variante di scarico per l'ignoto)? Un enum rende il
   contratto verificabile e rompe i consumatori che leggono le chiavi attuali.
2. **Limiti** — un tetto in byte per chiave, `context` e `detail`; un tetto alla
   **cardinalità** di `counts`, che oggi non esiste. Va deciso se l'eccedenza
   tronca, aggrega in una categoria residua, o rifiuta.
3. **Redazione** — quali valori possono comparire. La regola di S9 (nulla dal
   payload; i nomi di contratto solo in un campo tipizzato) è un candidato
   naturale, ma **applicarla qui è una decisione, non un corollario**: un report
   di perdita ha lo scopo opposto a quello di un messaggio d'errore, cioè dire
   con precisione *che cosa* si è perso.

Il punto 3 è quello che rende il debito non banale. Chiudere `counts` come si
sono chiusi i messaggi renderebbe il report meno utile proprio nella parte per
cui esiste.

## Che cosa questo documento **non** afferma

* non afferma che ci sia una vulnerabilità: nessun byte del payload risulta
  raggiungere `counts`; i nomi vengono da contratti e schemi;
* non afferma che sia release-blocking. **Non lo è ancora**, perché non è stato
  valutato — e non è stato valutato è diverso da è accettabile;
* non propone un gate: proporlo prima della decisione significherebbe cablare
  una risposta che nessuno ha ratificato.

Serve a impedire che la questione sparisca perché il sito che l'ha sollevata non
era un `PlenoraIoError`.

## Rapporto con gli altri registri

| Registro | Rapporto |
|---|---|
| S9 / INV-10 | disgiunto: altra struttura, altro wire. La regola di S9 è un **candidato** per il punto 3, non la sua risposta |
| ASSURANCE-N1 | disgiunto: quello è copertura di rami negativi, questo è un contratto |
| registro fallback | disgiunto |
