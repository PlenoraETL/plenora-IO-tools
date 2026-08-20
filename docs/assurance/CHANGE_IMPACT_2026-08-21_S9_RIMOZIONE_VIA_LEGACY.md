> Documento append-only: correzioni e seguiti vanno in coda, non nel corpo.

# Change impact analysis — S9: rimozione della via legacy

Data: 2026-08-21. Sigla: **S9 / rimozione legacy**.
Baseline: `95d42c6` (tranche 14, `plenora-io-cli`).
`plenora-io-error-v1` **invariato, incluso l'insieme esatto delle chiavi**.

**Validazione di livello 1.** Verificato, non *release-qualified*.

## Problema

Le quattordici tranche hanno portato il censimento a zero **nella produzione**.
I costruttori che accettano testo libero però esistevano ancora, e finché
esistono la garanzia di INV-10 è una convenzione sorvegliata da un gate: basta
non eseguire il gate perché sparisca.

Questo commit toglie la funzione. **La garanzia diventa l'assenza.**

## Cosa è stato rimosso

Undici costruttori pubblici che accettavano `String`, `impl Into<String>` o
testo runtime libero:

| Costruttore | Firma | Esito |
|---|---|---|
| `Contract`, `Unsupported`, `Schema`, `Crs`, `Wkb`, `LimitExceeded`, `OutputExists` | `(String)` | **rimossi** |
| `capability` | `(…, detail: impl Into<String>)` | **rimosso** |
| `format` | `(driver, reason: impl Into<String>)` | **rimosso** |
| `crs_unresolved` | `(driver, &RawCrs)` | **rimosso** (sostituito da `crs_non_risolto_redatto`) |
| `new` | `(…, message: impl Into<String>)` | **reso privato** |

`new` non è stato cancellato: resta la base interna di `reader_busy`,
`projection_unsupported`, `cancelled`, `Io` e `Json`, che compongono il proprio
messaggio da valori tipizzati e **non accettano testo dal chiamante**. Privata,
non è più raggiungibile da un consumatore, che è la proprietà richiesta.

`Io(std::io::Error)` e `Json(serde_json::Error)` **restano**: accettano un errore
tipizzato, non testo. Il loro messaggio interpola `error.kind()` e riga/colonna —
un enum chiuso e due numeri. Non erano nel perimetro di questa rimozione.

## Inventario della ricostruzione

Ogni voce è stata riverificata sull'albero finale, non dedotta dai passi:

| Voce | Atteso | Misurato |
|---|---|---|
| usi legacy nei test della CLI | 6 migrati | 6 — righe 1684, 1715, 1918, 1922, 1947, 2220 |
| usi legacy nei test del model | 17 migrati | 0 residui nel censimento |
| harness fuzz | 1 migrato | `limite_redatto` + import di `PublicMessage` |
| definizioni legacy in `error.rs` | 0 | **0** |
| `fn new` privata | presente | presente, non `pub` |
| blocchi doctest in `error.rs` | 12 → 25 | 25 (13 aggiunti) |
| di cui `compile_fail` | 5 → 11 | 11 (6 aggiunti) |
| di cui positivi | 7 aggiunti | 6 gemelle + 1 controprova |

## Il censimento arriva a coprire tutto il codice Rust

| Perimetro | Prima | Ora |
|---|---|---|
| produzione sotto `crates/` | contata | contata |
| codice di **test** | **esclusa** | **contata** |
| **doctest** | invisibile a `spoglia` | **letta** |
| `crates/plenora-bench`, `crates/plenora-fuzz` | escluse | **contate** |
| `fuzz/fuzz_targets/` | **fuori dall'albero scandito** | **contata** |

**Nessuna nuova allowlist.** Due esclusioni sono state *tolte*
(`ATTREZZAGGIO`) e l'unica esclusione introdotta è **semantica, non
nominativa**: i blocchi `compile_fail` e `ignore` non contano perché sono per
definizione codice che *non* viene compilato. Contarli significherebbe rossare
proprio la prova che la via legacy non esiste più.

