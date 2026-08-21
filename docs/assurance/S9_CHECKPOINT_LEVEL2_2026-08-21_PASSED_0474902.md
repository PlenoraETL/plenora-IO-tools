> Documento append-only: correzioni e seguiti vanno in coda, non nel corpo.

# S9 — checkpoint di livello 2 su `0474902`, superato

**Questo documento non governa la readiness di alcun componente né del sistema.**
`SYSTEM_RC_GATE.md` non è stato modificato.

## Esito, per voci separate

Le cinque righe vanno lette **separate**. Nessuna sintesi le tiene insieme,
perché rispondono a domande diverse e fonderle è il modo in cui un verde
parziale si legge come un verde intero.

```
checkpoint S9:              passed
registro ASSURANCE-N1:      integro
debito ASSURANCE-N1:        45 gruppi su 49 — release blocked
fuzz Shapefile / FileGDB:   lacune aperte
release_authorized:         false
```

```
perimetro:                      checkpoint intermedio S9
revisione verificata:           047490230a4fc7bdc99b1bae0088cc34bd972341
albero al momento della misura: pulito (0 file non committati)
albero a fine misura:           pulito
strumento:                      scripts/s9-checkpoint.sh
baseline differenziale:         effc4abe3f74ade083dbed72c94c286748809d9f
```

### Che cosa è chiuso, e che cosa no

È chiuso **il bucket S9 della verifica retrospettiva dei quartetti**: dei 49
gruppi differenziali, nessuno resta a carico di S9.

**S9 nel suo insieme non è chiuso.** Restano `driver-dxf` (20 usi legacy),
`plenora-io-cli` (6), la rimozione dei costruttori legacy e i test ostili
conclusivi.

Le due affermazioni sono diverse, e la prima senza la sua qualificazione si
legge come la seconda.

## Integrità della misura

| Criterio | Esito |
|---|---|
| SHA in testa e in coda | identici |
| albero alla misura e a fine corsa | pulito, entrambe |
| passi dichiarati nello script | 46 |
| passi eseguiti | 46 |
| passi verdi | 46 |
| passi **saltati** | 0 |
| identificatori duplicati | nessuno |

`verdi + falliti = passi dichiarati`, con `falliti` vuoto. Il conteggio non
basterebbe da solo: `salta()` conta i passi non eseguiti fra i passi **e** fra i
falliti, quindi un 46/46 con `falliti` non vuoto sarebbe un fallimento
travestito.

## Misure

### Copertura totale — due proiezioni, non due misure della stessa cosa

| Derivazione | Numeratore / denominatore | Valore | Soglia |
|---|---|---:|---:|
| record `DA:` del report LCOV | 26 724 / 31 097 | **85,94%** | 80% |
| colonna «Lines» di `llvm-cov` | — | **84,05%** | 80% |

Entrambe sopra soglia, entrambe richieste. **Non sono intercambiabili**: contano
insiemi diversi di righe strumentate a partire dallo stesso profdata. Il
denominatore differisce di circa il 3-4%.

Regioni 84,80%, funzioni 79,08% (colonna cargo).

La prima è quella che gli altri gate leggono: `check_coverage_exclusions` e la
diagnostica differenziale usano **lo stesso file**.

### Fuzz

| | |
|---|---|
| replay deterministico | **33 596 input** su 13 target, nessun crash |
| smoke 13/13 | senza finding, 0 in quarantena |

### Gate di S9

| | |
|---|---|
| censimento costruttori legacy | 26 residui in 2 crate; 12 componenti a zero |
| quartetto per sito | 27 file, 130 funzioni, invariato |
| registro fallback | 119 |

## Diagnostica differenziale

**49,35%** sulle righe cambiate fra `effc4ab` e `0474902`: 381 coperte, 391
scoperte.

Non è una soglia e non ne ha una. Il numero da solo non dice nulla: dice dove
guardare, e **la classificazione è già stata fatta** —
`S9_MATRICE_GRUPPI_DIFFERENZIALI.md`, 49 gruppi con i due assi che riconciliano
entrambi a 49.

Le righe scoperte non sono debito causato da S9: verificato su `effc4ab` che le
stesse righe avevano `conteggio=0` **prima** della migrazione.

## Che cosa resta aperto

### ASSURANCE-N1 — 45 gruppi, release-blocking

`docs/assurance/ASSURANCE_N1_copertura_negativa.json`, con disposizione e nota
obbligatorie per ognuno.

Il gate ha due modalità, e **il verde dell'una non è il verde dell'altra**:

* `--integrita` — verde: il registro è coerente. **Non dice che i rami siano
  coperti**, e lo stampa;
* `--release` — rossa: 45 gruppi su 49 senza copertura. Cablata nella qualifica
  di release.

### Lacune fuzz — non chiuse dall'averle dichiarate

| Lacuna | Che cosa serve |
|---|---|
| nessun fuzz target per il parsing `.shp` / `.dbf` | un target del **reader reale**. `shp_wkb` esercita la conversione WKB ⇄ shape: presentarlo come copertura del formato sarebbe falso |
| nessun fuzz target per FileGDB | uno **spike bounded**, con due soli esiti ammessi: target reale, oppure impossibilità tecnica dimostrata con eccezione esplicita e suite compensativa di fixture ostili nella matrice feature-on |

Dichiararle serve a non farle sparire, non a risolverle. Entrambe sono dovute
prima della release.

## SHA verificato e SHA del commit documentale

Il commit che **pubblica** questo documento ha un SHA diverso da `0474902`, e
**non eredita alcuna misura**: i numeri valgono per `0474902` e per nessun altro
albero.

## Precedenti

| Revisione | Esito |
|---|---|
| `8d6883f` | non superato — panic in `crs.rs` |
| `107b7b5` | superato |
| `effc4ab` | superato |
| `71adf70` | non superato — compilazione senza feature; copertura non disponibile, dati stale rifiutati |
| `0474902` | superato — questo documento |

Tutte immutate.

---

## Addendum del 2026-08-21 — due criteri di integrità erano più deboli di come sono scritti

**Il corpo resta com'era.** Questo addendum non ritira le misure: ritira la
**forza probatoria** di due righe della tabella «Integrità della misura».

I due limiti sono **distinti** e vanno letti separatamente.

### 1. Non esisteva un controllo automatico finale dell'albero

La riga «albero alla misura e a fine corsa | pulito, entrambe» descrive una
verifica che **lo script non faceva**. `scripts/s9-checkpoint.sh` calcolava
`git status --porcelain | wc -l` **una volta sola**, in testa, e non rileggeva
nulla alla fine.

Peggio del non rileggere: un conteggio non avrebbe comunque colto un passo che
modificasse un file **già** marcato `M`, perché il numero di righe resta
identico.

### 2. Lo SHA finale era la ristampa del valore iniziale

La riga «SHA in testa e in coda | identici» era **vera per costruzione**. Lo
script acquisiva `REVISIONE="$(git rev-parse HEAD)"` in testa e ristampava la
stessa variabile in coda: il confronto non poteva fallire, e non era un
confronto.

Un commit durante la corsa avrebbe lasciato l'albero invariato e spostato
`HEAD`: la misura avrebbe descritto un albero e l'esito ne avrebbe nominato un
altro, senza che nulla lo rivelasse.

### Che cosa resta valido, e su quale base

| | |
|---|---|
| le misure del corpo | **valide**: coperture, replay, smoke, gate ed esiti dei passi non dipendono da questi due controlli |
| l'albero pulito **in testa** | **verificato dallo script**, e per il livello 2 era condizione di partenza con `exit 2` |
| l'albero pulito **a fine corsa** | per `1806276` **osservato a mano**: il commit dell'evidenza, eseguito subito dopo la corsa, ha elencato come non committato il solo documento nuovo. È una constatazione, non una misura dello strumento, e va letta così |
| l'albero pulito a fine corsa, revisioni precedenti | **non documentabile**: non ne esiste una registrazione, e non viene affermato |
| lo SHA invariato | **non verificato** in nessuna delle corse precedenti a `INFRA-7` |

### Correzione

`INFRA-7` sostituisce entrambi i controlli con misure vere:

* `impronta_albero` — sha256 di `git diff --cached --binary --no-ext-diff
  --no-textconv`, `git diff` con le stesse opzioni, e percorso più hash del
  contenuto di ogni file non tracciato e non ignorato, con delimitazione
  esplicita. Confrontata in testa e in coda, e registrata come passo
  `albero_invariato`;
* `revisione_invariata` — `git rev-parse HEAD` **riletto** a fine corsa e
  confrontato con quello iniziale.

Entrambi sono passi veri: contano fra i passi, rossano il checkpoint, e hanno
sonde che li fanno fallire. Fra queste, le decisive: un file già sporco che
cambia lascia il conteggio identico e muove l'impronta; un commit vuoto lascia
l'impronta identica e muove `HEAD`.

**Nessuna evidenza storica è stata riscritta.**
