# S9 — secondo checkpoint di livello 2, superato

**Questo documento non governa la readiness di alcun componente né del sistema.**
Non è, e non va letto come, un'evidenza di release. `SYSTEM_RC_GATE.md` non è
stato modificato.

```
perimetro:                      checkpoint intermedio S9
revisione verificata:           effc4abe3f74ade083dbed72c94c286748809d9f
albero al momento della misura: pulito (0 file non committati)
esito:                          S9 checkpoint level 2 passed
release_authorized:             false
promozione di readiness:        nessuna, né di componente né di sistema
strumento:                      scripts/s9-checkpoint.sh
```

Precedenti, entrambi immutati:
`S9_CHECKPOINT_LEVEL2_2026-08-20.md` (non superato, `8d6883f`) e
`S9_CHECKPOINT_LEVEL2_2026-08-20_PASSED.md` (superato, `107b7b5`).

## Perché era dovuto

Terza tranche di driver dopo `107b7b5`: `driver-gpkg`, `driver-geoparquet`,
`driver-ipc` (design § 20).

## Esito misurato

**37 passi su 37.**

| Passo | Esito |
|---|---|
| `cargo fmt --all -- --check` | verde |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | verde |
| `cargo test --workspace --all-features` | verde, **31 binari** |
| 25 gate e sonde | verdi |
| catalogo FileGDB reale (`gdal-backend`, CLI vera) | verde |
| `fuzz-replay.sh` — replay deterministico | **32 425 input** su 13 target, nessun crash |
| `fuzz-smoke.sh` — smoke 13/13 | senza finding, **0 in quarantena** |
| copertura + gate delle esclusioni + soglia | verdi |

### Copertura

| Metrica | Valore | Soglia | Checkpoint precedente (`107b7b5`) |
|---|---:|---:|---:|
| righe | **84,45%** | 80% | 84,93% |
| regioni | 84,74% | — | 84,84% |
| funzioni | 78,84% | — | 78,89% |

Perimetro: 13 crate libreria; escluse `plenora-bench`, `plenora-fuzz`,
`plenora-io-cli`.

**La copertura di riga è scesa di 0,48 punti**, e vale la pena dire da dove
viene invece di limitarsi a constatare che è sopra soglia. Tre tranche hanno
sostituito `format!` a riga singola con messaggi curati su più righe, e hanno
estratto quattro funzioni (`mappa_campi_fisici`, `campo_fuori_range`,
`tipo_e_dimensioni`, `colonne_geometriche`): il denominatore cresce — le righe
totali passano da 31 612 a 31 839 — mentre i rami d'errore restano coperti
quanto prima.

Non è un peggioramento della verifica, ma **non è nemmeno un dettaglio da
ignorare**: se la tendenza continuasse per le quattro tranche restanti, la
soglia dell'80% arriverebbe a portata. Va guardata di nuovo al prossimo
checkpoint, e se scendesse ancora si aggiungono test — la soglia non si tocca.

### Stato S9 alla revisione verificata

```
costruttori d'errore legacy: 88 residui in 5 crate
migrati e a zero:            plenora-io-model, plenora-io-core, driver-common,
                             driver-geojson, driver-csv, driver-kml,
                             driver-gpkg, driver-geoparquet, driver-ipc
```

Nove componenti chiusi su quattordici. Registro dei fallback assurance: **115**,
invariato da tre tranche.

## Che cosa questo esito significa, e che cosa no

**Significa** che alla revisione `effc4ab` i nove componenti migrati non hanno
regressioni note rispetto ai gate dichiarati, che il replay di 32 425 input non
trova nulla, che il fuzzing non ha finding aperti su nessuno dei tredici target,
e che la copertura resta sopra la soglia.

**Non significa** che S9 sia chiuso — restano 88 occorrenze in cinque crate,
`plenora-io-cli` compresa — né che alcun componente sia pronto per una release.
S10, S11 e S12 non sono chiusi. La qualifica di release arriva solo sul
candidato finale.

## SHA verificato e SHA del commit documentale

Il commit che **pubblica** questo documento ha necessariamente un SHA diverso da
`effc4ab`, e **non eredita alcuna misura**: i numeri qui sopra valgono per
`effc4ab` e per nessun altro albero.

## Prossimo checkpoint

Dopo altre tre tranche di driver, cioè alla chiusura della tranche 12. Restano
`driver-filegdb` (22), `driver-shp` (22), `driver-dxf` (20), `driver-xls` (18),
e **per ultima** `plenora-io-cli` (6).
