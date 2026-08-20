# Change impact analysis — S9 tranche 4: `driver-geojson` redatto

Data: 2026-08-20. Sigla: **S9 / tranche 4**.
Baseline: `2e78791` (tranche 3, `driver-common`).
`plenora-io-error-v1` **invariato**.
Qualifica: **livello 1** — verificato, **non release-qualified** (design § 20).

## Il censimento manuale, prima della migrazione

Il registro autorevole dichiarava **5** usi legacy. Il censimento manuale ne ha
trovati molti di più, e sono quelli che contavano.

| Via | Firma | Chiamanti | Testo di dipendenza | Testo di payload |
|---|---|---:|---:|---:|
| `geometry::format_error` | `impl Into<String>` | 21 | **5** | 0 |
| `lib::err` | `impl Into<String>` | 13 | **7** | 0 |
| canale `Result<_, String>` in `geometry.rs` | ~10 funzioni | — | 0 | 0 |
| `finish_batch` | `Result<_, String>` | 2 | **1** | 0 |
| `DeError::custom` nei visitor | serde | 19 | 0 | **3** |

Fughe reali chiuse:

* **13 di testo di dipendenza** — `serde_json::Error` (9), l'errore di parsing
  di `geojson` (2), un `arrow` error in `finish_batch` (1), e il WKT generato
  in un messaggio (1);
* **3 di testo di payload** — `format!("GeoJSON top-level type '{value}' …")` e
  due varianti su `type` della Feature: `value` è una stringa **letta dal
  file**;
* **1 percorso di filesystem** — `PlenoraIoError::OutputExists(path.display()
  .to_string())`.

Il costruttore storico `OutputExists` ignorava già il proprio argomento, quindi
il percorso non finiva sul wire; ma la chiamata lo costruiva a ogni errore, e
lasciarla in piedi significava tenere in vita l'idea che quel percorso servisse
a qualcosa.

## Il difetto strutturale che la migrazione ha scoperto

Togliere il testo di dipendenza ha fatto fallire **cinque test**. Non erano
asserzioni troppo strette: erano il sintomo di un difetto reale.

I nostri errori uscivano dai visitor attraverso `DeError::custom`, cioè
**appiattiti in una stringa**. Il chiamante li rileggeva dal testo. Funzionava
per caso — il testo sopravviveva perché nessuno lo toglieva. Tolto il testo
della dipendenza, spariva anche il nostro, **e con lui il codice d'errore**: un
`LimitExceeded` sul tetto della cella geometrica arrivava al chiamante come un
`Format`.

Questo è peggio di una perdita di messaggio. È una **regressione semantica sul
wire**, del tipo che i consumatori usano per decidere (S9 ha ratificato che la
chiave è `(category, phase, code, retry)`, non il testo).

### La correzione

Non rimettere il testo. **Smettere di far passare un errore già strutturato
attraverso un canale che sa portare solo stringhe.**

`SchemaAccumulators` e `RowSink` — i due stati condivisi attraverso tutta la
catena di visitor — hanno ora un campo `errore: Option<PlenoraIoError>`. Il
gesto sta in un posto solo:

```rust
fn ferma_in<E: DeError>(slot: &mut Option<PlenoraIoError>, errore: PlenoraIoError) -> E {
    if slot.is_none() { *slot = Some(errore); }
    E::custom(INTERROTTO)
}
```

Il primo errore vince: quelli successivi sono conseguenze dell'interruzione,
non cause. `INTERROTTO` è un testo minimo — `serde` vuole qualcosa — scelto in
modo che, se per una via imprevista finisse in un errore pubblico, non dica una
cosa falsa.

Ai due punti di uscita, l'errore del canale ha la precedenza sul testo di
serde.

`ferma_in` prende lo **slot** e non lo stato intero perché `ValueSink` tiene in
prestito un builder di `RowSink`: due campi distinti si prestano insieme, la
struct intera no.

**I cinque test passano senza che una sola asserzione sia stata toccata.** Era
la prova che il difetto era nel codice e non nei test.

## Altri cambi

* `wkb_from_gj_value` restituisce `Result<()>` invece di
  `Result<(), String>`. L'ultimo passo convertiva un `PlenoraIoError` in testo
  per rispettare la firma: un errore già strutturato veniva appiattito proprio
  dove usciva dal modulo. È una funzione `pub` di un crate `publish = false`,
  usata da `plenora-fuzz`, che continua a compilare — `PlenoraIoError`
  implementa `Display`.
* `driver_common::saturating_u64` — helper condiviso per le conversioni
  `usize → u64` dei numeri strutturali. Serve a **ogni** driver che migra: senza,
  il registro dei fallback crescerebbe di una voce per driver per una
  conversione che non può fallire, e ogni voce somiglierebbe a una decisione
  presa in mancanza di meglio.

## Cosa esce dai messaggi

| Informazione | Esito |
|---|---|
| testo di `serde_json`, `geojson`, `arrow` | **eliminato**: testo di dipendenza |
| `type` letto dal documento (3 siti) | **eliminato**: testo di payload |
| percorso della destinazione esistente | **eliminato** |
| byte della geometria e tetto | **conservati** come conteggio e limite |
| tetto delle feature nell'inferenza | **conservato** come limite |
| codice d'errore dei limiti | **ripristinato**: era già perduto prima di questa tranche, ma il testo lo mascherava |

## Verifica (livello 1)

* `scripts/check_errori_redatti.py`: **148 → 133** in dieci crate;
  `plenora-io-model`, `plenora-io-core`, `driver-common`, `driver-geojson`
  migrati e a **zero**;
* sonde del censimento: 7/7;
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: pulito;
* `cargo test --workspace --all-features`: verde su 31 binari;
* gate specifici: `check_quarantena_fuzz`, `check_prevalidazione_decoder`,
  `check_public_identity`, `check_release_contract --historical`,
  `check_assurance_fallbacks` — tutti verdi;
* smoke dei soli target coinvolti: `geojson_reader`, `wkt_parse`.

### Registro dei fallback

`driver-geojson` passa da 2 a 4 (totale 113 → 115), con la ragione scritta: i
due `errore.take().unwrap_or_else(…)` ai punti di uscita sono il caso in cui il
canale laterale è vuoto perché a fallire è stato serde sul JSON malformato — un
caso legittimo, e il default è il messaggio giusto per quel caso.

I tre `unwrap_or(u64::MAX)` che le conversioni avrebbero richiesto **non** sono
stati registrati: sono passati da `driver_common::saturating_u64`. Il registro
non è stato alzato per far passare il gate, ed è stato alzato dove il fallback
c'è davvero.

## Prossimo passo

Tranche 5: un solo driver. Nel registro autorevole restano `driver-filegdb`
(22), `driver-shp` (22), `driver-dxf` (20), `driver-xls` (18), `driver-kml`
(12), `driver-gpkg` (10), `driver-csv` (8), `driver-geoparquet` (8),
`driver-ipc` (7), più `plenora-io-cli` (6).

Il checkpoint di livello 2 — batteria completa, copertura same-SHA, smoke 13/13
— è dovuto **dopo tre driver**, cioè alla chiusura della tranche 6.
