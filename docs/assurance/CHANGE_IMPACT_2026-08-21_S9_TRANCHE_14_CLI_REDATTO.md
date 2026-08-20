> Documento append-only: correzioni e seguiti vanno in coda, non nel corpo.

# Change impact analysis — S9 tranche 14: `plenora-io-cli` redatto

Data: 2026-08-21. Sigla: **S9 / tranche 14**, ultima del perimetro.
Baseline: `672c416` (tranche 13, `driver-dxf`).
`plenora-io-error-v1` **invariato**.

**Validazione di livello 1.** Verificato, non *release-qualified*.

## Problema

`plenora-io-cli` era l'ultimo crate con la via legacy aperta: **6 usi legacy di
produzione**. Il registro autorevole scende **6 → 0**: quattordici componenti su
quattordici a zero.

È anche il crate con la superficie di testo più esposta del workspace, perché è
l'unico che riceve testo direttamente dall'invocante — `argv`.

## L'audit ha trovato due helper, non uno

| Helper | Firma prima | Chiamanti |
|---|---|---|
| `usage_err` | `message: impl Into<String>` | **19** |
| `local_err_doc` | `message: impl Into<String>` | **5** diretti, oltre a `usage_err` |

`local_err_doc` non era emerso dalla prima passata: `usage_err` lo copre per 19
siti su 24, e solo i cinque errori di compilazione hanno rivelato che aveva
chiamanti propri. **Il censimento manuale in due tempi — prima l'helper ovvio,
poi ciò che il compilatore rifiuta — è stato necessario anche qui.**

Le firme sono ora `&PublicMessage` entrambe.

## Gli argomenti della riga di comando, per provenienza

La domanda che decide ogni sito non è «è testo runtime?» ma **da dove viene**:

| Valore | Siti | Provenienza | Esito |
|---|---:|---|---|
| nome del flag (`{flag}`) | 4 | letterali nostri, tutti i chiamanti passano `&'static str` | **conservato** — `CuratedPair(flag, …)` |
| indice di layer | 2 | `--layer N`, un `u32` | **conservato** — `NumeroStrutturale::Indice` |
| conteggio dei layer sorgente | 1 | numero | **conservato** — `NumeroStrutturale::Conteggio` |
| token di opzione sconosciuta | 1 | **`argv`** | **eliminato** |
| estensione non riconosciuta | 1 | **`argv`** (dal percorso) | **eliminato** |
| valore di `--opt` malformato | 1 | **`argv`** | **eliminato** |
| id del driver di destinazione | 1 | `&'static str` da `descriptor().id()` | **eliminato** per forma, non per provenienza (vedi sotto) |

La firma di `parse_usize`/`parse_u64` è stata stretta da `flag: &str` a
`flag: &'static str`: **il vincolo che rende vera l'affermazione «vocabolario
chiuso» è ora nel tipo**, non in una nota.

### I due elenchi ammessi recuperano ciò che i token toglievano

Togliere `opzione sconosciuta: {other}` costa all'utente interattivo la cosa che
gli serve: *quale* opzione. La CLI, in errore, emette il documento JSON su
stderr e nient'altro — non c'è un canale umano separato dove ripiegare.

Due costanti risolvono il problema senza testo runtime:

```rust
const ESTENSIONI_AMMESSE: &str = "parquet, geojson, csv, gpkg, shp, kml, …";
const OPZIONI_AMMESSE: &str = "--assume-crs, --durable, --in-opt, --layer, …";
```

Sono letterali nostri, quindi possono comparire in un messaggio pubblico. Il
messaggio dice ora *quali* opzioni esistono invece di ripetere quella sbagliata:
per un errore d'uso è **più utile**, non meno.

### `RejectedOptionToken` non è stato usato, ed è deliberato

`PublicMessage::OpzioneRifiutata` esiste apposta per far uscire un token
bounded. Non è applicabile qui: `RejectedOptionToken::conia` è **privata del
modulo** per costruzione — «chiunque voglia far uscire testo runtime da un errore
deve passare da qui, e qui non ci si arriva da fuori».

Aprire una via perché la CLI potesse coniarne allargherebbe un'eccezione
confinata per ratifica. **Sarebbe una decisione, non un dettaglio di
migrazione**, e non è stata presa.

