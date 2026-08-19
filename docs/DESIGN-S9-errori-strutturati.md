# Design S9 — errori strutturati e redazione per costruzione (L0.6 / INV-10)

Stato: **ratificato** il 2026-08-19. Le sei decisioni della sezione 9 sono
risolte in coda al documento, insieme a due vincoli aggiuntivi.

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

## 11. Ratifica (2026-08-19)

Le sei decisioni della sezione 9, risolte.

**1. Testo delle dipendenze: A come regola.** B soltanto quando la classe è
*già* rappresentata da un tipo del workspace. C mai — e con essa **nessun canale
alternativo**: né log, né `source`, né `context`, né `row_diagnostics`. Il testo
di una dipendenza non esce, e non esce da nessuna parte: aprire una seconda
porta perché la prima è chiusa sarebbe la stessa perdita con un nome diverso.

**2. Il testo di `message` cambia subito, senza doppia emissione.**
`plenora-io-error-v1` resta invariato **strutturalmente**. Il testo non è un
identificatore stabile e non lo è mai stato: chi correla errori usa
`(category, phase, code, retry)`. La rottura va documentata nelle release note.

**3. Tipi Rust confermati**: `driver: Option<&'static str>`,
`field: Option<ContractIdentifier>`. `ContractIdentifier` nasce **solo** dalla
risoluzione di un contratto validato — niente `From<String>`, niente costruttori
unchecked pubblici, niente `Deserialize` che aggiri la validazione.

**4. Cause multiple: non introdotte.** Resta la prima causa, deterministica.
Nessun totale e nessun elenco inventato.

**5. Test ostili su tutti e dieci i driver**, FileGDB compreso nel gate
feature-on. Va descritto per ciò che è: una prova di **attraversamento di ogni
driver**, non una dimostrazione dinamica dei 121 siti. L'esaustività è garantita
dal tipo e dai compile-fail; il test ostile prova che il tipo è davvero sul
percorso che l'utente attraversa.

**6. S9 precede S12.** Quando S12 aggiungerà errori nuovi, il tipo li obbligherà
già alla redazione. L'ordine inverso avrebbe richiesto due migrazioni sullo
stesso codice.

### Due vincoli aggiuntivi

**Parametri numerici**: sono ammessi solo **indici, conteggi, limiti o codici
strutturali tipizzati**. Mai un valore numerico letto dal payload. La
distinzione non è pedanteria: «riga 47» è una posizione nel file che il
chiamante conosce già, «valore 47.3» è il dato. Il primo aiuta a trovare il
problema, il secondo lo espone.

**Nessun costruttore pubblico** deve accettare `String`, `Cow<str>`, `impl
Display`, `dyn Error` o `&str` non `'static`. È l'enforcement che rende la
regola 1 una proprietà del tipo invece di una convenzione — e quindi ciò che i
compile-fail devono provare.

## 12. Vincoli esterni (2026-08-19)

* **`plenora-contracts` non si tocca e non si assume sincronizzato.** È un
  repository esterno, referenziato solo nella documentazione: non è una
  dipendenza di codice di questo workspace, verificato sui `Cargo.toml`. S9 vive
  interamente in IO-tools.
* **Nessuna doppia emissione**, come da decisione 2.
* **Il testo di `message` non è una chiave di compatibilità**, e non va trattato
  come tale da nessuna parte — nostri test compresi.
* **Matrice di handoff machine-readable** per il riallineamento esterno: un
  artefatto versionato che elenca, per ogni messaggio curato, il testo e i
  quattro assi. Serve a chi mantiene `plenora-contracts` per riallineare senza
  leggere il nostro codice, ed è generato dal codice, non scritto a mano.

## 13. Un conflitto fra due ratifiche, da sciogliere prima di implementare

Misurando i siti è emerso che **S6 e S9 si contraddicono** su un punto preciso,
ed entrambe le posizioni sono ratificate e argomentate.

**S6 ha ratificato che il valore ricevuto compaia nel messaggio.** Sta scritto
nel design S6 e nel codice:

> Il valore ricevuto **compare** nel messaggio. Non è una violazione della
> redazione: un'opzione arriva dal chiamante — riga di comando o API — non dal
> payload del file, e nasconderla renderebbe l'errore inutile proprio a chi deve
> correggerlo.

**S9 ha ratificato che nessun costruttore pubblico accetti `&str` non
`'static`.** Una chiave di `format_options` scritta male dall'utente è, per
definizione, non statica: non esiste nello schema, è per questo che viene
rifiutata.

### Superficie interessata

| Sito | Messaggio |
|---|---|
| `format_options::valida_opzioni` ×3 | nomina la chiave sconosciuta, la fase, il valore fuori grammatica |
| `format_options::booleano` | nomina la chiave |
| `driver-csv`, `driver-xls`, `driver-geoparquet` | `'{altro}' non riconosciuto` |
| `driver-shp` `publish_mode` | `'{other}' non valido` |

E tre asserzioni dei test S6 che verificano proprio questo:
`contains("optzione_inesistente")`, `contains("zstsd")`, `contains(chiave)`.

### Le tre uscite

