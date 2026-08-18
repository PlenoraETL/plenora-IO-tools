# Change impact analysis — schema dichiarativo delle `format_options` (L0.7)

Data: 2026-08-18. Sigla: **S6**.
Baseline: `efada48`.
Design ratificato: [`DESIGN-S6-format-options-schema.md`](../DESIGN-S6-format-options-schema.md).

## Problema

Le `format_options` erano una `BTreeMap<String, String>` che ogni driver
interpretava per conto proprio, senza che nulla dicesse quali chiavi esistono,
in che fase valgono e quali valori accettano. Tre conseguenze, tutte misurate
sul codice prima di toccarlo:

1. **Una chiave sconosciuta veniva ignorata in silenzio.** `wkt_colunm` non
   produceva errore: produceva un dataset senza geometria. Chi aveva sbagliato
   a scrivere scopriva il refuso a valle, o non lo scopriva.
2. **Un valore sconosciuto degradava a un default.**
   `compression=zstsd` scriveva un file **snappy**, valido, e non lo diceva a
   nessuno. `geometry_encoding=wkb` produceva WKT.
3. **Non esisteva un modo di sapere cosa un driver accetta** se non leggendone
   il sorgente: né la CLI né un SDK potevano elencare le opzioni.

## Cosa cambia

### Lo schema è un tipo, e il legame è strutturale

`plenora-io-model::format_options` porta i tipi (`SchemaOpzioniFormato`,
`OpzioneFormato`, `FaseOpzione`, `ValoreAmmesso`); ogni driver dichiara il
proprio `SCHEMA_OPZIONI` accanto al proprio `static DESCRIPTOR`; e
`FormatDescriptor` ha un campo `format_options` **obbligatorio e non
`Option`**.

La conseguenza è la ragione della scelta: aggiungere un driver senza dichiarare
uno schema non compila. Il test `every_driver_has_a_schema_for_options` resta,
ma verifica una proprietà già garantita dal tipo — che è la forma in cui un
test serve, a sorvegliare e non a reggere.

I sei driver che non interpretano nulla dichiarano `SchemaOpzioniFormato::VUOTO`.
L'elenco vuoto è un'**affermazione** — qualunque chiave è sconosciuta — non
un'omissione, ed è quella distinzione che il campo obbligatorio rende
osservabile.

### La validazione è imposta dalla firma, non dalla convenzione

FZ-0 aveva appena mostrato il costo della convenzione: una prevalidazione
corretta che nulla lega alla chiamata che protegge, e un gate apposta perché un
percorso nuovo non la salti. Qui il legame è nella firma:

| Prima | Dopo |
|---|---|
| `preflight_source(source, &mut opts)` | `preflight_source(descriptor, source, &mut opts)` |
| `validate_write(descriptor, plan, max_columns)` | `validate_write(descriptor, plan, max_columns, format_options)` |

Un driver che non consegna il descrittore o le opzioni non compila. Tutti e
dieci passano da entrambe.

La validazione di lettura avviene **prima** di toccare il filesystem: una
chiave sbagliata è un errore di configurazione, e diventerebbe un errore di I/O
solo se qualcuno la scoprisse dopo aver aperto il file.

### I quattro fallback silenziosi sono chiusi

| Driver | Fallback | Prima | Dopo |
|---|---|---|---|
| geoparquet | `compression` | `_ => SNAPPY` | nessun ramo `_`; valore ignoto → errore tipizzato |
| csv, xls | `geometry_encoding` | `matches!(…, Some("xy"))`: tutto il resto era WKT | `wkt` e `xy` sono due casi; ogni altro valore è un errore |
| csv | `delimiter` | primo byte di qualunque stringa, `unwrap_or(b',')` | esattamente un carattere ASCII, o errore |
| geoparquet | `bbox_legacy_by_name` | lista scritta a mano, `"false"` e `"pippo"` indistinguibili | forma booleana condivisa dello schema |