La regola sui test **si è rovesciata, ed è voluto**: finché la migrazione
procedeva un crate per volta, la via legacy nei test era la copertura della via
che si stava smantellando. Ora i costruttori non esistono, e una chiamata in un
test non è copertura: è codice che non compila.

## La prova che la rimozione è avvenuta

Tre prove indipendenti, e nessuna delle tre basta da sola.

### 1. I doctest da consumatore esterno, in coppie

I doctest di `plenora-io-model` compilano come **crate separati** che dipendono
dalla libreria: vedono la stessa superficie di un consumatore.

**Un `compile_fail` da solo prova poco**, e il documento lo dice invece di
lasciarlo intendere: passa se il blocco non compila per una ragione *qualunque*
— un import sbagliato basta. Annotare il codice d'errore atteso non aiuta:
**è stato verificato** che `compile_fail,E0277` resta verde dove l'errore vero è
`E0624`. Rustdoc non lo impone, e l'annotazione è stata tolta invece di
lasciarla sembrare una garanzia.

Le prove sono perciò in **coppie**: ogni blocco `compile_fail` è il blocco
positivo che lo precede — che compila e passa le sue asserzioni — **più una
riga**, quella che usa l'API vietata. Stessi import, stessi tipi, stesse
chiamate permesse. Se il negativo fallisse per una ragione diversa dalla riga
aggiunta, il positivo fallirebbe con lui e la coppia diventerebbe rossa.

La non vacuità è **strutturale**, non affermata.

Le sei righe vietate: `Contract(String)`, `redatto(…, &String)`,
`schema_redatto(&format!(…))`, `format(…)`, `new(…)`, `LimitExceeded(String)`.

### 2. La controprova positiva

Sulla superficie intera: testo curato, numero strutturale nel messaggio, e i
quattro assi scelti uno per uno. Prova che la via nuova non solo compila, ma
**serve a qualcosa**.

### 3. Il gate sul sorgente, indipendente da rustdoc

`scripts/check_errori_redatti.py` verifica che le definizioni **non siano
tornate a esistere**. È la prova che i `compile_fail` non possono dare, perché
non passa da rustdoc.

## Il quartetto: non-vacuo, e zero differenze

| | prima | ora |
|---|---:|---:|
| file tracciati | 27 | **28** |
| funzioni tracciate | 130 | **131** |
| differenze sulle voci esistenti | — | **0** |

Lo snapshot è cambiato **per sole aggiunte**: il diff non modifica nemmeno una
entrata esistente.

La voce nuova è `fuzz/fuzz_targets/harness.rs::convert`, e non è un sito nuovo:
è un sito **nuovo alla vista**. `check_quartetto_sito.py` importa `sorgenti` dal
censimento, quindi estendere quella funzione a `fuzz/` ha allargato in silenzio
anche il perimetro del quartetto.

Il quartetto è stato verificato **contro il costruttore rimosso, letto da git**:

| | `LimitExceeded(String)` (rimosso) | `limite_redatto` |
|---|---|---|
| category | ResourceLimit | ResourceLimit |
| phase | Validate | Validate |
| remote_effect | None | None |
| retry | Never | Never |
| code | LimitExceeded | LimitExceeded |

L'accoppiamento fra i due gate è ora **scritto nel codice**: chi tocca
`sorgenti` allarga due gate, non uno.

## `plenora-io-error-v1` invariato

Le sei chiavi restano `category`, `phase`, `remote_effect`, `retry`, `code`,
`message`, più `row_diagnostics` opzionale. Sono fissate da due test su **due
vie diverse** — `map_err` per gli errori dei driver, `usage_err` per gli errori
d'uso della CLI — introdotti nella tranche 14.

---

# L'incidente: la prima corsa delle sonde è stata distruttiva

Questa sezione non è una nota. **I risultati della prima corsa sono ritirati**:
non valgono come evidenza, né a favore né contro.

## Che cosa è successo

