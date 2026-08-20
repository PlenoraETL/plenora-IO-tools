# Change impact analysis — S9 tranche 3: `driver-common` redatto

Data: 2026-08-20. Sigla: **S9 / tranche 3**.
Baseline: `893a6db` (tranche 2, `plenora-io-core`).
`plenora-io-error-v1` **invariato**.

## Perché questa tranche conta più dei suoi numeri

Dieci usi legacy su 148: numericamente è la tranche più piccola. È però la
prima che prova il **metodo** che verrà ripetuto dieci volte sui driver, e il
metodo include un passo che la tranche 2 ha dimostrato necessario e che il
gate non sa fare: il censimento manuale degli helper.

## Il censimento manuale, prima della migrazione

Cercati in `crates/driver-common/src`: firme con `impl Into<String>`, `String`,
`&str` non `'static`, `Cow<str>` che finiscano in un costruttore di
`PlenoraIoError`.

| Helper | Firma | Chiamanti | Esito |
|---|---|---|---|
| `wkt_lossless::error` | `impl Into<String>` | **28** | migrato a `&'static str` |
| `prevalida_arrow::errore` | `motivo: &'static str` | 12 | già statico; solo il costruttore cambia |
| `lib.rs::append_typed` | `S: FnOnce(&'a T) -> Option<Cow<'a, str>>` | — | **non è una via d'errore**: la `Cow` è il valore da accodare, non un messaggio |

`wkt_lossless::error` è la scoperta che giustifica il passo. Ventotto chiamanti,
zero `PlenoraIoError::` al loro interno, quindi **zero visibilità per il gate** —
e fra loro **tre con testo di dipendenza**:

* `error(format!("sintassi non valida: {message}"))` — `message` è l'errore di
  parsing della crate `wkt`;
* due `error(format!("serializzazione fallita: {format_error}"))` —
  `std::fmt::Error`.

Più uno con testo derivato dal payload:
`error(format!("POLYGON atteso dalla crate wkt, ricevuto {testo:?}"))`, dove
`testo` è il WKT generato dalla geometria in ingresso.

**Il censimento dei costruttori dichiarava dieci siti; le vie aperte al testo
libero erano dieci più ventotto.**

## Cosa cambia

| File | usi legacy diretti | di cui `format!` | di cui valori interpolati | di cui testo di dipendenza |
|---|---|---|---|---|
| `lib.rs` | 7 | 2 | 2 | 0 |
| `wkt_lossless.rs` | 2 | 2 | 1 | 0 |
| `prevalida_arrow.rs` | 1 | 0 | 0 | 0 |
| **totale diretti** | **10** | **4** | **3** | **0** |
| *chiamanti di `wkt_lossless::error`* | *28* | *8* | *3* | **3** |

Il testo di dipendenza in `driver-common` era **zero fra i siti diretti** e
**tre fra i chiamanti dell'helper**: è la ragione per cui le due colonne vanno
lette separate, e la ragione per cui il passo manuale non è cerimoniale.

### `wkt_lossless::error`

Firma da `impl Into<String>` a `&'static str`. Il prefisso `WKT:` **resta** —
non passa più da `format!`, è il primo membro di una
`PublicMessage::CuratedPair(PREFISSO, message)`, entrambi `&'static str`. I
ventisei chiamanti letterali restano `error("…")` senza avvolgere niente.

*(Nota di percorso: la prima stesura aveva buttato via il prefisso lasciando un
commento che diceva il contrario. Il commento è stato corretto insieme al
codice — un commento che descrive quello che si voleva fare invece di quello
che il codice fa è peggio di nessun commento.)*

### `ColType::nome` e `classe_arrow`

Due nuovi vocabolari statici, nella stessa famiglia dell'errata S9.1:

