# Decision package — contratto dei report di perdita

Data: 2026-08-21. **Nulla di quanto segue è implementato.** Il documento esiste
per essere ratificato o corretto; ogni sezione si chiude con una
raccomandazione, non con una decisione presa.

Riferimento: `docs/assurance/DEBITO_contratto_report_di_perdita.md`, aperto alla
tranche 13 di S9.

---

## 0. Il perimetro è più grande di quanto il debito dichiarasse

Il debito originario nominava `LossReport.counts`. Il censimento fatto per
questo documento mostra **due** superfici sul wire, e la seconda è più esposta
della prima.

### Che cosa esce davvero, e dove

Il contratto `plenora-io-convert-v1` (e le buste di `inspect`/`layers`/`read`)
serializza:

```json
{
  "read_fidelity":  { "level": "...", "reasons": [ { "code": "...", "detail": "..." } ] },
  "write_fidelity": { ... },
  "read_loss":      { "lossless": bool, "counts": { "<categoria>": <u64> } },
  "write_loss":     { ... }
}
```

| Superficie | Sul wire | Contenuto |
|---|---|---|
| `LossReport.counts` | **sì** | chiavi di categoria, testo libero |
| `FidelityAssessment.reasons[].detail` | **sì** | testo libero |
| `LossReport.examples[].context` | **no** | `loss_doc` non lo emette |

**`examples` non è sul wire della CLI.** Resta nel tipo Rust e va deciso
comunque, ma la sua urgenza è diversa e il documento lo tiene separato.

### La superficie più esposta è `detail`, non `counts`

`crates/plenora-io-core/src/driver.rs`, quattro siti:

```rust
format!("{}: attributo '{}' non nativo o loss-reported", layer.name, field.name())
format!("{}: tipo {:?} di '{}' richiede coercion", layer.name, field.data_type(), field.name())
format!("{}: nullability di '{}' definita dal formato", layer.name, field.name())
format!("{}: geometrie multipart esplose in entità DXF", layer.name)
```

Portano sul wire, senza tetto in byte:

* **nomi di layer e di attributo** — per una lettura vengono dal file;
* **`{:?}` di `arrow::DataType`** — formattazione `Debug` di una dipendenza,
  che nessuno ha promesso di tenere stabile.

È esattamente la classe che S9 ha chiuso nei messaggi d'errore, sopravvissuta
su un altro contratto di wire perché non è un `PlenoraIoError`.

`FidelityReason` ha **già** la forma che S9 ha scelto — `code` tipizzato accanto
a `detail` libero. Metà del lavoro esiste dalla nascita del tipo; è la metà
libera che non è mai stata chiusa.

### Il vocabolario di `counts` mescola due lingue

| Forma | Esempi |
|---|---|
| identificatore macchina | `gpkg_integer_column_real_truncated`, `gpkg_integer_column_real_saturated`, `inconsistent_crs_representations` |
| **prosa italiana** | `"CIRCLE tassellata"`, `"ARC tassellato"`, `"blocco INSERT esploso"`, `"MultiPolygon esplosa in entità DXF"`, `"coercion tipo attributo"` |
| composto a runtime | `format!("{representation}_not_preserved_{category_suffix}")` — due vocabolari chiusi |
| **composto da payload** | `format!("attributo non rappresentato in DXF: {c}")` — `c` è un nome di colonna |

Lo stesso campo porta identificatori parsabili e frasi in italiano. Un
consumatore non può fare né l'una né l'altra cosa in modo affidabile: non può
`match`are, e non può mostrare all'utente perché metà del vocabolario è in
inglese-macchina.

**Questo è indipendente dalla limitatezza, e a mio avviso è il difetto più
grave dei due.**

### Limiti oggi

| Grandezza | Tetto |
|---|---|
| cardinalità di `counts` | nessuno **nel contratto**; di fatto ~`max_columns` = 4 096 per le categorie per-colonna |
| lunghezza di una chiave | **nessuno** |
| numero di `reasons` | 64 (`MAX_FIDELITY_REASONS`) |
| lunghezza di un `detail` | **nessuno** |
| numero di `examples` | 64 (`MAX_LOSS_EXAMPLES`) |
| byte totali della busta | **nessuno** |

La cardinalità è quindi delimitata *indirettamente*, da un limite che esiste per
un'altra ragione e che il chiamante può alzare. I byte no.

Per confronto: dopo S9 un messaggio d'errore ha `MAX_MESSAGE_BYTES = 2048`.

---

## 1. Struttura delle categorie e degli identificatori

