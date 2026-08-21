> Documento append-only: correzioni e seguiti vanno in coda, non nel corpo.

# S9 — checkpoint di livello 2 su `1806276`, superato. Chiusura di S9.

**Questo documento non governa la readiness di alcun componente né del sistema.**
`SYSTEM_RC_GATE.md` non è stato modificato.

## Esito, per voci separate

Le cinque righe vanno lette **separate**. Fonderle è il modo in cui un verde
parziale si legge come un verde intero.

```
checkpoint S9:              passed
perimetro S9:               chiuso
registro ASSURANCE-N1:      integro
debito ASSURANCE-N1:        45 gruppi su 49 — release blocked
fuzz Shapefile / FileGDB:   lacune aperte
report di perdita:          debito aperto, non valutato
release_authorized:         false
```

```
perimetro:                      chiusura di S9
revisione verificata:           1806276a0dd6c9977672ef229109a8e2afc321e5
albero al momento della misura: pulito (0 file non committati)
albero a fine misura:           pulito
strumento:                      scripts/s9-checkpoint.sh
baseline differenziale:         047490230a4fc7bdc99b1bae0088cc34bd972341
```

## Che cosa è chiuso, e che cosa no

**Il perimetro di S9 è chiuso.** Il censimento dei costruttori legacy è a
**zero residui in zero crate**, i costruttori che accettavano testo libero non
esistono più, e il divieto è verificato su produzione, test, doctest e `fuzz/`.

**S9 non autorizza una release.** Restano quattro voci, elencate in fondo, che
S9 non copriva e non pretende di aver chiuso.

## Integrità della misura

| Criterio | Esito |
|---|---|
| SHA in testa e in coda | identici |
| albero alla misura e a fine corsa | pulito, entrambe |
| passi dichiarati nello script | 48 |
| passi eseguiti | 48 |
| passi verdi | 48 |
| passi **saltati** | 0 |

`verdi + falliti = passi dichiarati`, con `falliti` vuoto. Il conteggio non
basterebbe da solo: `salta()` conta i passi non eseguiti fra i passi **e** fra i
falliti, quindi un 48/48 con `falliti` non vuoto sarebbe un fallimento
travestito.

## La corsa precedente, su `86c1bd0`, non è passata

Va detto qui e non in nota, perché è la ragione per cui esiste `1806276`.

Il checkpoint su `86c1bd0` è andato **rosso: 47 su 48**, con `fmt`. Il formato
era rotto da `95d42c6` — la tranche 14 — e i tre commit successivi lo hanno
portato avanti. Verificato per bisezione su worktree separati: `0474902`,
l'ultimo SHA passato da un checkpoint, era formattato.

**Il difetto stava nella batteria di livello 1, non nel repository.** La
componevo a mano, e non conteneva `cargo fmt --check`; il checkpoint sì. Quattro
commit dichiarati «verificati a livello 1» erano più deboli del checkpoint di
esattamente quel passo, e leggendo l'esito nessuno poteva accorgersene: la
batteria stampava «FALLITI: nessuno» su un insieme di passi più piccolo di
quello che il livello 1 comprende.

La lezione non è «ricordarsi di formattare», ed è registrata nel design:

> una batteria composta a mano diverge dal checkpoint, e **diverge in silenzio**.

È la stessa regola già applicata alle sonde — estratte dallo script del
checkpoint invece di essere rielencate — che non era applicata ai passi di
build. Corretto in `1806276`.

## Misure

### Copertura totale — due proiezioni, non due misure della stessa cosa

| Derivazione | Numeratore / denominatore | Valore | Soglia |
|---|---|---:|---:|
| record `DA:` del report LCOV | 26 635 / 31 055 | **85,77%** | 80% |
| colonna «Lines» di `llvm-cov` | 32 246 − 5 204 | **83,86%** | 80% |

Entrambe sopra soglia, entrambe richieste. **Non sono intercambiabili**: contano
insiemi diversi di righe strumentate a partire dallo stesso profdata.

Regioni 84,81%, funzioni 79,29% (colonna cargo).

La prima è quella che gli altri gate leggono: `check_coverage_exclusions` e la
diagnostica differenziale usano **lo stesso file**.