Il caso `delimiter` è quello che il registro `check_assurance_fallbacks`
contava: `driver-csv` scende da 3 a 2 occorrenze. È l'unico verso in cui quel
contatore deve muoversi da solo.

### Il registro è derivato

`DriverRegistry::format_options()` compone il registro dall'elenco dei driver
registrati. Non esiste una tabella da tenere allineata: non c'è una riga da
dimenticare quando si aggiunge un driver, né una che sopravviva a uno rimosso.

S6 **non** introduce il comando `options` della CLI: L0.7 lo abilita, la facade
lo esporrà.

### Lo schema entra nel catalogo pubblico

`FormatDescriptor` è `Serialize`, quindi lo schema compare in
`plenora-io catalog`. Gli identificatori interni restano italiani, la forma
serializzata no: `key`, `phase`, `value`, `default`, `description`, con
`phase ∈ {read, write, both}` e `value` fra `text`, `enum`, `boolean`, `char`,
`integer {min, max}` — lo stesso snake_case inglese del resto del catalogo.

Aggiungere un campo al catalogo è un cambio di schema: **`descriptor_version`
aumenta di uno per tutti e dieci i driver** (filegdb 9 → 10, geoparquet 6 → 7,
gli altri otto 7 → 8).

## Verifica

### I sei test trasversali

Tutti e sei obbligatori in ratifica, in `plenora-io-cli/src/conformance_tests.rs`:

| Test | Cosa prova |
|---|---|
| `every_driver_has_a_schema_for_options` | il registro copre l'elenco dei driver; nessuna chiave vuota o duplicata; ogni default dichiarato rispetta la propria forma |
| `unknown_option_key_produces_typed_error_not_silent_ignore` | `open` rifiuta con `InvalidConfiguration`, nomina la chiave rifiutata **e** elenca quelle accettate |
| `unknown_compression_value_produces_typed_error_not_snappy_default` | `compression=zstsd` fallisce, nomina `zstsd` e `zstd`, e **non lascia il file** |
| `una_chiave_di_scrittura_passata_in_lettura_e_rifiutata` | la fase è parte dello schema; l'errore nomina la fase |
| `ogni_valore_ammesso_dallo_schema_e_accettato_dal_driver` | **tredici scritture reali**, una per valore enumerato di scrittura |
| `il_default_dichiarato_e_quello_applicato` | omettere l'opzione e dichiararla al default producono file **identici byte a byte** |

Gli ultimi due sono i più importanti perché non erano nell'elenco di L0.7:
senza di essi lo schema diventerebbe documentazione, cioè una seconda verità
che diverge dal codice in silenzio — il difetto che S6 esiste per chiudere.

`ogni_valore_ammesso…` scrive davvero perché è lì che lo schema può promettere
più di quanto il driver mantenga: il driver traduce il valore in una scelta — un
codec, un encoding, una forma di pubblicazione — e la traduzione può non avere
il caso. `lz4` per GeoParquet e `shapefile_directory_dataset` per Shapefile sono
esercitati da una scrittura completa, non da un `assert` sullo schema.

`il_default_dichiarato…` confronta i byte perché è l'unico modo di distinguere
«il driver applica il default dichiarato» da «il driver applica un default suo
che per caso oggi coincide».

A questi si aggiungono **11 test di grammatica** in `format_options.rs`: per
ogni forma un valore accettato e uno rifiutato, incluse le varianti che la
ratifica esclude (`on`, stringa vuota, `1.0`, maiuscole, `+8`).

### Due casi trattati esplicitamente invece che saltati

* **`publish_mode` e la destinazione.** Il valore deve concordare con il
  suffisso del sink — un vincolo che lega opzione e destinazione, che nessuna
  grammatica su un singolo valore può esprimere. Il test porta una tabella
  esplicita `valore → suffisso` invece di saltare il caso: se domani un'altra
  opzione avesse lo stesso accoppiamento, si aggiunge una riga e resta visibile
  che l'accoppiamento esiste.
