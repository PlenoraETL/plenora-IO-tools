> Documento append-only: correzioni e seguiti vanno in coda, non nel corpo.

# S9 / INFRA-7 — checkpoint di livello 2 su `1c2707e`

**Questo documento non governa la readiness di alcun componente né del sistema.**
`SYSTEM_RC_GATE.md` non è stato modificato.

## Perimetro

Questa corsa **non riapre S9**, chiuso su `1806276`. Qualifica `INFRA-7`, cioè
una correzione della **forza probatoria dell'harness**: l'elenco chiuso dei
passi pesanti, l'impronta dell'albero e la rilettura della revisione.

```
perimetro:               qualifica di INFRA-7
revisione verificata:    1c2707e7b3727b0e312a824d34ea3ca54b42440e
albero in testa:         0 file non committati
albero in coda:          0 file non committati
impronta iniziale:       e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
impronta finale:         e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
strumento:               scripts/s9-checkpoint.sh
baseline differenziale:  1806276a0dd6c9977672ef229109a8e2afc321e5
release_authorized:      false
```

`1806276` è la baseline pertinente: `0474902` apparteneva al differenziale di
chiusura di S9, e qui il perimetro cambiato è soltanto l'harness.

## Integrità della misura — ora misurata, non asserita

| Criterio | Come è stabilito | Esito |
|---|---|---|
| SHA in testa e in coda | `git rev-parse HEAD` **riletto** a fine corsa | **verde** — SHA riletto identico |
| albero invariato | impronta sha256 confrontata in testa e in coda | **verde** — impronte identiche |
| albero pulito alla conclusione | `git status --porcelain` vuoto | **vuoto** |
| passi dichiarati | | **50** |
| passi verdi | | **50** |
| passi omessi | livello 2: deve essere **0** | **0** |
| passi falliti | | **0** |

È la prima corsa in cui le prime due righe sono **misure**. Nelle evidenze
precedenti la prima era vera per costruzione — la coda ristampava la variabile
della testa — e la seconda descriveva una verifica che lo script non faceva.
L'addendum in coda a quelle evidenze lo registra.

## Il confine dell'impronta, provato da questa corsa

Il livello 2 esegue fuzz e copertura, che **scrivono davvero**: replay e smoke
toccano `fuzz/target/` e `fuzz/corpus/`, `cargo llvm-cov` scrive in `target/`.
Tutti gitignorati.

Che `--exclude-standard` li saltasse era finora dedotto da `.gitignore`, non
misurato. Questa corsa lo misura, e prova il confine nella forma giusta:

> gli artefatti ignorati possono cambiare; nessun contenuto versionabile deve
> cambiare.

Un'impronta che rossasse qui sarebbe stata inutilizzabile al livello 2 — cioè
proprio dove serve.

## Misure

### Copertura

| Derivazione | Valore | Soglia |
|---|---:|---:|
| record `DA:` del report LCOV | **85,77%** (26 635 / 31 055) | 80% |
| colonna «Lines» di `llvm-cov` | **83,86%** (32 246 − 5 204) | 80% |

### Fuzz

| | |
|---|---|
| replay deterministico | **35 098 input** su 13 target, nessun crash |
| smoke | 13 target, senza finding, 0 in quarantena |

### Gate

| Gate | Esito |
|---|---|
| censimento costruttori legacy | **0 residui in 0 crate**; 14 componenti a zero |
| quartetto per sito | 28 file, 131 funzioni, invariato |
| promozioni a `'static` | zero non autorizzate, **una attestata** |
| registro fallback | 119 |

## Diagnostica differenziale

**n/d — nessuna riga eseguibile cambiata** fra `1806276` e `1c2707e`:
0 coperte, 0 scoperte.

È l'esito corretto e va letto per quello che dice: fra le due revisioni
non è cambiata **una riga di Rust**. `INFRA-7` tocca uno script di shell,
un suite di sonde e documenti. Un `n/d` qui non è una misura mancata: è
la misura di un perimetro vuoto.

## Che cosa resta aperto — e blocca la release

Invariato rispetto a `1806276`. `INFRA-7` non tocca nessuna di queste voci.

| Voce | Stato |
|---|---|
| 45 gruppi di ASSURANCE-N1 | release-blocking |
| fuzz target per il reader `.shp` / `.dbf` | aperto |
| spike di fattibilità fuzz per FileGDB | aperto |
| contratto dei report di perdita | aperto, **non valutato**; decisioni in `docs/DECISION-PACKAGE-contratto-report-di-perdita.md` |
| S10–S12 e qualifica cross-component | invariati nel piano |

## SHA verificato e SHA del commit documentale