### L'id del driver

Il messaggio single-layer citava `dst.descriptor().id()`, che è `&'static str`
da un `const fn`: **conservabile per provenienza**. È stato tolto per forma —
nessuna variante di `PublicMessage` porta insieme uno statico e un numero, e il
conteggio dei layer è l'informazione che serve davvero. Aggiungere una variante
avrebbe richiesto un'errata.

## Il quartetto, e il punto che nessuno snapshot vede

`local_err_doc` usava `PlenoraIoError::new`, che imposta `code = Generic`. È
stato convertito in:

```rust
&PlenoraIoError::redatto(
    plenora_io_model::IoErrorCode::Generic,
    category, phase, RemoteEffect::None, RetryDisposition::Never, message,
)
```

**`redatto` con `Generic`, non un costruttore di famiglia** — esattamente la
regressione della tranche 2, evitata a mano.

`check_quartetto_sito.py`: **0 differenze**.

Ma qui lo snapshot **non basta**, e va detto: sul wire il `code` della CLI non
viene da `IoErrorCode`, viene dal letterale passato a `err_doc`
(`"CLI_USAGE"`, `"NO_LAYER"`, …). Il gate osserva i costruttori, non
`err_doc`. Per la via d'uso il quartetto è fissato **da un test**, non dallo
snapshot.

## I due test nuovi

### La busta d'uso non era coperta

Un test sulle sei chiavi esisteva già, ma passa per `map_err` — la via degli
errori dei driver. La via **`usage_err` → `local_err_doc` → `err_doc`** non
aveva alcuna verifica di forma.

`la_busta_degli_errori_d_uso_ha_esattamente_le_sei_chiavi_v1` fissa le sei
chiavi (`category`, `phase`, `remote_effect`, `retry`, `code`, `message`), l'exit
`2`, e il quartetto della via d'uso.

Un'asserzione del test era sbagliata e il test l'ha corretta: `retry` sul wire è
un oggetto `{"kind": "never"}`, non una stringa nuda. Era stata scritta su come
me l'aspettavo invece che sull'osservato.

### Nessun argomento entra nella busta

`nessun_argomento_della_riga_di_comando_entra_nella_busta` costruisce sei
invocazioni con un marcatore improbabile e verifica che non compaia nel
documento serializzato.

**Sonda negativa eseguita**: reintroducendo `format!("opzione sconosciuta:
{other}")`, il test diventa rosso e stampa il marcatore dentro la busta. Il test
non è vacuo.

## Un warning di clippy che è un risultato

Dopo la migrazione, `other` nel match delle estensioni è diventato **variabile
inutilizzata**: esisteva solo per finire nel messaggio. Ora è legato a `_`, e il
binding è la prova — verificata dal compilatore — che il valore non esce più.

## I sei siti restanti sono nei test

Il censimento conta la produzione. `plenora-io-cli` ha **6 ulteriori usi legacy
nei propri test** (righe 1632, 1665, 1784, 1785, 1808, 2079), che il gate non
conta e che **si romperanno alla rimozione della via legacy**. Non sono debito
di questa tranche; sono lavoro della prossima, ed è registrato qui perché non
vengano scoperti allora.

## Impatto sui consumatori

**Il testo di `message` cambia.** Rottura già ratificata.
**Lo schema di `plenora-io-error-v1` non cambia**: sei chiavi, ora verificate su
entrambe le vie.

## Verifica

* `scripts/check_errori_redatti.py`: **0 residui**; quattordici componenti a zero;
* `scripts/check_quartetto_sito.py`: **0 differenze**;
* clippy e test verdi con e senza feature;
* **14 sonde dei gate** derivate dallo script del checkpoint — non rielencate a
  mano: l'elenco è estratto da `scripts/s9-checkpoint.sh`, che è la fonte
  autorevole di quali siano e di come si invochino;
* `check_assurance_fallbacks`, `check_assurance_n1 --integrita`: verdi.

## Prossimo passo

**Rimozione della via legacy, in un commit distinto**, con prova di entrambe le
proprietà: zero utilizzi nel workspace, e un consumatore esterno che non può più
costruire errori attraverso la vecchia API. Poi i test ostili conclusivi e il
checkpoint finale con baseline `0474902`.
