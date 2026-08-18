# Design S6 — schema dichiarativo `format_options` (L0.7)

Stato: **proposta di design, da ratificare prima dell'implementazione**.
Baseline: `4c96f94`. Dipendenza dichiarata: S4 (chiuso).

## Problema

`format_options` è oggi una `BTreeMap<String, String>` che ogni driver
interroga per conto proprio. Ne seguono due difetti, entrambi silenziosi.

**Una chiave sconosciuta non esiste.** Nessuno la legge, nessuno la rifiuta.
`--opt wkt_colunm=geom` — con il refuso — produce una lettura senza geometria,
non un errore.

**Un valore invalido degrada al default.** Il caso peggiore è
`driver-geoparquet`:

```rust
match opts.format_options.get("compression").map(String::as_str) {
    Some("zstd") => Compression::ZSTD(ZstdLevel::default()),
    ...
    _ => Compression::SNAPPY,
}
```

`--opt compression=zstdd` scrive un file **snappy** e non lo dice a nessuno.
Chi lo ha chiesto crede di avere zstd finché non misura.

### Censimento completo

| Driver | Chiave | Fase | Forma del valore | Comportamento oggi |
|---|---|---|---|---|
| csv | `wkt_column` | lettura | nome di colonna | libero, validato dopo contro le intestazioni |
| csv | `x_column`, `y_column` | lettura | nome di colonna | come sopra |
| csv | `delimiter` | lettura, scrittura | un carattere | **primo byte** del valore, default `,` — `delimiter=` dà `,`, `delimiter=;;` dà `;` |
| csv | `geometry_encoding` | scrittura | `xy` | qualunque cosa diversa da `xy` significa WKT — **degrada** |
| xls | `sheet` | lettura | nome di foglio | libero, validato dopo |
| xls | `wkt_column`, `x_column`, `y_column` | lettura | nome di colonna | libero |
| xls | `geometry_encoding` | scrittura | `xy` | **degrada** come csv |
| geoparquet | `compression` | scrittura | `zstd`\|`gzip`\|`brotli`\|`lz4`\|`none`\|`uncompressed` | **degrada** a snappy |
| geoparquet | `bbox_legacy_by_name` | lettura | booleano | `1`\|`true`\|`yes` è vero, **tutto il resto è falso** |
| shp | `publish_mode` | scrittura | `directory_dataset`\|`loose_set` | **già tipizzato**: valore ignoto → `Unsupported` |

`shp` è il modello: rifiuta il valore ignoto nominando quelli ammessi. S6
estende quel comportamento a tutte le opzioni, e aggiunge il rifiuto delle
chiavi sconosciute che oggi manca ovunque.

## Requisito

Da L0.7 del decision package:

> Registry `plenora-io-model::format_options` con `FormatOptionsSchema` per
> driver; chiavi sconosciute → `PlenoraIoError::Unsupported`; valori invalidi →
> errore tipizzato.

Test attesi, che il gate di release verifica **per nome**:
`every_driver_has_a_schema_for_options`,
`unknown_option_key_produces_typed_error_not_silent_ignore`,
`unknown_compression_value_produces_typed_error_not_snappy_default`.

## Forma proposta

```rust
// plenora-io-model/src/format_options.rs

/// In quale fase l'opzione ha significato.
pub enum FaseOpzione { Lettura, Scrittura, Entrambe }

/// Cosa il valore può essere. Deliberatamente povero: sono opzioni da riga di
/// comando, non un linguaggio.
pub enum ValoreAmmesso {
    /// Testo non vuoto: nome di colonna, nome di foglio.
    Testo,
    /// Uno di un insieme chiuso, elencato nell'errore quando non combacia.
    Enumerato(&'static [&'static str]),
    /// Booleano, nelle forme accettate dal progetto.
    Booleano,
    /// Un solo carattere ASCII.
    Carattere,
}

pub struct OpzioneFormato {
    pub chiave: &'static str,
    pub fase: FaseOpzione,
    pub valore: ValoreAmmesso,
    /// Il default **dichiarato**, non quello che capita: è ciò che il comando
    /// `options` mostrerà e ciò che il driver deve applicare quando la chiave
    /// è assente.
    pub predefinito: Option<&'static str>,
    pub descrizione: &'static str,
}

pub struct SchemaOpzioniFormato {
    pub driver: &'static str,
    pub opzioni: &'static [OpzioneFormato],
}
```

