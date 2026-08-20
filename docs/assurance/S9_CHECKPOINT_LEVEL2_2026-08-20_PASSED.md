# S9 — checkpoint di livello 2, superato

**Questo documento non governa la readiness di alcun componente né del sistema.**
Non è, e non va letto come, un'evidenza di release. `SYSTEM_RC_GATE.md` non è
stato modificato.

```
perimetro:                      checkpoint intermedio S9
revisione verificata:           107b7b5399d79afeddec1fe94a3dbc90cdfef0ee
albero al momento della misura: pulito (0 file non committati)
esito:                          S9 checkpoint level 2 passed
release_authorized:             false
promozione di readiness:        nessuna, né di componente né di sistema
strumento:                      scripts/s9-checkpoint.sh
```

Questa evidenza **non sostituisce** `S9_CHECKPOINT_LEVEL2_2026-08-20.md`, che
registra il tentativo **non superato** su `8d6883f` e resta immutato. Le due
vanno lette insieme: la prima corsa ha trovato un difetto reale, e l'averlo
trovato è il motivo per cui la seconda vale qualcosa.

## Perché la revisione verificata è `107b7b5` e non `fdd4bd2`

`fdd4bd2` chiude il panic trovato dal checkpoint fallito. Ma prima di
rimisurare sono stati corretti tre difetti dell'harness (`107b7b5`, INFRA-2), e
correggere lo strumento **sposta la revisione misurata**: il verde appartiene
all'albero su cui la misura è girata, non a quello che si voleva misurare
all'inizio.

**A `fdd4bd2` non va attribuito questo esito.** Resta il commit che chiude il
panic, e nient'altro.

## Esito misurato

**37 passi su 37.**

| Passo | Esito |
|---|---|
| `cargo fmt --all -- --check` | verde |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | verde |
| `cargo test --workspace --all-features` | verde, **31 binari** |
| 25 gate e sonde (censimento, quarantena, prevalidazione, identità, contratto di release, pin, fork, budget, permit, fallback) | verdi |
| catalogo FileGDB reale (`gdal-backend`, CLI vera) | verde |
| `fuzz-replay.sh` — replay deterministico | **31 871 input** su 13 target, nessun crash |
| `fuzz-smoke.sh` — smoke 13/13 | senza finding su 13 target, **0 in quarantena** |
| copertura + gate delle esclusioni + soglia | verdi |

### Copertura

| Metrica | Valore | Soglia |
|---|---:|---:|
| righe | **84,93%** | 80% |
| regioni | 84,84% | — |
| funzioni | 78,89% | — |

Perimetro: 13 crate libreria; escluse `plenora-bench`, `plenora-fuzz`,
`plenora-io-cli`. Il gate delle esclusioni ha verificato che il report osservato
rispetti lo stesso perimetro della soglia — misurato **dopo** la generazione del
report, che è una delle tre correzioni di INFRA-2.

### Stato S9 alla revisione verificata

```
costruttori d'errore legacy: 113 residui in 8 crate
migrati e a zero:            plenora-io-model, plenora-io-core, driver-common,
                             driver-geojson, driver-csv, driver-kml
```

Registro dei fallback assurance: **115**, invariato.

## Che cosa questo esito significa, e che cosa no

**Significa** che alla revisione `107b7b5` le sei componenti migrate non hanno
regressioni note rispetto ai gate dichiarati, che il fuzzing non trova finding
aperti su nessuno dei tredici target, e che la copertura resta sopra la soglia.

**Non significa** che S9 sia chiuso — restano 113 occorrenze in otto crate — né
che alcun componente sia pronto per una release. S10, S11 e S12 non sono
chiusi. La qualifica di release arriva solo sul candidato finale, e non è una
proprietà che un passo intermedio possa conferire.

## SHA verificato e SHA del commit documentale

Il commit che **pubblica** questo documento ha necessariamente un SHA diverso da
`107b7b5`, e **non eredita alcuna misura**: i numeri qui sopra valgono per
`107b7b5` e per nessun altro albero.

Dichiarare same-SHA un commit successivo senza rieseguire
`scripts/s9-checkpoint.sh` sarebbe l'errore che questa sezione esiste per
impedire — ed è lo stesso errore, in forma più sottile, dell'attribuire questo
verde a `fdd4bd2`.

## Prossimo checkpoint

Dopo altre tre tranche di driver, cioè alla chiusura della tranche 9 (design
§ 20). Restano da migrare `driver-filegdb` (22), `driver-shp` (22),
`driver-dxf` (20), `driver-xls` (18), `driver-gpkg` (10), `driver-geoparquet`
(8), `driver-ipc` (7) e `plenora-io-cli` (6).
