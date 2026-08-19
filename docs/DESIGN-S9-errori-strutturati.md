# Design S9 — errori strutturati e redazione per costruzione (L0.6 / INV-10)

Stato: **proposta**, non ratificata. Nessuna riga di codice va scritta prima
della ratifica.

Baseline: `1d24141` (S8.1).

## 1. Il problema, misurato

`PlenoraIoError` ha oggi un campo `message: String` alimentato a runtime. Non è
un dettaglio di rappresentazione: è un **canale**, e il canale è già usato.

Contati sul workspace alla baseline:

| | Siti |
|---|---|
| costruzioni totali di `PlenoraIoError` | 273 |
| con `format!(…)`, cioè testo derivato a runtime | **210** |
| con stringa letterale | 63 |

E questo è ciò che i `format!` interpolano, per frequenza:

| Segnaposto | Siti | Cosa porta fuori |
|---|---|---|
| `{e}` / `{error}` | **121** | il testo d'errore di una **dipendenza** |
| `{path}` | 8 | un percorso del filesystem |
| `{name}`, `{id}`, `{layer}` | 15 | identificatori di schema |
| `{index}`, `{code}` | 14 | indici e codici numerici |
| altri | ~50 | misto |

I 121 siti con `{e}` sono il cuore del problema. Quel testo viene da `calamine`,
`parquet`, `arrow`, `csv`, `serde_json`, `rusqlite`, GDAL. Nessuna di quelle
librerie promette che il proprio messaggio non contenga un percorso, un valore
di cella o un frammento del payload — e alcune è **documentato** che li
contengano. Ogni volta che scriviamo `format!("apertura XLSX: {e}")` stiamo
propagando testo che non abbiamo scritto e non possiamo prevedere.

Non è teorico: durante XLSX-HARDENING abbiamo dovuto redigere a mano il
messaggio di un panico proprio perché conteneva il valore della cella
responsabile.

### Cosa invece è già a posto

Due cose che INV-10 chiede e che **non** vanno rifatte, verificate sul codice e
non assunte:

* **Nessun fingerprint pubblico.** `impronta_di_panico` è stata rimossa in FZ-0.
  L'unico FNV nel modello è `DigestAccumulator` in `budget.rs`, che assorbe le
  voci della sorgente per produrre un `SourceDigest`: non esce da quel file e
  non compare in nessun envelope — verificato con una ricerca su tutto il
  workspace. L'FNV in `plenora-fuzz` è attrezzaggio, non codice spedito.
* **Le prevalidazioni recenti hanno già messaggi statici.** FZ-0.1 e FZ-0.2
  usano costanti `&'static str`, e il commento sul perché è già scritto accanto
  a esse. Sono il modello di ciò che S9 generalizza.

## 2. Cosa S9 deve produrre

Da INV-10, senza reinterpretazioni:

* `PlenoraIoError` **senza** `message: String` pubblico alimentato a runtime;
* un enum `PublicMessage` i cui parametri stanno in un insieme consentito:
  `&'static str`, enum del workspace, indici numerici;
* un `ErrorContext` strutturato, con `ContractIdentifier` costruibile **solo**
  da un contratto già validato;
* struttura wire di `plenora-io-error-v1` **invariata**; il testo di `message`
  intenzionalmente diverso;
* enforcement dal compilatore: nessun costruttore pubblico che accetti `String`
  libera.

## 3. La decisione che governa il costo: i 121 siti con `{e}`

È qui che S9 si decide. Tre opzioni, e non sono equivalenti.

| | Cosa succede al testo della dipendenza | Costo | Cosa si perde |
|---|---|---|---|
| **A. Scartato** | non esce, punto | basso: 121 siti diventano una costante per classe | la diagnosi di un errore raro diventa «non leggibile», senza dire perché |
| **B. Mappato** | ogni classe di fallimento della dipendenza diventa una variante d'enum | **alto**: bisogna enumerare le classi, dipendenza per dipendenza | niente, se l'enumerazione è completa; ma completa non lo sarà mai |
| **C. Deviato in `row_diagnostics`** | resta disponibile, dietro la policy `emit`/`redact` già esistente | medio | niente sul contratto; il rischio si sposta su chi abilita `emit` |

