# Change impact analysis — S9 tranche 5: `driver-csv` redatto

Data: 2026-08-20. Sigla: **S9 / tranche 5**.
Baseline: `607f658` (tranche 4, `driver-geojson`).
`plenora-io-error-v1` **invariato**.
Qualifica: **livello 1** — verificato, **non release-qualified** (design § 20).

## Il censimento, ora su due classi

Prima tranche condotta con la regola estesa del 2026-08-20 (design § 21): si
cerca la **fuga di testo** *e* la **cancellazione di struttura**.

### Classe 1 — fuga di testo

| Via | Firma | Chiamanti |
|---|---|---:|
| `err` | `impl Into<String>` | 35 |

### Classe 2 — cancellazione di struttura

| Forma | Occorrenze | Origine del testo |
|---|---:|---|
| `map_err(\|e\| err(e.to_string()))` | **14** | crate `csv`, percorso di scrittura |
| `err(format!("…: {e}"))` | **9** | crate `csv` (apertura, riga, coordinate), `arrow` (`record batch`) |
| `Result<_, String>` | 0 | — |
| `DeError::custom` | 0 | — |

**Nessun canale `Result<_, String>` e nessun `DeError::custom`**: `driver-csv`
non usa serde per la lettura. La cancellazione di struttura qui è tutta nella
forma `to_string()` diretta — 23 siti — e non c'era una perdita di *codice*
d'errore come in `driver-geojson`, perché il testo veniva riclassificato subito
da `err` come `Format`, che è la classificazione giusta per quei casi.

Questo è il risultato che rende utile la distinzione: la stessa forma di
codice, in due driver, ha conseguenze diverse. In `driver-geojson` cancellava
il tipo; qui cancellava solo il dettaglio.

### Fughe reali chiuse

* **23 di testo di dipendenza** — crate `csv` (22) e `arrow` (1);
* **3 di valori d'opzione** — i nomi di `wkt_column`, `x_column`, `y_column`
  interpolati nei messaggi di colonna assente;
* **1 di valore d'opzione** — `geometry_encoding` nel ramo difensivo;
* **1 percorso di filesystem** — `OutputExists(path.display().to_string())`.

## Perdita diagnostica dichiarata

I nomi delle colonne geometriche **non entrano più nei messaggi**.

Sono valori d'opzione, non payload — e la ratifica S6 aveva riconosciuto che
un'opzione arriva dal chiamante e nasconderla rende l'errore inutile a chi deve
correggerla. Ma il meccanismo ratificato per farla uscire è il
`RejectedOptionToken`, coniabile **solo** dentro `valida_opzioni`, e qui non si
applica: `wkt_column`, `x_column` e `y_column` sono dichiarate
`ValoreAmmesso::Testo`, quindi lo schema le accetta; il rifiuto nasce dopo, dal
confronto con l'intestazione di **questo** file.

Il messaggio dice ora «colonna WKT assente dall'intestazione» senza dire quale
nome è stato cercato. Chi ha scritto l'opzione lo sa; chi legge il log dopo,
no.

**È un caso che merita una decisione esplicita**, non una scelta implementativa:
un token coniabile per «valore d'opzione rifiutato dal driver contro il file»
sarebbe la stessa garanzia del token esistente, applicata a un rifiuto che il
validatore centrale non può vedere. Non l'ho introdotto: allargare da solo
l'unica eccezione ratificata sarebbe il modo in cui un'eccezione diventa una
regola.

`geometry_encoding` invece **non è una perdita**: lo schema lo dichiara
`Enumerato(&["wkt", "xy"])`, quindi un valore diverso è già stato respinto da
`valida_opzioni` con il suo token. Il ramo nel driver è difensivo, e in pratica
irraggiungibile quando la validazione ha girato.

## Altri cambi

`colonne_geometriche` estratta da `open`, che aveva superato le cento righe
consentite da clippy. L'alternativa era accorciare il commento che spiega
perché il nome non esce — cioè togliere la parte da tenere. Il blocco era già
di fatto una funzione: ora ha un nome.

## Verifica (livello 1)

* `scripts/check_errori_redatti.py`: **133 → 125** in nove crate;
  `plenora-io-model`, `plenora-io-core`, `driver-common`, `driver-geojson`,
  `driver-csv` migrati e a **zero**;
* sonde del censimento: 7/7;
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: pulito;
* `cargo test --workspace --all-features`: verde su 31 binari;
* gate specifici: `check_quarantena_fuzz`, `check_prevalidazione_decoder`,
  `check_public_identity` verdi;
* `check_assurance_fallbacks`: **totale invariato a 115** — le conversioni
  `usize → u64` sono passate da `driver_common::saturating_u64`;
* smoke dei soli target coinvolti: `csv_reader`, `wkt_parse`.

## Prossimo passo

Tranche 6: `driver-kml` (12 usi legacy). **Dopo la tranche 6 è dovuto il
checkpoint di livello 2**: batteria completa, copertura same-SHA, smoke 13/13.
