> Documento append-only: correzioni e seguiti vanno in coda, non nel corpo.

# Checkpoint di livello 2 su `0fb799d` — ASSURANCE-N1 tranche 1 e INFRA-7.1

**Questo documento non governa la readiness di alcun componente né del sistema.**
`SYSTEM_RC_GATE.md` non è stato modificato.

## Perimetro

Qualifica lo SHA complessivo, che contiene **sia** la tranche 1 di ASSURANCE-N1
**sia** INFRA-7.1. Quest'ultimo era «verificato, non qualificato» dal commit
`d62652c`: **`d62652c` non viene qualificato retroattivamente** — la qualifica
vale per `0fb799d`, che lo contiene.

```
perimetro:               ASSURANCE-N1 tranche 1 + qualifica di INFRA-7.1
revisione verificata:    0fb799df1bcd8e7a4e7598ac9f63b14d4447006c
albero in testa:         0 file non committati
albero in coda:          0 file non committati
impronta iniziale:       3ef0f10e2187fe1b3c7c91224f4abb3fefe51dbf442ad7f67918f74118e91bbd
impronta finale:         identica
strumento:               scripts/s9-checkpoint.sh
baseline differenziale:  1c2707e7b3727b0e312a824d34ea3ca54b42440e
release_authorized:      false
```

## Integrità della misura

| Criterio | Come è stabilito | Esito |
|---|---|---|
| SHA in testa e in coda | `git rev-parse HEAD` **riletto** a fine corsa | identici |
| albero invariato | impronta sha256 confrontata in testa e in coda | identiche |
| albero pulito alla conclusione | `git status --porcelain` | vuoto |
| **passi riconciliati dagli identificatori** | vedi sotto | **54 = 54** |
| passi omessi | livello 2: deve essere 0 | **0** |
| passi falliti | | **0** |

### La riconciliazione, e perché non basta il rapporto stampato

Lo strumento stampa «54/54». Quel numero viene da **due contatori interni**: se
un passo fosse registrato due volte, o se un identificatore comparisse due volte
con esiti diversi, il rapporto resterebbe plausibile. È un'affermazione dello
strumento su se stesso.

Il conteggio è stato ricostruito **dalle righe stampate**, una per passo, con il
proprio identificatore:

```
identificatori distinti:   54
righe di passo osservate:  54
  verde 54   ROSSO 0   omesso 0   SALTATO 0   NON AUTORIZZATO 0
identificatori duplicati:  nessuno
dichiarati dallo strumento: verdi=54 totale=54
RICONCILIATO
```

### L'impronta di un albero pulito è una costante verificabile

`3ef0f10e…` coincide con `sha256("impronta-albero-v1\0")`, verificato a parte.
Sotto il prefisso versionato di INFRA-7.1, un albero pulito e senza file non
tracciati **ha un'impronta nota**: qualunque valore diverso, al livello 2,
significa che qualcosa è stato scritto.

## ASSURANCE-N1 — tranche 1

| | |
|---|---|
| gruppi aperti | **43** (erano 45) |
| gruppi chiusi nella tranche | `driver-xls::open`, `driver-xls::create` |
| lasciato aperto deliberatamente | `driver-xls::validate_archive_ratio` — 1 ramo su 3 coperto, **nessuna compensazione** |
| prove **eseguite** | 6 su 2 configurazioni: 4 coprono un ramo, 2 provano un'irraggiungibilità |

I quattro gate di ASSURANCE-N1 hanno girato **per la prima volta dentro il
checkpoint**: non erano cablati, e da quando il livello 1 deriva dallo script
avevano smesso di girare del tutto.

`assurance_n1_prove` verde significa che i sei test dichiarati sono stati
**eseguiti e sono passati**, non soltanto nominati. La differenza è misurata:
marcando un test `#[ignore]`, quel gate diventa rosso e il gate statico resta
verde.

## Misure

### Copertura

| Derivazione | Valore | Soglia |
|---|---:|---:|
| record `DA:` del report LCOV | **85,83%** (26 773 / 31 192) | 80% |
| colonna «Lines» di `llvm-cov` | **83,94%** (32 383 − 5 200) | 80% |

Regioni 84,87%, funzioni 79,41%.

### Fuzz

| | |
|---|---|
| replay deterministico | **35 562 input** su 13 target, nessun crash |
| smoke 13/13 | senza finding, 0 in quarantena |

### Gate

| Gate | Esito |
|---|---|
| censimento costruttori legacy | 0 residui in 0 crate |
| quartetto per sito | 28 file, 131 funzioni, invariato |
| promozioni a `'static` | zero non autorizzate, una attestata |
| registro fallback | 119 |
| ASSURANCE-N1 integrità | 49 gruppi, coerente |
| ASSURANCE-N1 prove eseguite | 6, verificate per esecuzione |

## Diagnostica differenziale

**88,32%** sulle righe cambiate fra `1c2707e` e `0fb799d`: 121 coperte, 16
scoperte su 137 eseguibili.

È il primo differenziale con un perimetro non vuoto dopo la chiusura di S9 — fra
`1806276` e `1c2707e` non era cambiata una riga di Rust.

### Le 16 righe scoperte sono i rami di fallimento dei test stessi

Tutte in `crates/driver-xls/src/lib.rs`, fra 1952 e 2195, e tutte della stessa
natura:

```
1952| panic!(".xls non deve essere aperto da questo driver");
1985| panic!("senza CRS dichiarato la geometria non e' interpretabile");
2091| panic!("{nome}: il piano va rifiutato");
2149| "e' una configurazione non valida, non una capability mancante"
```

Sono i `panic!` degli `else` e le stringhe dei messaggi d'asserzione: **codice
che si esegue solo se il test fallisce**. Un test verde non li tocca per
costruzione, e coprirli richiederebbe di far fallire il test.

Il differenziale è quindi ~100% delle righe cambiate **raggiungibili**, e il
numero 88,32% va letto con questa qualificazione — non come sedici righe di
debito nuovo.

## Che cosa resta aperto — e blocca la release

| Voce | Stato |
|---|---|
| **43 gruppi** di copertura negativa | release-blocking, erano 45 |
| fuzz target per il reader `.shp` / `.dbf` | aperto |
| spike di fattibilità fuzz per FileGDB | aperto |
| contratto dei report di perdita | aperto, non valutato; decisioni in `docs/DECISION-PACKAGE-contratto-report-di-perdita.md` |
| S10–S12 e qualifica cross-component | invariati nel piano |

## SHA verificato e SHA del commit documentale

Il commit che **pubblica** questo documento ha un SHA diverso da `0fb799d`, e
**non eredita alcuna misura**: i numeri valgono per `0fb799d` e per nessun altro
albero.

## Precedenti

| Revisione | Esito |
|---|---|
| `0474902` | superato |
| `86c1bd0` | non superato — `fmt`, 47/48 |
| `1806276` | superato — chiusura di S9 |
| `1c2707e` | superato — qualifica di INFRA-7 |
| `0fb799d` | superato — questo documento |

Tutte immutate.