### Opzioni

| | Descrizione | Costo |
|---|---|---|
| **A** | `counts: BTreeMap<String, u64>` invariato, con una convenzione scritta | nessuna garanzia: una convenzione senza gate è ciò che S9 ha appena finito di smontare |
| **B** | enum `LossCategory` chiuso, `#[non_exhaustive]`, serializzato `snake_case` | rompe i consumatori che leggono le chiavi attuali; obbliga a nominare ogni categoria |
| **C** | enum chiuso **più** una variante di scarico `Other { … }` tipizzata | non rompe l'estensibilità, ma la variante di scarico diventa la porta che tutti useranno |

### Raccomandazione: **B**, con la coppia `(categoria, contesto)`

```rust
pub struct LossEntry {
    pub category: LossCategory,          // enum chiuso, #[non_exhaustive]
    pub scope: Option<ContractIdentifier>, // il nome, tipizzato — non nel testo
    pub count: u64,
}
```

Tre ragioni:

1. il vocabolario di fatto è **già chiuso**: quattordici valori distinti,
   nessuno costruito da input libero salvo il sito DXF e i due composti da
   vocabolari chiusi. Nominarli in un enum non toglie nulla che esista;
2. risolve il problema delle due lingue **per costruzione**: il codice è
   parsabile, e la presentazione all'utente è del consumatore;
3. il nome di colonna del sito DXF smette di entrare nella chiave e diventa uno
   `scope` tipizzato — la stessa mossa della tranche 2 sugli errori, con lo
   stesso effetto: la cardinalità della mappa torna a dipendere solo dal
   vocabolario.

**Contro, e va pesato:** `Other` non esiste in B. Un driver nuovo che voglia
riportare una perdita non prevista deve aggiungere una variante, cioè toccare
`plenora-io-core`. È attrito deliberato — ma è attrito reale, e se lo si giudica
eccessivo l'opzione C resta disponibile al prezzo scritto sopra.

### Su `FidelityReason.detail`

Raccomandazione: **`detail: Option<PublicMessage>`**, cioè la stessa via di S9.
`code` è già tipizzato; i nomi di layer e attributo passano a un campo
`scope: Option<ContractIdentifier>`; `{:?}` di `DataType` diventa
`ArrowTypeClass::nome()`, che esiste già ed è stato introdotto proprio per
questo nella tranche 2.

Non richiede tipi nuovi: richiede di usare quelli che S9 ha costruito.

---

## 2. Limiti: cardinalità, dimensione per stringa, byte totali

I tre sono **indipendenti** e vanno ratificati separatamente: un tetto sulla
cardinalità non limita i byte, e un tetto per stringa non limita il totale.

### Raccomandazione

| Grandezza | Valore | Ragione |
|---|---|---|
| cardinalità di `counts` | **256** voci | il vocabolario di fatto ne ha 14; 256 lascia spazio a `scope` distinti senza permettere una voce per colonna |
| lunghezza di uno `scope` | riuso di `ContractIdentifier` | il tetto è già il suo, non se ne inventa un altro |
| `reasons` | **64**, invariato | già in vigore, non c'è ragione di muoverlo |
| **byte totali della busta di perdita** | **32 KiB** | è il tetto che oggi non esiste in nessuna forma, ed è l'unico che limita davvero ciò che un consumatore riceve |

Il tetto sui byte totali è quello che raccomando con più convinzione, perché è
l'unico che non può essere aggirato componendo grandezze ciascuna sotto il
proprio limite.

**Non** raccomando di riusare `MAX_MESSAGE_BYTES = 2048`: un report di perdita è
un aggregato e 2 KiB lo troncherebbero nell'uso normale, non solo in quello
ostile. Se il numero 32 KiB è arbitrario — e lo è — va discusso; ma il fatto che
un tetto debba esistere non lo è.

---

## 3. Politica di redazione

**Qui la regola di S9 non si applica per analogia, e questa è la sezione in cui
raccomando con meno forza.**

Un messaggio d'errore dice *che cosa è andato storto*; il chiamante conosce già
il proprio input. Un report di perdita dice *che cosa si è perso*, e il suo
valore sta proprio nel dire **quale** colonna, **quale** layer. Chiuderlo come
si sono chiusi i messaggi lo renderebbe meno utile nella parte per cui esiste.

### Opzioni

