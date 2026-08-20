# S9 — checkpoint di livello 2, 2026-08-20

**Questo documento non governa la readiness di alcun componente né del sistema.**
Non è, e non va letto come, un'evidenza di release. `SYSTEM_RC_GATE.md` non è
stato modificato.

```
perimetro:                  checkpoint intermedio S9
revisione verificata:       8d6883fb755428cf93c942dd1bb7d39022227ce9
albero al momento della misura: pulito (0 file non committati)
esito:                      NON SUPERATO
release_authorized:         false
promozione di readiness:    nessuna, né di componente né di sistema
```

## Perché era dovuto

Il design § 20 (ratificato il 2026-08-20) prevede un checkpoint completo **ogni
tre driver**. `8d6883f` chiude la terza tranche di driver — `driver-geojson`,
`driver-csv`, `driver-kml`.

## Esito misurato

| Verifica | Esito |
|---|---|
| batteria completa dei gate | **29/31** |
| `fuzz-smoke` 13/13 | **ROSSO** — `target con finding: gpkg_reader` |
| `check_coverage_exclusions --lcov lcov.info` (dentro la batteria) | rosso per assenza dell'input |
| `check_coverage_exclusions --lcov lcov.info` (sul report vero) | **verde** |
| copertura, righe | **84,93%** contro la soglia dell'80% |
| copertura, regioni | 84,84% |
| copertura, funzioni | 78,89% |
| catalogo FileGDB reale | verde |

Due dei rossi vanno letti in modo diverso, e la distinzione è il punto:

**Il rosso della copertura è un difetto dell'harness**, non del codice: la
batteria girava prima della misura, quindi leggeva un `lcov.info` che non
esisteva ancora. Sul report vero il gate passa (`gate_esclusioni=0`).

L'avevo chiamato «artefatto dell'ordine di esecuzione», il che lo faceva
sembrare accettabile. Non lo è: un rosso che si ripete a ogni corsa smette di
essere letto, ed è così che il rosso accanto — quello vero — rischia di passare
con lui. È corretto in INFRA-2 (`107b7b5`).

**Il rosso di `fuzz-smoke` no.** È un finding reale.

## Il finding

`gpkg_reader`, artefatto
`crash-bb81978ddfcd16d81a3bbcef92e1dc71e689335a` (29 353 byte).

```
thread '<unnamed>' panicked at crates/plenora-io-model/src/crs.rs:180:29:
start byte index 9 is not a char boundary; it is inside '齚' (bytes 8..11 of string)
```

**È un difetto nostro, non di una dipendenza.** `wkt_root_epsg` scorreva
`definition.to_ascii_uppercase().as_bytes()` — cioè **indici di byte** — e poi
affettava la **stringa**: `upper[index..].starts_with(marker)`. Quando l'indice
cade dentro un carattere multi-byte, Rust panica. Nel bordo I/O un panic è un
abort: `libfuzzer-sys` lo riporta come «deadly signal».

La definizione di un CRS arriva dal file, quindi il difetto era raggiungibile da
un input ostile. Non serviva niente di esotico: basta un ideogramma dentro le
parentesi di primo livello, prima di dove un `AUTHORITY[` potrebbe comparire.

### Perché lo smoke successivo era verde

Nella stessa corsa, lo smoke 13/13 eseguito subito dopo la batteria ha dato
`smoke=0`. Non è una contraddizione e non annulla il finding: gli artefatti
finiscono in `fuzz/artifacts/`, che **non** viene rigiocato, e sessanta secondi
di mutazioni non hanno riscoperto lo stesso input.

Vale la pena scriverlo perché è il modo esatto in cui un finding può essere
archiviato per errore: una seconda corsa verde sullo stesso target *sembra* una
smentita della prima.

## Chiusura

Il difetto è chiuso da un commit separato, non da una tranche S9: è una
correzione di panic-safety, della stessa famiglia di FZ-0.

* correzione: confronto **sui byte** (`bytes[index..].starts_with(marker.as_bytes())`)
  e `definition.get(…)` invece dell'indicizzazione. Il marcatore è ASCII, e
  nessun byte ASCII compare dentro una sequenza UTF-8 multi-byte: se il
  confronto riesce, `index` è per forza un confine. Il `get` resta comunque,
  perché affidare un panic a una deduzione significa che il giorno in cui la
  deduzione smettesse di valere il difetto tornerebbe ad abortire il processo
  invece di dare `None`;
* test di regressione
  `una_definizione_wkt_multibyte_non_fa_panicare_il_bordo`, **verificato che
  fallisce senza la correzione** con lo stesso panic del fuzzer;
* seme promosso a `fuzz/seeds/gpkg_reader/wkt-multibyte-che-fa-panicare.gpkg`,
  con digest e provenienza in `fuzz/seeds/README.md`. Il seme non è stato
  alterato per ottenere il verde;
* replay dell'artefatto dopo la correzione: **passa**.

## Che cosa resta da fare

Il checkpoint di livello 2 **va rieseguito** sulla revisione che contiene la
correzione. Questo documento qualifica `8d6883f`, e `8d6883f` non ha superato.

**Il commit che pubblica questo documento ha un SHA diverso da quello
verificato**, e non eredita alcuna misura: la copertura e i gate qui riportati
valgono per `8d6883f` e per nessun altro albero. Dichiarare same-SHA un commit
successivo senza rieseguire la batteria sarebbe esattamente l'errore che questa
sezione esiste per impedire.

## Seguito

Il checkpoint e' stato rieseguito, e l'esito superato e' in
`S9_CHECKPOINT_LEVEL2_2026-08-20_PASSED.md`. La revisione verificata **non** e'
`fdd4bd2` — che chiude solo il panic — ma `107b7b5`, perche' prima di
rimisurare sono stati corretti i tre difetti dell'harness descritti qui sotto,
e correggere lo strumento sposta la revisione misurata.

**Nulla di quanto misurato in questo documento e' stato modificato.** `8d6883f`
non ha superato, e resta cosi'.

## Nota di processo

`gates3.sh` esegue già `scripts/fuzz-smoke.sh` su tutti e tredici i target: il
passo «smoke 13/13» aggiunto separatamente lo **ripete**, per una quarantina di
minuti che non aggiungono nulla. Al prossimo checkpoint lo smoke va letto dalla
batteria invece di essere rieseguito.

Inoltre l'harness troncava a sei righe il log di ogni gate rosso, e i dettagli
del crash sono andati persi: sono stati recuperati riproducendo l'artefatto. Al
prossimo checkpoint il log di un gate rosso va conservato per intero.