Le sonde negative del gate mutavano un file, eseguivano il gate, e ripulivano
con `git checkout -- <file>`. I file mutati avevano **modifiche non
committate**: il ripristino li ha riportati a `HEAD`, cancellando la migrazione
dei test del model, la rimozione dei costruttori, i doctest appena scritti e la
migrazione dell'harness fuzz.

## Perché i risultati sono invalidi, e non solo «sospetti»

Le sonde 2, 3, 4 e 5 hanno tutte riportato *«`pub fn new` è tornata a
esistere»*. Sembrava che il gate le stesse cogliendo. **Non era così**:
`error.rs` era tornato a `HEAD` sotto di loro, e il gate segnalava la
definizione ripristinata invece della mutazione della sonda.

Ogni sonda misurava un albero diverso da quello che credeva di misurare. In
particolare il rosso della sonda sui blocchi `compile_fail` **non è la prova di
un difetto**: era confuso dallo stesso ripristino. Nessuno di quei risultati è
stato riusato.

## La correzione, che non è «stare attenti»

| Difesa | Come |
|---|---|
| workspace non scrivibile | montato `:ro`: una sonda che provasse a scrivere **fallisce** invece di riuscire di nascosto |
| copia indipendente | `cp -r`; verificato inode diverso e link count 1 |
| nessun `git` | il ripristino di ogni sonda copia dalla sorgente in sola lettura |

Il metodo del controllo hard link è stato **validato separatamente**: un hard
link vero condivide l'inode e porta il link count a 2, `cp -r` produce inode
nuovo con link count 1. Senza questa controprova, «inode diversi» sarebbe stato
un numero senza significato dimostrato. La controprova *dentro* la corsa non era
eseguibile con il mount in sola lettura, quindi è un secondo esperimento e non
lo stesso: va letta così.

## La sonda di non distruttività

Sentinella inserita nell'albero, digest di **99 file** prima e dopo:

```
digest prima:  99 file
digest dopo:   99 file
diff:          nessuna differenza
sentinella:    presente
git status:    identico
```

È il controllo che avrebbe colto l'incidente al primo giro.

## Le sonde, rieseguite da zero

**8 su 8 corrette, 0 rosse.** Nessun risultato riusato dalla corsa invalida.

| Sonda | Atteso | Esito |
|---|---|---|
| albero intatto | verde | verde |
| definizione legacy ricomparsa | rosso | rosso |
| chiamata legacy in produzione | rosso | rosso |
| chiamata legacy in un test | rosso | rosso |
| chiamata legacy in un doctest | rosso | rosso |
| chiamata legacy in `fuzz/` | rosso | rosso |
| blocco `compile_fail` escluso | verde | verde |
| blocco `ignore` escluso | verde | verde |

Le sonde non sono rimaste in uno script di lavoro: sono state portate nel suite
`scripts/test_check_errori_redatti.py`, che il checkpoint invoca. **Da 7 a 12**
— due riscritte al contratto nuovo, cinque aggiunte.

---

## Verifica di livello 1

* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: pulito;
* `cargo clippy --workspace --all-targets -- -D warnings`: pulito;
* `cargo test --workspace --all-features` e `cargo test --workspace`: verdi;
* `cargo test --workspace --all-features --doc`: verde;
* `scripts/check_errori_redatti.py`: **0 residui in 0 crate**, su produzione,
  test, doctest e `fuzz/`;
* `scripts/check_quartetto_sito.py`: 28 file, 131 funzioni, **0 differenze**;
* **14 sonde** derivate da `scripts/s9-checkpoint.sh`, non rielencate a mano;
* `check_assurance_fallbacks`: 119; `check_assurance_n1 --integrita`: verde;
* `cargo +nightly fuzz build`: verde.

## Che cosa resta

I test ostili conclusivi e il checkpoint finale con baseline `0474902`.
Separati e release-blocking: i 45 gruppi di ASSURANCE-N1 e le due lacune fuzz
(reader `.shp`/`.dbf`, spike FileGDB). Aperto e non ancora valutato: il debito
sul contratto dei report di perdita.