**Raccomandazione: A come regola, B dove la classe è già nota, C mai per il
testo delle dipendenze.**

Il ragionamento su C è la parte che conta. `row_diagnostics` ha una policy
`emit`/`redact` che governa **il valore di una chiave scelta dall'operatore** —
un campo che l'operatore ha nominato, di cui conosce la sensibilità. Il testo
d'errore di una libreria C non è quello: nessuno lo ha scelto, nessuno sa cosa
contenga, e metterlo dietro un interruttore chiamato `emit` invita ad accenderlo
per «avere più diagnostica». Sarebbe redazione per configurazione, e INV-10
chiede redazione per costruzione.

Su A: la perdita è reale e va accettata a occhi aperti. Un errore `parquet` che
oggi dice «Invalid page header» domani dirà «file Parquet non leggibile». Chi
sviluppa il driver ha comunque `RUST_LOG` e il seme; chi opera non aveva modo di
agire su quel testo comunque.

Su B: si applica dove la classe **è già** un tipo nostro. `WkbFailureKind`
esiste; `CapabilityReason` esiste. Estenderli è naturale. Inventare
`ParquetFailureKind` con trenta varianti per rispecchiare un enum di una
dipendenza che cambia a ogni minor **no**: sarebbe una copia che diverge in
silenzio, cioè il difetto che S6 ha appena chiuso altrove.

## 4. Forma proposta

Sostanzialmente quella del pacchetto, con tre precisazioni che nascono dal
codice.

```rust
#[non_exhaustive]
pub struct PlenoraIoError {
    kind: ErrorKind,
    message: PublicMessage,
    context: ErrorContext,
    category: ErrorCategory,
    phase: ErrorPhase,
    remote_effect: RemoteEffect,
    retry: RetryDisposition,
    code: IoErrorCode,
    row_diagnostics: Option<Box<RowDiagnostics>>,
}
```

**Precisazione 1 — `driver` e `field` cambiano tipo.** Oggi sono
`Option<String>`, e `field` è il posto dove finiscono i nomi di colonna
costruiti a mano. Diventano `&'static str` per il driver (è già un letterale in
ogni sito) e `ContractIdentifier` per il campo. Non è un dettaglio: `field:
Option<String>` è un secondo canale libero accanto a `message`, e chiuderne uno
solo lascerebbe la porta aperta.

**Precisazione 2 — la migrazione è il lavoro, non il tipo.** I tipi sono un
giorno. I 273 siti sono cinque. Il valore sta nel fatto che il compilatore
guida la conversione: dopo il cambio di firma, ogni sito non convertito è un
errore. Come per INV-14, dove due sostituzioni sbagliate sono state prese dal
compilatore e una solo dal gate con tutte le feature.

**Precisazione 3 — `PublicMessage` deve essere `const`-costruibile.** Le
prevalidazioni di FZ-0.1 e FZ-0.2 dichiarano i propri messaggi come costanti, e
i gate lo verificano. Se `PublicMessage` non fosse costruibile in contesto
`const`, quelle costanti diventerebbero funzioni e il gate perderebbe la presa.

## 5. Il wire

Struttura invariata, testo diverso. Concretamente, per l'envelope
`plenora-io-error-v1`:

| Campo | Prima | Dopo |
|---|---|---|
| `contract`, `protocol_version`, `status` | invariati | invariati |
| `category`, `phase`, `remote_effect`, `retry`, `code` | invariati | invariati |
| `driver` | `Option<String>` | invariato sul wire, derivato da `&'static str` |
| `field` | `Option<String>` | invariato sul wire, derivato da `ContractIdentifier` |
| `message` | testo libero | **testo curato, deterministico** |
| `row_diagnostics` | invariato | invariato |

**Questo rompe chiunque faccia match sul testo di `message`.** Va detto adesso e
accettato in ratifica, non scoperto dopo. I quattro assi
`(category, phase, code, retry)` sono lì apposta e restano deterministici: chi
correla errori deve usare quelli. Va verificato che i nostri stessi test non
facciano match sul testo — e alcuni lo fanno, quindi la migrazione li tocca.