La validazione è una funzione sola:

```rust
pub fn valida_opzioni(
    schema: &SchemaOpzioniFormato,
    opzioni: &BTreeMap<String, String>,
    fase: FaseOpzione,
) -> Result<()>;
```

## Tre decisioni da ratificare

### D1 — dove vive lo schema

| Opzione | Pro | Contro |
|---|---|---|
| **A. Registry nel modello**, tabella per id di driver | è ciò che L0.7 dice; il modello non dipende da nessuno, quindi il registry è consultabile da CLI e SDK senza tirarsi dietro i driver | l'id lega schema e driver **per stringa**: un driver senza schema è un buco che solo un test può trovare |
| **B. Campo nel `FormatDescriptor`** | il legame è per costruzione: un driver senza schema non compila, e `every_driver_has_a_schema_for_options` diventa una tautologia invece di un test | contraddice la lettera di L0.7; il descrittore vive in `plenora-io-core`, quindi il modello non lo vede |
| **C. Tipi nel modello, schemi nei driver, registry in core** | il legame resta per costruzione e i tipi restano dove L0.7 li vuole | tre posti invece di uno |

**Raccomandazione: C.** I tipi in `plenora-io-model::format_options`, come
chiede L0.7; ogni driver dichiara il proprio `static SCHEMA_OPZIONI` accanto al
proprio `static DESCRIPTOR`, come già fa per le capability; il registry per il
comando `options` si compone in core dall'elenco dei driver, che lì esiste già.
Il test `every_driver_has_a_schema_for_options` resta, ma verifica che il
registry sia **completo rispetto all'elenco dei driver**, non che qualcuno si
sia ricordato di aggiungere una riga.

### D2 — dove la validazione viene imposta

FZ-0 ha appena mostrato il costo della convenzione: la prevalidazione dei
decoder è corretta, ma nulla la lega alla chiamata che protegge, e serve un gate
apposta perché un percorso nuovo non la salti.

Qui il legame si può fare **nel tipo**, perché esiste già un passaggio
obbligato:

* lettura — `preflight_source(source, &mut opts)`, chiamata da tutti e dieci i
  driver;
* scrittura — `validate_write(descriptor, plan, max_columns)`, idem, e prende
  **già** il descrittore.

**Raccomandazione:** cambiare le due firme in
`preflight_source(descriptor, source, &mut opts)` e
`validate_write(descriptor, plan, opts)`, e validare lì dentro. Un driver che
dimentichi la validazione non compila. Non serve un gate, e il precedente di
FZ-0 dice perché conviene.

Costo: due firme cambiate in dieci driver. È un cambio meccanico, e
`validate_write` riceverebbe le opzioni invece di `max_columns()` — che è
proprio la forma già adottata altrove nel core, dove un commento spiega che
prendere le opzioni intere invece di una vista estratta evita di chiedere due
volte la stessa cosa.

### D3 — quanto essere severi, e cosa si rompe

Rifiutare vale solo se rifiuta davvero. Le conseguenze:

| Chiave | Prima | Dopo | Rompe |
|---|---|---|---|
| `compression` | valore ignoto → snappy | valore ignoto → `Unsupported` | chi passava un valore sbagliato e credeva di avere il default |
| `geometry_encoding` | ≠ `xy` → WKT | solo `xy` o `wkt`; altro → `Unsupported` | chi passava `WKT` maiuscolo o un refuso |
| `bbox_legacy_by_name` | non-vero → falso | booleano vero o falso; altro → `Unsupported` | chi passava `on` o `1.0` |
| `delimiter` | primo byte, vuoto → `,` | esattamente un carattere; altro → `Unsupported` | chi passava una stringa più lunga |
| chiave sconosciuta | ignorata | `Unsupported` con l'elenco delle chiavi accettate | i refusi, che è il punto |

