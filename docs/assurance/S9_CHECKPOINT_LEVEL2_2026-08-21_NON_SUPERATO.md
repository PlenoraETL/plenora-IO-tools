# S9 — checkpoint di livello 2 su `71adf70`, non superato

**Questo documento non governa la readiness di alcun componente né del sistema.**
`SYSTEM_RC_GATE.md` non è stato modificato.

```
perimetro:                      checkpoint intermedio S9
revisione verificata:           71adf70afd1fb19c899bca7da54822690ecff394
albero al momento della misura: pulito (0 file non committati)
esito:                          NON SUPERATO
copertura:                      NON DISPONIBILE — dati stale rifiutati
release_authorized:             false
promozione di readiness:        nessuna, né di componente né di sistema
```

## Il finding

`driver-filegdb` **non compilava senza la feature `gdal-backend`**. La
migrazione della tranche 11 aveva lasciato `&PublicMessage::Curated(...)` non
qualificato in due punti del ramo stub, dove il tipo non è importato.

Chiuso in `81b644a`.

### Perché nessuna verifica di livello 1 l'ha preso

Tutte le mie verifiche giravano con `--all-features`, che abilita
`gdal-backend`: il ramo stub non veniva compilato da nessuna. A trovarlo è stata
la misura di copertura, che gira senza feature — cioè un passo che serve ad
altro.

In tranche 11 avevo eseguito `cargo clippy -p driver-filegdb --all-targets`
proprio per coprire quel percorso, ma il comando finiva in una pipe verso `grep`
e ho letto l'assenza di righe come un verde. È l'errore che la regola del
2026-08-17 vieta — l'esito non si legge da una pipe — ripetuto con `| grep`
invece che con `| tail`.

CI lo avrebbe preso (`ci.yml:161` esegue `cargo test --workspace --all-targets`
senza feature). Il checkpoint no, e ora sì: `test_default` e `clippy_default`
sono passi dichiarati.

## La copertura di questa corsa non esiste

**Il numero `75,98%` che questa corsa ha stampato non è una misura di
`71adf70`.** Non va conservato, citato o confrontato.

`coverage_misura` è fallito. `coverage_export`, `check_coverage_exclusions` e
`coverage_soglia` sono comunque girati, e sono andati **verdi**, leggendo i dati
di profiling lasciati nel volume `target` dalla corsa precedente — quella su
`effc4ab`.

Il segnale che l'ha rivelato: 42 delle righe dichiarate «scoperte» erano **il
test che avevo scritto in INFRA-3**, che in quel report non poteva esistere.

Quindi, per questa revisione:

| | |
|---|---|
| copertura totale | **non disponibile** |
| gate delle esclusioni | **non valido** — ha letto un report di un altro albero |
| soglia dell'80% | **non valida** — ha detto verde su una misura non avvenuta |
| diagnostica differenziale | **non valida** — 147 siti non classificabili |

**Nessuno dei 147 siti differenziali è stato classificato**, e non lo sarà: la
classificazione si farà sui numeri della prima corsa in cui la catena della
copertura sia integra.

## I difetti dell'harness che questa corsa ha rivelato

| Difetto | Chiuso in |
|---|---|
| `passo()` stampava «ROSSO (exit 0)»: un `if` con condizione falsa e senza `else` restituisce 0 | `81b644a` |
| il set di feature predefinito non era compilato da nessun passo | `81b644a` (`test_default`, `clippy_default`) |
| i passi della copertura giravano dopo una misura fallita, su dati stale | `afade90` (INFRA-5) |
| i passi della copertura dipendevano dalla **misura**, non dal passo **precedente** | `2026-08-21` (INFRA-6) |
| `lcov.info` non veniva eliminato prima dell'export | INFRA-6 |
| la soglia leggeva il **profdata**, non il report che gli altri gate leggono | INFRA-6 |
| un export con esito zero e file vuoto non era un caso previsto | INFRA-6 |

Venti sonde in `scripts/test_s9_checkpoint.sh` coprono ora tutti questi
comportamenti, comprese le negative: un comando che esce con 17 deve essere
riportato `ROSSO (exit 17)`; un passo saltato conta fra i falliti e non fra i
verdi; la catena nomina chi l'ha spezzata.

## Le evidenze passed precedenti restano valide

Il bug di `passo()` alterava **solo il numero stampato**, non l'esito globale:
sul fallimento `falliti+=(...)` esegue comunque e `verdi` non viene
incrementato, e il blocco finale decide su `${#falliti[@]}`.

Prova empirica: questa stessa corsa, con il bug presente, ha stampato
`passi: 37/38` ed `esito: NON SUPERATO`.

`S9_CHECKPOINT_LEVEL2_2026-08-20_PASSED.md` (`107b7b5`) e
`S9_CHECKPOINT_LEVEL2_2026-08-21_PASSED.md` (`effc4ab`) riportavano entrambe
37/37 senza falliti, con `coverage_misura` verde nei rispettivi log. Restano
valide e **non ricevono addendum**.

## Prossimo passo

Il checkpoint va rieseguito sulla revisione che contiene INFRA-6 — non su
`71adf70` e non su `afade90`. La baseline della diagnostica resta
`effc4abe3f74ade083dbed72c94c286748809d9f`, l'ultima revisione **verificata**.