| | Politica | Effetto |
|---|---|---|
| **A** | nessun nome esce | il report diventa un istogramma anonimo: si sa che 3 attributi sono stati persi, non quali |
| **B** | i nomi escono, ma **solo** in un campo tipizzato `scope: ContractIdentifier` | il testo resta curato; il nome è dove un consumatore lo cerca |
| **C** | come B, e in più un `LossExample` con il valore | reintroduce payload sul wire |

### Raccomandazione: **B**

`ContractIdentifier` è costruibile solo da un contratto validato. Per una
scrittura il nome è del chiamante; per una lettura viene dal file, ed è la
stessa proprietà già accettata per il campo `field` degli errori — che peraltro
**non** è sul wire v1, mentre `scope` lo sarebbe. La differenza va ratificata
esplicitamente, non fatta scivolare.

**`{:?}` di tipi di dipendenza esce in ogni caso**, e su questo non vedo
compromessi: `Debug` non è un formato promesso, e `ArrowTypeClass::nome()`
esiste già.

---

## 4. Comportamento al limite, deterministico

Il tetto non serve se ciò che accade quando lo si tocca non è definito.

### Raccomandazione

| Regola | |
|---|---|
| **si tronca, non si rifiuta** | una conversione riuscita non deve fallire perché il *report* è grande: l'output è valido, è la diagnostica a essere incompleta |
| **la perdita è dichiarata nella busta** | un campo `truncated: true` e un `omitted: <u64>`. Un report troncato in silenzio dice «tre categorie» dove ce n'erano trecento, ed è peggio di nessun report |
| **l'ordine è deterministico** | `BTreeMap` ordina già per chiave; con un enum si ordina per discriminante. Nessun ordine di inserimento, nessun hash |
| **si tiene ciò che conta di più** | a parità di tetto si conservano le voci con `count` maggiore, non le prime incontrate |

L'ultima regola è quella che raccomando di discutere: «le prime N» è più semplice
e più prevedibile; «le N maggiori» è più utile ma dipende dall'ordine di
elaborazione fino alla fine. La seconda richiede di raccogliere tutto e poi
tagliare, cioè di **non** limitare la memoria — che è metà della ragione per cui
i tetti esistono.

Se si sceglie la semplicità: **le prime N in ordine di categoria**, con
`omitted` che dice quante ne mancano.

---

## 5. Compatibilità e versionamento della busta CLI

Le buste interessate sono `plenora-io-convert-v1` e le buste di
`inspect`/`layers`/`read`, che portano `fidelity` intero.

**Qualunque delle opzioni sopra è una rottura per un consumatore che legga le
chiavi di `counts` o il testo di `detail`.**

### Opzioni

| | | Costo |
|---|---|---|
| **A** | bump a `plenora-io-convert-v2`, `v1` rimosso | rottura netta, una sola forma da mantenere |
| **B** | `v2` accanto a `v1`, `v1` deprecato | due forme da tenere allineate; è il modo in cui i campi divergono |
| **C** | `v1` invariato, i campi nuovi **aggiunti** accanto ai vecchi | nessuna rottura, ma il vecchio campo resta non bounded — cioè il problema non è risolto, è affiancato |

### Raccomandazione: **A**

C è la tentazione ovvia e non risolve niente: la superficie non bounded resta
sul wire accanto a quella bounded. B raddoppia il lavoro e, per esperienza di
questo repository, produce due verità che divergono in silenzio.

La rottura è accettabile per la stessa ragione ratificata nella decisione 2 di
S9: la chiave di compatibilità è ciò che è dichiarato tale, e il testo di
`detail` non lo è mai stato.

**Va però ratificato esplicitamente che `counts` e `detail` non sono chiavi di
compatibilità**, come si fece per `message`. Senza quella ratifica la rottura è
una rottura e basta.

---

## Che cosa serve dopo la ratifica

Non è nel perimetro di questo documento, ma va detto perché pesa sulla
decisione:

* un gate analogo a `check_errori_redatti.py` per il vocabolario delle
  categorie, con le sue sonde;
* i test ostili estesi alla busta di perdita — oggi coprono la busta d'errore;
* il tetto sui byte totali verificato con una fixture che lo supera, non solo
  con un'asserzione sul valore della costante.

## Che cosa questo documento **non** afferma

* non afferma che ci sia una vulnerabilità: nessun byte del payload risulta
  raggiungere `counts`; i nomi vengono da contratti e schemi, e la cardinalità
  è delimitata di fatto da `max_columns`;
* non afferma che l'implementazione debba cominciare: le cinque decisioni sono
  aperte, e tre di esse (struttura, redazione, versionamento) cambiano il lavoro
  in modo sostanziale a seconda di come vengono prese.
