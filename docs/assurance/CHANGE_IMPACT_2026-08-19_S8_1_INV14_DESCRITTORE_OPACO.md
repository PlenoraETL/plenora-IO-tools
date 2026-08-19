# Change impact analysis — `FormatDescriptor` opaco, costruito solo da `const_new` (INV-14)

Data: 2026-08-19. Sigla: **S8.1**.
Baseline: `e47db0f` (INFRA-1.2). Non cambia la matrice di S8.

## Problema

`FormatDescriptor` aveva ventitré campi pubblici e si costruiva con un literal.
Due conseguenze, ed è la seconda che conta.

La prima è ordinaria: i campi pubblici sono superficie, e ogni lettura diretta
lega un consumatore alla rappresentazione invece che al significato.

La seconda è il motivo per cui INV-14 esiste. Con un literal, **aggiungere un
campo è un'operazione che si può fare senza guardare i driver**: basta dargli un
`Default`, o dichiararlo `Option`, e nove descrittori su dieci restano al valore
che qualcun altro ha scelto. È esattamente il modo in cui `read_mode` è arrivato
a conflare tre assi — nessuno ha mai dovuto decidere, driver per driver, che
cosa quel campo dicesse davvero.

## Cosa cambia

* `FormatDescriptor` è `#[non_exhaustive]`;
* i ventitré campi sono **privati**;
* ventitré getter `const`, uno per campo;
* `const fn const_new(...)` con **tutti** i campi obbligatori, `read_mode`
  legacy compreso e esplicito;
* i dieci driver dichiarano `static DESCRIPTOR = FormatDescriptor::const_new(…)`.

**Nessun mapping automatico fra `read_mode` e `native_read_mode`.** `const_new`
li prende entrambi come parametri distinti. Derivare l'uno dall'altro
cancellerebbe l'informazione per cui S8 è stato fatto: i due divergono in sette
driver su dieci, e la divergenza è il dato.

I commenti che motivavano ogni valore sono stati portati sull'argomento
corrispondente, non persi nella conversione: erano la ragione del valore, e un
argomento posizionale senza ragione è peggio di un campo nominato senza ragione.

### Il contratto pubblico non cambia — verificato, non assunto

Il catalogo prodotto dopo la migrazione è **identico byte per byte** a quello
della baseline S8:

```
$ diff catalogo-S8.json catalogo-S8.1.json   # nessuna differenza
```

`serde` serializza i campi privati come prima. Per questo `descriptor_version`
**non** viene bumpato: la versione traccia lo schema del catalogo, e lo schema è
lo stesso. Bumparla per una modifica invisibile direbbe ai consumatori di
guardare qualcosa che non è successo.

## Verifica

### Il compile test di INV-14, in quattro casi

Sono doctest su `FormatDescriptor`, quindi vivono accanto all'invariante che
descrivono e falliscono con essa:

| Caso | Atteso |
|---|---|
| literal con campi nominati | non compila |
| aggiornamento funzionale `{ id: …, ..base }` | non compila |
| lettura diretta `descriptor.id` | non compila |
| lettura via getter `descriptor.id()` | **compila** |

Il quarto non è decorazione: senza, i primi tre passerebbero anche se il tipo
sparisse.

### Il resto della suite

561 test, 27/27 gate verdi. La migrazione ha toccato circa 220 letture di campo
in sedici file, e il valore di quella conversione sta tutto nel fatto che il
compilatore l'ha guidata: nessuna è stata dedotta.

**Due errori della migrazione automatica, entrambi presi dal compilatore.** La
sostituzione testuale ha convertito anche `options.format_options` e
`ResolvedCrs.id` — campi veri di altri tipi, che per un attimo sono diventati
chiamate a metodi inesistenti. Il secondo era sotto `--features gdal-backend`,
quindi il build predefinito non lo vedeva: l'ha preso il gate che compila con
tutte le feature. È la ragione per cui quel gate esiste, e vale la pena
notare che una migrazione «meccanica» su un file di testo non è meccanica
finché non compila.

## Errata al pacchetto decisionale

Il pacchetto prevedeva per INV-14 «tutti i campi privati; accesso solo via
getter». Resta vero verso l'esterno. Dentro il crate c'è **una** eccezione
dichiarata: `con_write_capabilities`, `#[cfg(test)]` e `pub(crate)`, che
restituisce un descrittore nuovo con altre capability di scrittura.

Serve ai test delle capability, che costruiscono varianti di uno stesso
descrittore cambiando un campo solo. L'alternativa era riesporre il campo —
cioè togliere l'invariante per comodità di un test, che è il modo in cui
un'invariante smette di valere. Un costruttore test-only che *restituisce un
valore nuovo* non permette la mutazione che l'invariante vieta.

## Perimetro e rischi residui

Toccati: `plenora-io-core/src/descriptor.rs` (struct, getter, `const_new`,
compile test), i dieci driver, `plenora-io-core/src/{capabilities,driver,registry,request}.rs`,
`plenora-io-cli/src/{main,conformance_tests}.rs`, il pacchetto decisionale.

Non toccati: **comportamento a runtime, nessuno**; il catalogo, verificato
identico; la matrice S8, invariata.

Residui dichiarati, tutti **accettati** e non da chiudere qui:

* `DeliverySemantics::Streaming` e le due `BufferingStrategy` non usate restano
  dichiarabili e non implementate.
* `native_read_mode` resta una **dichiarazione**, non una misura. Nessun gate
  testuale la verifica, e non ne va introdotto uno: un gate che cercasse
  `StagedSpool` o `read_to_end` nei driver sarebbe fragile in entrambe le
  direzioni — falso positivo su un uso legittimo, falso negativo su una
  materializzazione scritta altrimenti. Il valore è garantito dalla revisione,
  che è quanto questo asse può promettere oggi.
* `const_new` ha ventitré argomenti posizionali. È il prezzo del costruttore
  `const`: un builder darebbe leggibilità ma non `const`, e senza `const` i
  descrittori non potrebbero essere `static`. Gli argomenti sono nell'ordine dei
  campi, e i commenti li accompagnano.