* **FileGDB senza il tier GDB.** Rifiuta per driver indisponibile **prima** di
  guardare le opzioni. Non è un'ignoranza silenziosa: è un rifiuto che precede,
  e pretendere lì la categoria dello schema misurerebbe l'ordine dei controlli
  invece del controllo. Con la feature attiva (gate «test FileGDB feature-on»)
  il driver rientra nel giro.

### Copertura non rivendicata

Le enumerazioni di **lettura** — oggi solo `row_diagnostics.key_policy` —
pretendono un dataset con una chiave già presente, cioè un fixture che vive nel
driver. `emit` e `redact` sono esercitate entrambe dai test di `driver-shp`. Il
test trasversale non le duplica e **non finge di coprirle**: lo dice nel proprio
commento.

## Quattro divergenze dal design ratificato

Il design era stato ratificato con un censimento incompleto. Tutte e quattro
risolte **verso il codice**, e registrate nell'errata del design:

1. `publish_mode` — i valori reali sono `shapefile_directory_dataset` e
   `loose_shapefile_set`, non `directory_dataset`/`loose_set`. Uno schema con i
   valori del documento avrebbe rifiutato ciò che il driver accetta.
2. Tre chiavi mancanti: `row_diagnostics.key_field`, `row_diagnostics.key_policy`,
   `row_diagnostics.examples_limit`.
3. Una **forma in più**: `ValoreAmmesso::Intero { min, max }`, che la grammatica
   ratificata non aveva. Segue la stessa regola delle altre — una sola grafia,
   sole cifre ASCII: `+8` è rifiutato benché `u64::from_str` lo accetti.
4. **Categoria d'errore.** Il design indicava `Unsupported`; l'implementazione
   usa `InvalidConfiguration` / fase `Validate` / retry `Never`. `Unsupported` è
   una risposta *sul prodotto* e davanti a essa un chiamante automatico cambia
   driver; qui la risposta è *sull'input*, e la reazione corretta è correggere
   la richiesta. È anche la categoria che `driver-shp` già produceva per gli
   stessi controlli.

Il punto 4 cambia la categoria osservabile di `publish_mode` e della
compressione da `Unsupported` a `InvalidConfiguration`.

## Perimetro e rischi residui

Toccati: `plenora-io-model` (modulo nuovo `format_options`), `plenora-io-core`
(`descriptor`, `capabilities`, `driver`, `registry`), i dieci driver,
`plenora-io-cli/src/conformance_tests.rs`, due gate
(`check_assurance_fallbacks.sh`, `check_wkb_limits_defaults.py`), il design.

Non toccati: formati su disco, comandi della CLI, contratti di lettura e
scrittura oltre alla categoria d'errore del punto 4.

Residui dichiarati:

* **La copertura delle enumerazioni di lettura non è trasversale** (sopra). Se
  un driver aggiungesse domani un'enumerazione di lettura, il test trasversale
  non se ne accorgerebbe: la coprirebbero solo i suoi test.
* **Vincoli fra chiavi restano nel driver.** `wkt_column` esclusivo con
  `x_column`/`y_column`, `key_policy` che richiede `key_field`, `publish_mode`
  legato al suffisso: lo schema descrive valori, non relazioni. Nessuno di
  questi è degradato in silenzio — sono tutti errori tipizzati — ma non sono
  dichiarati.
* **Il censimento di `check_wkb_limits_defaults` è per `path:riga`.** Le due
  occorrenze legittime hanno cambiato riga per effetto delle dichiarazioni di
  schema (gpkg 1673 → 1681, shp 2460 → 2513): stesse occorrenze, stesso codice.
  Il gate va riallineato a ogni spostamento di riga, ed è fragile per
  costruzione — fuori dal perimetro di S6.
* Il residuo `PageHeader.uncompressed_page_size` (FZ-0.1) era aperto e
  separato quando S6 e' stato scritto: S6 non lo tocca. **Chiuso da FZ-0.2 il
  2026-08-18.**
