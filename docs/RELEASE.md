# Rilascio — dove siamo e dove andiamo

I numeri di questo documento vengono da
[`assurance/current-state.json`](../assurance/current-state.json), e `check_docset`
verifica che coincidano. Due verità manuali divergono, e divergono in silenzio.

```
release_authorized: false
```

---

## Stato

### Revisioni

| | SHA | Significato |
|---|---|---|
| baseline documentale | `2fe9b54` | revisione da cui parte il docset corrente |
| ultima qualificata | `75e5301` | ultimo SHA passato da un checkpoint di livello 2 |

**Lo SHA misurato non è il commit che ne pubblica l'evidenza.** Un'evidenza sta
in un commit successivo e non eredita la misura: i numeri valgono per l'albero
misurato e per nessun altro.

### Ultima misura — `75e5301`

| | |
|---|---|
| checkpoint | **57 passi su 57**, 0 omessi, 0 falliti |
| replay deterministico | **36 055 input** su **13 target**, nessun crash |
| smoke | **13 target su 13**, nessun finding |
| quarantena | **vuota** |
| copertura, report LCOV | **85,84%** (26 774 / 31 192 righe strumentate) |
| copertura, colonna cargo | **83,94%** |
| soglia | 80% |

Le due percentuali sono **due proiezioni dello stesso profdata**, non due
misure della stessa cosa: contano insiemi diversi di righe strumentate.
Entrambe sono richieste.

Il conteggio dei passi è **riconciliato dagli identificatori** — 57 distinti,
nessun duplicato — e non accettato dal rapporto che lo strumento stampa su se
stesso.

La diagnostica differenziale rispetto a `0fb799d` è **n/d**: nessuna **riga
Rust strumentata dalla misura LCOV** è cambiata. Sono cambiati documenti, script
Python e shell, e commenti Rust — nessuno dei quali entra in quella misura.

Non è una misura mancata, ed è la ragione per cui la formulazione conta: dire
«nessuna riga eseguibile è cambiata» sarebbe falso, perché gli script cambiati
sono eseguibili. Semplicemente, la copertura non li osserva.

### Chiuso

**S9 — errori strutturati.** Il censimento dei costruttori che accettano testo
libero è a **zero** su quattordici componenti: produzione, test, doctest e
target di fuzzing. I costruttori non esistono più, quindi la garanzia è
l'assenza della funzione e non una convenzione sorvegliata.

Qualificato su `1806276`.

### Aperto

| | Stato |
|---|---|
| ASSURANCE-N1 | **43 gruppi aperti** su 49 |
| fuzz del reader Shapefile | aperto |
| spike FileGDB | aperto |
| contratto `LossReport` | **non ratificato** |
| S10, S11, S12 | aperti |
| qualifica cross-component | aperta |

Ognuna di queste voci **blocca il rilascio**.

---

## Roadmap

L'ordine è quello di lavoro, non di importanza. Ogni punto dichiara che cosa
serve per uscirne e quale blocco rimuove.

Nessuna stima temporale è presentata come impegno. Ciò che si sa del costo è
scritto dove è stato misurato.

### 1. Chiusura dei 43 gruppi ASSURANCE-N1

**Criterio di uscita.** Ogni gruppo del registro è `chiuso`, con una prova che è
un **test eseguito**, oppure `irraggiungibile` con le righe scoperte e la
guardia che rifiuta per prima. `check_assurance_n1.py --release` diventa verde.

**Blocco rimosso.** I rami d'errore negativi smettono di essere non verificati.

**Costo.** Il costo dominante non è scrivere i test ma **determinare quali rami
siano raggiungibili**: in un gruppo su tre affrontati finora, un solo ramo su
tre lo era. Quella determinazione non si parallelizza e non si fa leggendo i
commenti.

### 2. Fuzz target del reader `.shp` / `.dbf`

**Criterio di uscita.** Un target che esercita il **parsing reale** di `.shp` e
`.dbf`, non la conversione geometrica.

`shp_wkb` converte fra WKB e forme ESRI: presentarlo come copertura del formato
sarebbe falso, ed è la ragione per cui questo punto esiste separatamente.

**Blocco rimosso.** L'unico driver con un parser di formato non esercitato da
alcun fuzzing entra nella stessa copertura degli altri.

### 3. Spike FileGDB bounded

**Criterio di uscita.** Due esiti sono ammessi, e nessun terzo:

1. un fuzz target reale per il percorso FileGDB;
2. **impossibilità tecnica dimostrata**, con eccezione esplicita e una suite
   compensativa di fixture ostili nella matrice `gdal-backend`.

Lo spike è *bounded*: se non converge a uno dei due esiti, il risultato è il
secondo con la dimostrazione, non un rinvio.

**Blocco rimosso.** L'unico driver che dipende da una libreria C esterna smette
di essere l'unico senza copertura di fuzzing né compensazione dichiarata.

### 4. Ratifica e implementazione di `LossReport`

**Criterio di uscita.** Le cinque decisioni sono ratificate e implementate:
struttura delle categorie, limiti — cardinalità, byte per stringa, byte totali —,
politica di redazione, comportamento deterministico al limite, versionamento
della busta.

La superficie è già sul wire, quindi qualunque scelta è un cambio di contratto e
richiede una nuova versione.

**Blocco rimosso.** L'ultima superficie pubblica senza contratto ratificato ne
acquista uno. Vedi [PRODUCT.md § LossReport](PRODUCT.md#lossreport--non-ratificato).

### 5. S10, S11, S12

| Lotto | Perimetro |
|---|---|
| **S10** | validazione completa di GeoParquet 1.1 |
| **S11** | `wkb_shape` ispeziona i figli delle collection |
| **S12** | parsing bounded di WKT e GeoJSON, fuzz dedicato, capability `hostile_input_hardened` |

**Criterio di uscita.** Ciascun lotto chiuso con il proprio checkpoint di
livello 2 e la propria evidenza.

**Blocco rimosso.** Il perimetro del componente è completo. S12 in particolare
rimuove l'ultima asimmetria fra i formati: oggi WKT e GeoJSON hanno tetti, ma
non una capability dichiarata che li renda verificabili dall'esterno.

### 6. Qualifica cross-component

**Criterio di uscita.** La catena `IO-tools → data-tools → database-tools` è
qualificata in **entrambe le direzioni**, su fixture con revisioni, piattaforma,
comandi ed esiti registrati.

Il perimetro e l'harness sono di **proprietà esterna**: questo repository non
contiene né esegue test che compilino gli altri due componenti. La definizione
è in [`release/system-rc-gate.json`](../release/system-rc-gate.json).

**Blocco rimosso.** La readiness di sistema smette di essere non verificata.
Resta distinta dalla readiness del componente: nessuna delle due implica
l'altra.

### 7. Decisione finale di rilascio

**Criterio di uscita.** Tutti i punti precedenti chiusi;
`check_release_contract.py --release` verde, cioè nessun invariante
`release_blocking`; un checkpoint di livello 2 su un albero pulito, con SHA e
impronta invariati; l'evidenza in un commit separato.

Solo allora `release_authorized` può diventare `true`, e sarà una decisione
scritta — non la conseguenza automatica di sei caselle verdi.