Sono tutte rotture **volute**: in ogni riga il comportamento vecchio produceva
un risultato diverso da quello chiesto, senza dirlo. Vanno però dichiarate nel
change impact, perché una pipeline che oggi passa una chiave sbagliata domani
si ferma.

**Domanda aperta per la ratifica:** serve una via d'uscita — per esempio
`--allow-unknown-options` — per chi ha script che passano chiavi di altri
strumenti? La mia raccomandazione è **no**: una via d'uscita generale
riporterebbe il difetto sotto un altro nome, e chi ha davvero questo problema
può filtrare le chiavi prima di chiamarci. Se serve, va ratificata come
eccezione esplicita e non come default.

## Forma degli errori

Chiave sconosciuta:

```
PlenoraIoError::Unsupported(
  "csv: opzione 'wkt_colunm' sconosciuta in lettura; \
   accettate: delimiter, wkt_column, x_column, y_column"
)
```

Valore invalido:

```
PlenoraIoError::Unsupported(
  "geoparquet: compression 'zstdd' non valido; \
   accettati: zstd, gzip, brotli, lz4, none, uncompressed, snappy"
)
```

Il valore ricevuto **compare** nel messaggio. Non è una violazione della
redazione: un'opzione arriva dal chiamante — riga di comando o API — non dal
payload del file, e nasconderla renderebbe l'errore inutile proprio a chi deve
correggerlo. È la stessa scelta che `publish_mode` fa già oggi.

L'elenco degli ammessi viene dallo schema, quindi non può divergere da ciò che
il driver accetta davvero.

## Piano di test

| Test | Cosa dimostra |
|---|---|
| `every_driver_has_a_schema_for_options` | il registry copre l'elenco dei driver, non un elenco scritto a mano |
| `unknown_option_key_produces_typed_error_not_silent_ignore` | il refuso si ferma, e l'errore nomina le chiavi accettate |
| `unknown_compression_value_produces_typed_error_not_snappy_default` | il caso peggiore del censimento non degrada più |
| `una_chiave_di_scrittura_passata_in_lettura_e_rifiutata` | la fase è parte dello schema, non decorazione |
| `ogni_valore_ammesso_dallo_schema_e_accettato_dal_driver` | per ogni `Enumerato`, ogni valore elencato viene davvero accettato: impedisce che lo schema prometta più di quanto il driver mantenga |
| `il_default_dichiarato_e_quello_applicato` | il default dello schema è quello che il driver usa quando la chiave manca |

Gli ultimi due sono i più importanti e non sono nell'elenco di L0.7: senza di
essi lo schema diventa documentazione, cioè una seconda verità che diverge dal
codice in silenzio — esattamente il difetto che S6 esiste per chiudere.

## Cosa S6 non fa

* non introduce il comando `options` della CLI: L0.7 lo abilita, la facade lo
  espone (fuori dal Lotto 0);
* non tocca le opzioni **non** in `format_options` — `--assume-crs`, i limiti,
  `--durable` — che hanno già tipi propri;
* non cambia i valori accettati da nessun driver: li **dichiara**. Un driver che
  oggi accetta `xy` accetterà `xy`, e lo dirà.

## Sequenza di implementazione proposta

1. tipi e `valida_opzioni` in `plenora-io-model::format_options`, con i test
   della funzione pura;
2. `static SCHEMA_OPZIONI` per i quattro driver che hanno opzioni (csv, xls,
   geoparquet, shp) e schema vuoto per gli altri sei;
3. cambio delle due firme in core e aggancio della validazione;
4. rimozione dei default silenziosi nei driver, ora irraggiungibili;
5. registry e test di completezza;
6. change impact con la tabella delle rotture.

I passi 1-2 sono indipendenti e non rompono niente; il 3 è il cambio meccanico
sui dieci driver; il 4 è dove il comportamento cambia davvero.