* `ColType::nome()` sostituisce `{column_type:?}` in `incompatible_value`;
* `classe_arrow(&DataType)` sostituisce `{:?}` sul tipo Arrow in
  `arrow_cell_to_json`. `driver-common` **non dipende da `plenora-io-core`** e
  non vede `ArrowTypeClass`: è lo stesso vocabolario, dichiarato dove serve.
  Non è una tassonomia completa dei tipi Arrow — è l'insieme delle classi che
  questo bordo distingue, e il ramo `altro` copre per costruzione il resto.

Il `Debug` di `DataType` non è solo instabile: per i tipi annidati **contiene i
nomi dei campi**, che vengono dal file. Era un canale di uscita per il payload,
non solo una dipendenza da un formato non promesso.

## Cosa esce dai messaggi

| Informazione | Prima | Ora | Esito |
|---|---|---|---|
| errore di sintassi della crate `wkt` | interpolato | assente | **eliminato**: testo di dipendenza, ed è il caso da cui INV-10 è partito. La posizione dell'errore non è recuperabile senza farlo uscire, e non la si inventa |
| `std::fmt::Error` in serializzazione | interpolato | assente | **eliminato**: testo di dipendenza |
| WKT generato quando non inizia per `POLYGON` | interpolato | assente | **eliminato**: derivato dal payload |
| dimensionalità attesa dalla geometria | interpolata | assente | **perduta**: è la dimensionalità della geometria, che il chiamante ha in mano. Resta quella **osservata**, che è l'informazione che lui non ha |
| tipo inferito della colonna (`ColType`) | `{:?}` | `nome()` | invariato nella sostanza |
| tipo Arrow non convertibile | `{:?}` su `DataType` | classe statica | ristretto: la classe invece del tipo esatto |
| byte della cella WKT e tetto | interpolati | `CuratedBetween` con conteggio e limite | invariato: sono un conteggio e un tetto, cioè numeri strutturali |

## Impatto sui consumatori

Il testo di `message` cambia nei siti migrati — rottura già ratificata
(decisione 2 di S9). Lo schema di `plenora-io-error-v1` non cambia, e categoria,
fase, effetto remoto, retry e `IoErrorCode` sono preservati sito per sito.

## Verifica

* `scripts/check_errori_redatti.py`: **148 → 138** in undici crate;
  `plenora-io-model`, `plenora-io-core`, `driver-common` migrati e a **zero**;
* sonde del censimento: 7/7;
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: pulito;
* `cargo test --workspace --all-features`: verde su 31 binari;
* batteria completa dei gate: **31/31** — inclusi `fuzz-smoke` 13/13, il
  catalogo FileGDB reale e `check_coverage_exclusions.py --lcov lcov.info`
  eseguito sul report vero;
* copertura: **85,59% di righe** contro la soglia dell'80% (84,95% regioni,
  78,95% funzioni), misurata su questo albero.

## Qualifica di questo commit

Questa tranche è verificata con la **batteria completa** — è l'ultima a esserlo
per obbligo. Dalla tranche 4 vale la validazione a due livelli ratificata il
2026-08-20 (design § 20): livello 1 per ogni crate, livello 2 — batteria
completa, copertura, smoke 13/13 — **ogni tre driver e alla chiusura di S9**.

I commit di livello 1 sono **verificati ma non release-qualified**. La misura di
copertura che sostiene la qualifica deve essere **same-SHA**: quella riportata
qui dimostra che la soglia è raggiungibile su questo albero, e vale per questo
commit soltanto.

## Prossimo passo

Tranche 4: il **primo** dei dieci driver. Il registro autorevole li ordina
`driver-filegdb` (22), `driver-shp` (22), `driver-dxf` (20), `driver-xls` (18),
`driver-kml` (12), `driver-gpkg` (10), `driver-csv` (8), `driver-geoparquet`
(8), `driver-ipc` (7), `driver-geojson` (5), più `plenora-io-cli` (6).

Per ognuno, **prima** della migrazione: il censimento manuale degli helper. In
`driver-common` ha trovato ventotto vie che il gate non vedeva, e tre di esse
facevano uscire testo di dipendenza.