---

## Addendum del 2026-08-21 — `'static` non prova l'origine letterale

**Il corpo resta com'era.** Questo addendum non corregge un errore di fatto:
aggiunge un limite che il corpo lasciava intendere più forte di com'è.

### L'affermazione da qualificare

Il corpo dice che i doctest `compile_fail` provano che «una `String` non entra».
È vero **per il tipo**, e non basta:

> `&'static str` garantisce la **durata**, non la **provenienza**.

Un chiamante deliberato promuove testo runtime a `'static` con `Box::leak` e lo
infila in un `PublicMessage::Curated` senza che il compilatore obietti.

La dimostrazione era già dentro questo lavoro, e non l'avevo letta come tale: i
test sul tetto del messaggio ottenevano i propri statici lunghi **proprio con
`Box::leak`**. Uno dei due risale alla tranche 2 — l'illusione di provenienza
c'era da allora.

### La garanzia realistica

I crate sono interni e `publish = false`. L'avversario di questo invariante è la
distrazione, non un aggressore:

> S9 impedisce la propagazione **accidentale** di testo runtime nel workspace;
> non rende crittograficamente inconiabile un messaggio dinamico da codice
> ostile.

### Le tre conseguenze, applicate

| | |
|---|---|
| statici dei test | `Box::leak` sostituito da `concat!` su letterali (`otto_volte!`), che produce un letterale: la provenienza è letterale **per costruzione**. Due occorrenze, una della tranche 2 |
| divieto | `scripts/check_niente_leak.py`: nessun `Box::leak` / `String::leak` / `Vec::leak` / `.leak()` in tutto il workspace — produzione, **test** e **doctest**. L'unica occorrenza mai esistita era in un test, e un divieto limitato alla produzione non l'avrebbe intercettato. 14 sonde. Registrato nel checkpoint |
| dichiarazione | la doc di `PlenoraIoError` porta ora una sezione «Che cosa questa garanzia **non** è», con un doctest che **usa** `Box::leak` e che **compila e passa** |

Quel doctest è deliberato: una garanzia descritta più forte di com'è è peggio di
una garanzia dichiarata con il suo limite. Il limite è ora **provato**, non
ammesso a parole.

### La proprietà non è «zero occorrenze»

Una prima stesura di questo gate escludeva **tutti** i doctest, per non rossare
la dimostrazione. Era sbagliato, e in un modo che vale la pena nominare: è una
deroga **più ampia di una allowlist**. Un `Box::leak` in un qualunque altro
esempio della documentazione — cioè nella prima cosa che un consumatore copia —
sarebbe rimasto invisibile.

La proprietà verificata è ora:

> zero occorrenze non autorizzate; **una sola** dimostrazione eseguibile e
> identificata.

L'identità è il marcatore `DIMOSTRAZIONE-LIMITE-STATIC` **dentro il blocco**,
non un numero di riga: un'identità per riga si stacca al primo commit che sposta
il file. È la stessa lezione di INFRA-1.

Il gate diventa rosso in tutti e quattro i modi in cui l'attestazione può
rompersi, e ognuno ha la sua sonda:

| Rottura | Esito |
|---|---|
| un'occorrenza in un doctest non attestato | rosso |
| il marcatore usato in un altro file | rosso — l'attestazione è legata al file, altrimenti ci si autorizza copiando un commento |
| due occorrenze attestate | rosso — una deroga che cresce non è più una deroga |
| l'attestazione sopravvive alla propria occorrenza | rosso — stessa regola delle voci fantasma del censimento |

«Mantenere il doctest eseguibile» **si difende da sé**: l'estrattore condiviso
esclude `ignore` e `compile_fail`, quindi marcare così la dimostrazione la
renderebbe invisibile al gate, che conterebbe zero attestazioni e diventerebbe
rosso.

Che cosa sia un doctest lo decide `doctest_che_devono_compilare`, **riusata** dal
censimento: due definizioni diverse divergerebbero, e divergerebbero in
silenzio.
