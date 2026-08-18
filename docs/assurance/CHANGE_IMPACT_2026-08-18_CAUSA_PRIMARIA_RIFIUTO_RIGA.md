# Change impact analysis — la causa primaria di una riga rifiutata sopravvive all'assenza di `input_total`

Data: 2026-08-18.
Baseline: `ed1ede3` (FZ-0.2.1).

## Problema

Quando la validazione di scrittura rifiuta delle righe, `write_rejection_error`
costruisce un errore `DataMapping`/`Write` con allegato un report
`plenora-io-row-diagnostics-v1`. Il report **pretende** `input_total` positivo:
è il suo contratto, e `RowDiagnostics::validate_write` lo verifica.

Ma `input_total` è **opzionale** per chi scrive: si dichiara con
`declare_input_total` prima del primo `write`, e nessuno obbliga a farlo.
Quando mancava, la funzione tornava:

```rust
return PlenoraIoError::Contract(
    "input_total esatto richiesto prima della validazione row-scoped".to_owned(),
);
```

cioè `InvalidPlan` / `Validate` / codice `Contract`.

Il difetto non è che l'errore fosse sbagliato in sé: è che **sostituiva la causa
primaria con una condizione dell'infrastruttura diagnostica**. La riga era
invalida — un nullo su un contratto non nullable, una geometria non
convertibile — e chi leggeva l'errore vedeva un problema interno al prodotto.
Categoria, fase e causa erano tutte e tre diverse dal vero.

Incontrato scrivendo un WKB da 40 MB durante FZ-0.2: la riga veniva rifiutata
per un limite, e l'errore parlava di `input_total`.

## Cosa cambia

Quando `input_total` manca, l'errore conserva **categoria, fase, driver, causa e
ragione di capability**; solo il report non viene allegato.

| | Prima | Dopo |
|---|---|---|
| categoria | `InvalidPlan` | `DataMapping` |
| fase | `Validate` | `Write` |
| causa | perduta | nel messaggio |
| `driver` | assente | valorizzato |
| `capability_reason` | perduta | conservata |
| report | assente | assente |
| totale | — | **non inventato** |

Il report resta assente per una ragione precisa, non per rinuncia: allegarlo con
un totale che nessuno ha dichiarato produrrebbe un documento che **afferma** un
fatto che non c'è. Un report mancante è una lacuna dichiarata; un report con un
totale inventato è una lacuna nascosta dentro un dato che sembra buono.

La causa passa nel messaggio perché senza report non avrebbe dove stare. È un
vocabolario chiuso — `contract.nullability`, `conversion.invalid_geometry`,
`test.rejected` — non un valore derivato dal payload, che non potrebbe uscire.

## Verifica

### Un test asseriva il difetto

`row_scoped_write_rejection_without_input_total_is_a_contract_error` pretendeva
esattamente il comportamento sbagliato: codice `Contract`, categoria
`InvalidPlan`, fase `Validate`. Non era un caso non coperto — era un caso
coperto **al contrario**, e il gate lo difendeva.

È stato riscritto, non cancellato: ora si chiama
`…_keeps_the_primary_cause` e asserisce le cinque proprietà che devono
sopravvivere, più la controprova con il totale dichiarato — senza la quale
«nessun report» potrebbe voler dire «il report non funziona più» invece di «qui
non è emettibile».

### End-to-end, con le due proprietà che il livello core non può mostrare

In `driver-geoparquet`, geometria non nullable e batch con un nullo, senza
dichiarare il totale:

| Proprietà | Come |
|---|---|
| categoria e fase | `DataMapping` / `Write` |
| causa | il messaggio contiene `contract.nullability` |
| il totale non è inventato | `row_diagnostics.is_none()` |
| l'errore non parla d'altro | il messaggio **non** contiene `input_total` |
| **nessuna pubblicazione** | il file di destinazione non esiste dopo il `drop` del writer |
| **nessun output parziale** | la directory di destinazione resta vuota |

Le ultime due contano perché un rifiuto che lasciasse uno staging orfano
sposterebbe il problema invece di fermarlo.

### Per mutazione

Ripristinato il `return PlenoraIoError::Contract(…)`, il test end-to-end
fallisce con `left: InvalidPlan, right: DataMapping`. Ripristinata la
correzione, torna verde: il test osserva la correzione, non sé stesso.

## Perimetro e rischi residui

Toccati: `crates/plenora-io-core/src/driver.rs`,
`crates/driver-geoparquet/src/lib.rs` (soli test).

Non toccati: il contratto `plenora-io-row-diagnostics-v1`, che continua a
pretendere `input_total` positivo. Rilassarlo sarebbe stato l'altra strada —
report sempre presente, totale dichiarato assente con un `knowledge_limit` — ma
cambia un contratto pubblico per un caso che il chiamante può chiudere da sé
dichiarando il totale.

Residui dichiarati:

* **Senza `input_total` non c'è report, quindi non ci sono esempi né conteggi.**
  Chi vuole le diagnostiche complete deve dichiarare il totale. Ora l'errore
  dice *cosa* è andato storto, non *quali righe*.
* **La causa nel messaggio è la prima violazione**, non tutte. Con violazioni di
  cause diverse nello stesso batch, il messaggio ne nomina una sola. Con il
  report ci sarebbero i conteggi per causa; senza, la scelta è fra una causa e
  nessuna.