Il commit che **pubblica** questo documento ha un SHA diverso da `1c2707e`, e
**non eredita alcuna misura**: i numeri valgono per `1c2707e` e per nessun altro
albero.

## Un limite dell'impronta, trovato da questa corsa

L'impronta di `1c2707e` è `e3b0c442…`, che è **lo sha256 della stringa vuota**:
su un albero pulito e senza file non tracciati, i tre componenti non producono
byte. È corretto, ed è anche una proprietà utile — al livello 2 l'albero deve
essere pulito, quindi qualunque valore diverso da quella costante significa che
qualcosa è stato scritto.

**Ma lo stesso valore si ottiene se `git` fallisce del tutto.** `impronta_albero`
sopprime stderr con `2>/dev/null`; eseguita fuori da un repository restituisce
la stessa costante. Verificato:

```
cd /tmp && { git diff --cached --binary 2>/dev/null; git diff --binary 2>/dev/null; } | sha256sum
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
```

È esattamente la famiglia di difetto che questa serie insegue: **un valore che
significa due cose**. Lo avevo già incontrato senza riconoscerlo — la prima
prova end-to-end di `INFRA-7`, eseguita per sbaglio da `/`, stampò quella
costante e io la lessi come «impronta calcolata».

### Perché la qualifica di `1c2707e` regge lo stesso

`git` ha dimostrabilmente funzionato in questa corsa, e non per deduzione:

* `REVISIONE` è calcolata con `git rev-parse HEAD` **senza** soppressione, e ha
  prodotto un SHA valido in testa e in coda;
* cinquanta passi hanno girato dentro il repository, fra cui gate che leggono
  file tracciati;
* `git status --porcelain` è stato eseguito e ha risposto.

L'ambiguità riguarda la **funzione**, non questa misura.

### Seguito proposto — non applicato

Distinguere i due casi costa una riga: far fallire `impronta_albero` se
`git rev-parse --is-inside-work-tree` non risponde, invece di sopprimere e
restituire il vuoto. Non è stato applicato qui perché cambierebbe lo SHA appena
qualificato, e una correzione dell'harness va qualificata a sua volta.

Registrato come seguito, non come debito accettato: la differenza è che un
seguito ha un rimedio scritto.

---

## Addendum del 2026-08-21 — INFRA-7.1 chiude l'ambiguità, e cambia il valore

**Il corpo resta com'era.** L'addendum non ritira nulla: registra il seguito
proposto in fondo al corpo, ora applicato, e una conseguenza che va letta prima
di confrontare numeri.

### Il seguito è stato applicato

`impronta_albero` **acquisisce prima e hasha dopo**. La versione qualificata su
`1c2707e` convogliava i tre comandi in una pipe con `2>/dev/null`: se `git`
falliva, la pipe riceveva zero byte. Ora ogni comando scrive in un file
temporaneo con il proprio controllo d'esito, e l'hash si produce solo se **tutti
e tre** hanno acquisito. La funzione **fallisce** invece di restituire il vuoto,
e i chiamanti gestiscono il fallimento: `exit 2` in testa, passo rosso in coda.

Controllare il solo `rev-parse` non sarebbe bastato, ed è la sonda che lo
dimostra: un `git` fittizio che risponde a `rev-parse` e fallisce su
`diff --cached` deve far fallire l'impronta.

### Il valore di un albero pulito **cambia**, ed è atteso

L'impronta porta ora un prefisso versionato `impronta-albero-v1\0`. Serve a due
cose: nessuna impronta può più valere lo sha256 della stringa vuota — nemmeno su
un albero pulito — e il formato è dichiarato, così un cambio futuro si riconosce
invece di somigliare a un albero cambiato.

| | |
|---|---|
| impronta di questo documento (`1c2707e`, albero pulito) | `e3b0c442…` |
| stesso albero dopo INFRA-7.1 | **diversa**, per costruzione |

**Non è un albero cambiato: è un formato cambiato.** Chi confronta le due
evidenze deve leggerle così.

### Stato di INFRA-7.1

**Verificato, non qualificato.** Livello 1 verde — 41 passi, 9 omessi — e 60
sonde del checkpoint verdi, fra cui le quattro richieste (repository assente,
errore git interno, repository pulito, modifiche ignorate) più una controprova
che l'elenco non chiedeva: **un file versionabile deve muovere l'impronta**.
Senza, «gli artefatti ignorati non la muovono» passerebbe per un confine mentre
potrebbe essere una funzione che non si muove mai.

La qualifica completa coinciderà con il prossimo checkpoint sostanziale.
**La qualifica di `1c2707e` resta valida**: riguarda il codice qualificato
allora, e le sue misure non dipendono dall'ambiguità chiusa qui.