### Fuzz

| | |
|---|---|
| replay deterministico | **34 577 input** su 13 target, nessun crash |
| smoke 13/13 | senza finding, 0 in quarantena |

### Gate di S9

| Gate | Esito |
|---|---|
| censimento costruttori legacy | **0 residui in 0 crate**; 14 componenti a zero |
| quartetto per sito | 28 file, 131 funzioni, invariato |
| promozioni a `'static` | zero non autorizzate, **una attestata** (`DIMOSTRAZIONE-LIMITE-STATIC`) |
| registro fallback | 119 |
| catalogo FileGDB reale | verde |

## Diagnostica differenziale

**42,86%** sulle righe cambiate fra `0474902` e `1806276`: 90 coperte, 120
scoperte su 210 eseguibili. Altre 314 righe cambiate non sono eseguibili e
restano fuori misura.

Non è una soglia e non ne ha una. Il numero dice dove guardare, e **la
classificazione è già stata fatta**: `S9_MATRICE_GRUPPI_DIFFERENZIALI.md`, 49
gruppi con i due assi che riconciliano entrambi a 49.

Le righe scoperte sono in gran parte in `driver-dxf`, migrato nella tranche 13.
Non sono debito causato da S9: verificato in precedenza su `effc4ab` che le
stesse righe avevano `conteggio=0` **prima** della migrazione. S9 le ha rese
**visibili**, non non verificate.

## Che cosa S9 ha chiuso

| | |
|---|---|
| tranche 1-14 | quattordici crate migrati alla via redatta |
| rimozione legacy | 11 costruttori pubblici rimossi, `new` resa privata |
| perimetro del censimento | produzione, test, doctest, `fuzz/` |
| prove da consumatore esterno | 6 coppie `compile_fail` + controprova positiva |
| limite di `'static` | dichiarato con un doctest che compila e passa |
| prove ostili | 10 driver, tre fasi, due configurazioni FileGDB |

### Ciò che S9 garantisce, con il suo limite

> S9 impedisce la propagazione **accidentale** di testo runtime nel workspace;
> non rende crittograficamente inconiabile un messaggio dinamico da codice
> ostile.

`&'static str` garantisce la durata, non la provenienza. I crate sono interni e
`publish = false`: l'avversario di questo invariante è la distrazione, non un
aggressore. Il limite è provato da un doctest, non ammesso a parole.

## Che cosa resta aperto — e blocca la release

Nessuna di queste voci è chiusa da questo checkpoint, e nessuna lo era nel
perimetro di S9.

| Voce | Dove | Stato |
|---|---|---|
| 45 gruppi di copertura negativa | `ASSURANCE_N1_copertura_negativa.json` | release-blocking |
| fuzz target per il reader `.shp` / `.dbf` | ASSURANCE-N1, disposizione `seme_fuzz` | aperto |
| spike di fattibilità fuzz per FileGDB | da aprire, bounded, due esiti ammessi | aperto |
| contratto dei report di perdita | `DEBITO_contratto_report_di_perdita.md` | **aperto e non valutato** |

L'ultima riga merita la distinzione: *non valutato* è diverso da *accettabile*.
`LossReport.counts` ha cardinalità senza tetto e nessuna sua stringa ha un
limite in byte, e viene serializzato nel JSON della CLI. Le tre decisioni —
struttura, limiti, redazione — non sono state prese.

`--integrita` di ASSURANCE-N1 è **verde**: il registro è coerente. Non dice che
i rami siano coperti, e lo stampa. `--release` è **rossa**.

## SHA verificato e SHA del commit documentale

Il commit che **pubblica** questo documento ha un SHA diverso da `1806276`, e
**non eredita alcuna misura**: i numeri valgono per `1806276` e per nessun altro
albero.

## Precedenti

| Revisione | Esito |
|---|---|
| `8d6883f` | non superato — panic in `crs.rs` |
| `107b7b5` | superato |
| `effc4ab` | superato |
| `71adf70` | non superato — compilazione senza feature; copertura non disponibile |
| `0474902` | superato |
| `86c1bd0` | **non superato** — `fmt`, 47/48 |
| `1806276` | superato — questo documento, chiusura di S9 |

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