## 6. Cause multiple — il residuo che arriva qui

La correzione della causa primaria (commit `b786177`) ha lasciato un residuo
esplicitamente rinviato a S9: senza `input_total` l'errore nomina **una** causa,
quella della prima riga rifiutata. È un fail-fast deterministico, non un
campione, ed è accettato come tale.

S9 è il posto dove decidere se serve di più, perché è la prima volta che si
tocca la forma dell'errore. Due opzioni:

* **lasciare così**: una causa, deterministica, e i conteggi per causa restano
  disponibili solo quando il report è emettibile;
* **`PublicMessage` con un elenco di cause**: `Curated(&'static str)` diventa
  affiancato da una variante con `&'static [ContractViolationKind]`.

**Raccomandazione: lasciare così in S9.** L'elenco di cause ha senso solo se il
consumatore ne fa qualcosa di diverso dal leggerlo, e oggi non c'è un
consumatore che lo faccia. Aggiungerlo perché «S9 tocca gli errori» è il modo
in cui una struttura cresce senza che nessuno la chieda.

## 7. Cosa S9 non fa

* Non introduce nuove categorie d'errore. In particolare **non**
  `TerminatedAfterAcceptedBatches`: appartiene a `DeliverySemantics::Streaming`,
  che resta dichiarabile e non implementata (S8).
* Non tocca `row_diagnostics` né il suo contratto.
* Non cambia i quattro assi.
* Non introduce gate testuali per verificare la redazione. Un gate che cercasse
  `format!` dentro un costruttore d'errore sarebbe fragile in entrambe le
  direzioni; l'enforcement è il **tipo**, e se il tipo non basta il gate non lo
  rimedia.

## 8. Piano di test proposto

| Test | Cosa dimostra |
|---|---|
| compile-fail: `PlenoraIoError` da `String` | nessun costruttore libero |
| compile-fail: `PublicMessage::Curated(formato_a_runtime)` | il messaggio è compile-time |
| compile-fail: `ContractIdentifier::from_string(…)` | non esiste |
| `il_wire_ha_la_stessa_forma_della_baseline` | tutti i campi, stesso ordine, stessi tipi; solo `message` diverso |
| `nessun_errore_del_workspace_contiene_testo_di_dipendenza` | costruito facendo fallire ogni driver su un input ostile e verificando che il messaggio sia una delle costanti dichiarate |
| `i_quattro_assi_restano_deterministici` | stesso input, stesso `(category, phase, code, retry)` |

Il quinto è quello che vale: è l'unico che misura la proprietà invece di
dichiararla, ed è anche il più caro da costruire, perché richiede un input
ostile per driver. Va dimensionato in ratifica.

## 9. Decisioni che servono in ratifica

1. **Opzione A/B/C per i 121 siti con `{e}`** — raccomandato A come regola, B
   dove la classe è già un tipo nostro, C mai.
2. **Rottura del testo di `message` sul wire**: accettata o serve un periodo di
   doppia emissione?
3. **`driver` e `field` cambiano tipo in Rust** (wire invariato): confermato?
4. **Cause multiple**: lasciate come sono (raccomandato) o modellate ora?
5. **Ampiezza del test n. 5**: tutti e dieci i driver o un sottoinsieme?
6. **Ordine rispetto a S12**: il pacchetto dice che S12 può andare in parallelo.
   S9 tocca ogni sito d'errore del workspace; S12 ne aggiunge. Farli insieme
   significa due migrazioni sullo stesso codice.

## 10. Stima

Il pacchetto dice 5-7 giorni-persona. Sulla base dei numeri misurati — 273 siti,
di cui 210 da riscrivere davvero, più la migrazione dei test che fanno match sul
testo — la stima è **plausibile ma sul limite inferiore**. Il fattore di rischio
non sono i tipi: è la decisione n. 1, perché B applicato largamente
moltiplicherebbe il lavoro senza un tetto chiaro.