**(a) Vince S9.** I messaggi smettono di nominare la chiave o il valore
sbagliato, ed elencano solo ciò che è accettato — che è `&'static str`, viene
dallo schema. I test S6 vengono riscritti. Costo: chi sbaglia a scrivere deve
confrontare da sé il proprio input con l'elenco. Su una mappa di opzioni è
fattibile; è comunque una perdita rispetto a ciò che S6 ha ratificato.

**(b) Vince S6, con un'eccezione stretta.** Un tipo opaco — `OpzioneRiferita` —
costruibile **solo** dentro `plenora-io-model`, con costruttori `pub(crate)`.
`PublicMessage` guadagna una variante che lo porta. L'API pubblica continua a
non accettare nessun `&str` non statico, quindi la lettera del vincolo S9 regge;
l'unico posto che può coniare quel tipo è il validatore delle opzioni, che è
nostro. Il modello di minaccia di INV-10 riguarda il **payload**, e una chiave
di configurazione non lo è.

**(c) Il valore va nell'`ErrorContext` invece che nel messaggio.** Escluso: il
DTO dovrebbe emetterlo in un campo wire nuovo, e la struttura di
`plenora-io-error-v1` deve restare invariata.

**Raccomandazione: (b).** Tiene la proprietà che S6 ha ratificato e argomentato,
e la paga con un tipo che confina l'eccezione a un punto solo invece di lasciare
una scorciatoia generale. (a) è difendibile ma cancella una decisione presa
consapevolmente tre passi fa, e lo farebbe come effetto collaterale di un
vincolo scritto pensando ad altro — il testo delle dipendenze C, non le opzioni
della riga di comando.

**Questo punto va sciolto prima di scrivere codice**: da esso dipendono la forma
di `PublicMessage`, otto siti e tre test.

## 14. Errata normativa (2026-08-19) — `RejectedOptionToken`

La proprietà di redazione non è più «nessun testo runtime». È:

> **Nessun testo runtime, salvo il token bounded di un'opzione rifiutata
> prodotto dal validatore centrale.**

È un'**eccezione normativa esplicita**, non un'interpretazione implicita di S9:
sta scritta qui, nel design S6 e nel pacchetto decisionale, e vale solo per ciò
che segue.

### Forma richiesta

* tipo opaco `RejectedOptionToken`;
* campi privati e **costruttore privato del modulo `format_options`** — non
  `pub(crate)`: la differenza è che nemmeno il resto del modello può coniarlo;
* nessun `From<String>`, nessun `From<&str>`, nessun `Deserialize`, nessun
  costruttore unchecked, **nessun accessor alla stringa originale**;
* creabile **soltanto** da `valida_opzioni`;
* **unica** variante di `PublicMessage` autorizzata a contenere testo runtime;
* **rendering bounded**: caratteri di controllo, virgolette e backslash
  escaped; troncamento deterministico;
* **nessun uso** per payload, percorsi, nomi letti dai file o testi delle
  dipendenze.

L'ultima riga è quella che tiene: il token esiste per una chiave di
configurazione che l'utente ha digitato, e per niente altro. Un secondo uso lo
trasformerebbe nella scorciatoia generale che (b) esisteva per non aprire.

### Test aggiuntivi richiesti

| Test | Cosa dimostra |
|---|---|
| newline e caratteri di controllo | non entrano grezzi nel wire |
| token molto lungo | troncato, in modo deterministico |
| costruzione da codice esterno | non compila |
| ogni altro costruttore di `PublicMessage` | non accetta testo runtime |

I test S6 sui refusi sicuri — `contains("optzione_inesistente")`,
`contains("zstsd")` — **restano validi**: è la proprietà che (b) conserva.

## 15. Riconciliazione del censimento (121 → 111)

Il primo numero era **corretto come conteggio di siti**; il secondo era un
sottoinsieme, e la coincidenza fra «121 occorrenze di segnaposto» e «121 siti»
ha reso la differenza invisibile. Misurato con un solo metodo, sui siti:

| | Siti |
|---|---|
| `format!` che contengono `{e}` o `{error}` | **121** |
| di cui nella forma stretta `"prefisso: {e}"` | 111 |
| **differenza** | **10** |

I dieci, identificati uno per uno e non stimati:

| File | Righe | Forma | Perimetro |
|---|---|---|---|
| `driver-filegdb/src/lib.rs` | 745, 748, 757, 874, 877, 1402 | `"… '{}': {e}"` — un `{}` posizionale **oltre** a `{e}` | **dentro**: codice spedito sotto `gdal-backend` |
| `plenora-bench/src/main.rs` | 742, 744 | `"{}: {error}"` | fuori: attrezzaggio |
| `plenora-fuzz/src/main.rs` | 257, 279 | `"lettura {}: {error}"` | fuori: attrezzaggio |

**Perimetro S9 per il testo di dipendenza: 117 siti** — 111 nella forma stretta
più i 6 di FileGDB, che oltre al testo della dipendenza portano un nome di
campo o di layer, quindi vanno trattati anche per quello.

I 4 in `plenora-bench` e `plenora-fuzz` sono esclusi per la stessa ragione per
cui sono esclusi dalla copertura: non sono codice spedito. L'esclusione è
dichiarata qui, non silenziosa.
