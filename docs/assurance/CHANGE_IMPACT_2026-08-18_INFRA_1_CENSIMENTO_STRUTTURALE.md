# Change impact analysis — censimento strutturale al posto del censimento per riga

Data: 2026-08-18. Sigla: **INFRA-1**.
Baseline: `82a256e` (S6).

## Problema

`check_wkb_limits_defaults.py` censiva le occorrenze legittime di
`WkbLimits::default()` con chiave `percorso:riga`. La chiave era fragile per
costruzione: qualunque modifica **sopra** un'occorrenza — una dichiarazione
aggiunta, una riformattazione, un commento — la spostava, e il gate diventava
rosso a codice invariato.

Non è un'ipotesi. È successo in S6: le dichiarazioni di `SCHEMA_OPZIONI` hanno
spostato le due occorrenze legittime da `driver-gpkg:1673` a `1681` e da
`driver-shp:2460` a `2513`. Stesso codice, stessa ragione, gate rosso.

Il danno non è il minuto perso a riallineare. È che un gate che si accende sui
movimenti insegna a riallinearlo **senza guardare**: dopo la terza volta, il
numero si aggiorna per far tornare il verde, e quel giorno passa anche
un'occorrenza vera.

## Cosa cambia

La chiave è ora `percorso::funzione`, con il **numero di occorrenze attese
dentro quella funzione**:

```python
LEGITTIME: dict[str, tuple[int, str]] = {
    "crates/driver-gpkg/src/lib.rs::__fuzz_gpkg_geometry": (1, "…"),
    "crates/driver-shp/src/lib.rs::__fuzz_wkb_roundtrip":  (1, "…"),
}
```

La funzione che racchiude un'occorrenza si ricava dal sorgente già spogliato di
commenti e stringhe: si bilanciano le tonde dopo `fn nome` per saltare generics
e argomenti, si prende la prima graffa, e si risale alla corrispondente. Un `;`
a tonde chiuse è una dichiarazione di trait senza corpo e non apre nessun
intervallo. Fra più intervalli che contengono la posizione vince quello che
comincia più tardi: le funzioni annidate sono rare in Rust ma esistono, e
attribuire l'occorrenza a quella esterna renderebbe la chiave meno stabile
proprio dove serve.

Un'occorrenza fuori da ogni funzione — un `static` o un `const` a livello di
modulo — prende la chiave `<modulo>`, che non è in `LEGITTIME`: è quindi rossa,
com'è giusto, e il messaggio lo dice invece di attribuirla a una funzione
qualsiasi.

### Tre condizioni rosse invece di una

| Condizione | Perché |
|---|---|
| chiave assente da `LEGITTIME` | è la condizione che c'era: una nuova occorrenza in un punto non classificato |
| conteggio diverso da quello censito | **nuova**: una seconda `WkbLimits::default()` dentro una funzione già censita non è coperta dalla ragione scritta per la prima |
| voce di `LEGITTIME` senza occorrenza nel codice | **nuova**: una voce che sopravvive al proprio codice tiene in vita una ragione che nessuno rilegge |

La seconda esiste perché è il buco che la chiave per funzione aprirebbe se
portasse solo il nome: senza conteggio, aggiungere un default accanto a uno
censito passerebbe.

`verifica(radice)` è ora separata da `main()` e restituisce
`(violazioni, conteggi)`, così le sonde girano su un albero finto invece che
sui file veri — che è la convenzione degli altri gate del repository.

## Verifica

### Nove sonde, sui due obblighi opposti

Un gate del genere ha due doveri che tirano in direzioni contrarie, e vanno
provati **entrambi**: se si prova solo la tolleranza, la tolleranza mangia il
gate; se si prova solo la severità, torna la fragilità che INFRA-1 chiude.

| Sonda | Obbligo |
|---|---|
| `l_albero_conforme_passa` | riferimento |
| `lo_spostamento_verticale_non_accende_il_gate` | tolleranza — 40 righe inserite sopra |
| `la_riformattazione_non_accende_il_gate` | tolleranza — chiamata spezzata su più righe |
| `rinominare_il_file_intorno_non_conta` | tolleranza — una funzione nuova inserita sopra |
| `una_occorrenza_in_una_funzione_nuova_e_rossa` | severità — la condizione storica |
| `una_seconda_occorrenza_nella_stessa_funzione_e_rossa` | severità — il buco che il conteggio chiude |
| `una_occorrenza_fuori_da_ogni_funzione_e_rossa` | severità — `static` a livello di modulo |
| `una_voce_che_sopravvive_al_proprio_codice_e_rossa` | severità — nessun fantasma nel censimento |
| `la_motivazione_nel_commento_non_conta_come_occorrenza` | lo spoglio, che il gate già faceva |

Le sonde entrano nella CI accanto al gate, come per
`check_prevalidazione_decoder` e `check_quarantena_fuzz`.

### La prova diretta sull'albero reale

Il gate nuovo è stato eseguito **sull'albero al commit `efada48`** (pre-S6, con
l'occorrenza a riga 2460) e sull'albero corrente (riga 2513). Stesso censimento,
verde in entrambi:

```
WkbLimits::default() censiti: 2 legittimi in produzione (per funzione, non per
riga), zero residui, 47 nei test, 4 nell'attrezzaggio
```

È esattamente lo spostamento che aveva acceso la chiave vecchia.

## Perimetro e rischi residui

Toccati: `scripts/check_wkb_limits_defaults.py`,
`scripts/test_check_wkb_limits_defaults.py` (nuovo), `.github/workflows/ci.yml`.

Non toccati: nessun sorgente Rust, nessun contratto, nessun altro gate.

Residui dichiarati:

* **I conteggi aggregati `test` e `attrezzaggio` restano numeri globali** (47 e
  4). Non risentono degli spostamenti — solo di occorrenze genuinamente nuove,
  che è il loro scopo — ma non dicono *dove*. Renderli strutturali
  significherebbe 51 voci di censimento per un rischio che non c'è: il tetto
  non governa nulla né in un test né nel fuzz harness.
* **Il riconoscimento della funzione è sintattico.** Una macro che genera un
  corpo di funzione contenente `WkbLimits::default()` verrebbe attribuita alla
  funzione che la invoca, o a `<modulo>` se invocata fuori. Nessun caso del
  genere esiste oggi nel workspace; se comparisse, la chiave resterebbe stabile
  ma meno parlante.
* Lo stesso schema di censimento per riga **non è usato altrove**: gli altri
  gate contano per crate (`check_assurance_fallbacks`) o verificano proprietà
  strutturali. Nessun altro gate ha questa fragilità da correggere.
