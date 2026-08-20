# Change impact analysis — S9 tranche 6: `driver-kml` redatto

Data: 2026-08-20. Sigla: **S9 / tranche 6**.
Baseline: `b7333c8` (tranche 5, `driver-csv`).
`plenora-io-error-v1` **invariato**.

## Censimento a due classi

| Via | Forma | Chiamanti / occorrenze |
|---|---|---:|
| `err` | `impl Into<String>` | **77** |
| `err(format!("…: {error}"))` | cancellazione di struttura | 14 |
| `err(format!(…))` con valore letto dal file | fuga di payload | **2** |
| `err(format!(…))` con un tetto nostro | numero strutturale | 2 |
| `Result<_, String>` | — | 0 |
| `DeError::custom` | — | 0 |

Usi legacy diretti: **12**.

Come in `driver-csv`, la cancellazione di struttura qui **non perdeva il tipo
d'errore**: `err` riclassifica subito come `Format`. Il difetto di
`driver-geojson` — dove l'errore usciva dal dominio e veniva riclassificato a
valle sul testo — non si ripresenta.

## Le due fughe di payload

Sono la scoperta di questa tranche, e nessuna delle due sarebbe emersa
guardando solo le firme.

**1. `err(format!("entità XML sconosciuta: &{name};"))`**

`name` è il nome dell'entità XML **letto dal file**. Fino a qui veniva
deliberatamente escapato in ASCII per essere messo nel messaggio — un modo per
renderlo *leggibile*, che non lo rendeva *lecito*.

**2. `err(format!("contenuto testuale KML non valido: {event:?}"))`**

`event` è un `quick_xml::events::Event`, e il suo `Debug` **contiene i byte
grezzi dell'elemento**. Era il payload, per intero, dentro un messaggio
pubblico. Il `{:?}` lo nascondeva: nella riga si legge il nome di una variabile,
non il contenuto di un file.

Entrambe chiuse: il messaggio dice ora cosa è successo, non cosa c'era scritto.

## Correzione al registro dei fallback

`FALLBACK_REGISTER.md` descriveva i due `escape_ascii` di `driver-kml` come
«rendono leggibili con escape ASCII soltanto i token XML invalidi **nei
messaggi d'errore**». Non è così: costruiscono il **testo estratto** dal KML
quando non è UTF-8 valido — sono nel percorso dei dati, non degli errori.

La descrizione è stata corretta. Un registro che descrive una decisione diversa
da quella presa vale meno di nessun registro: è la stessa classe di difetto del
commento che dice quello che si voleva fare invece di quello che il codice fa.

## Cosa esce dai messaggi

| Informazione | Esito |
|---|---|
| testo di `quick_xml`, `arrow`, errori UTF-8 (14 siti) | **eliminato**: testo di dipendenza |
| nome dell'entità XML sconosciuta | **eliminato**: letto dal file |
| `Debug` dell'`Event` di quick_xml | **eliminato**: conteneva i byte grezzi dell'elemento |
| `MAX_XML_DEPTH`, `max_rows`, tetto dello spool | **conservati** come limiti |
| byte scritti nello spool | **conservati** come conteggio |

## Verifica

* `scripts/check_errori_redatti.py`: **125 → 113** in otto crate; sei
  componenti migrati e a **zero**: `plenora-io-model`, `plenora-io-core`,
  `driver-common`, `driver-geojson`, `driver-csv`, `driver-kml`;
* sonde del censimento: 7/7;
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: pulito;
* `cargo test --workspace --all-features`: verde su 31 binari;
* gate specifici: `check_quarantena_fuzz`, `check_prevalidazione_decoder`,
  `check_public_identity` verdi;
* `check_assurance_fallbacks`: **totale invariato a 115**.

### Checkpoint di livello 2

Questa è la terza tranche di driver dopo la ratifica del 2026-08-20, quindi il
checkpoint completo è **dovuto** (design § 20): batteria completa dei gate,
copertura misurata **same-SHA**, smoke **13/13**.

L'esito è registrato in `SYSTEM_RC_GATE.md` con il SHA che qualifica. Fino ad
allora questo commit resta **verificato ma non release-qualified**, come i due
che lo precedono.

## Prossimo passo

Tranche 7: un solo driver. Restano `driver-filegdb` (22), `driver-shp` (22),
`driver-dxf` (20), `driver-xls` (18), `driver-gpkg` (10), `driver-geoparquet`
(8), `driver-ipc` (7), più `plenora-io-cli` (6).
