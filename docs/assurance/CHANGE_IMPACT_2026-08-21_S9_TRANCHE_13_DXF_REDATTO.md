> Documento append-only: correzioni e seguiti vanno in coda, non nel corpo.

# Change impact analysis — S9 tranche 13: `driver-dxf` redatto

Data: 2026-08-21. Sigla: **S9 / tranche 13**.
Baseline: `0474902` (checkpoint di livello 2 superato).
`plenora-io-error-v1` **invariato**.

**Validazione di livello 1.** Il commit è *verificato*, non *release-qualified*:
nessuna misura di copertura lo accompagna, e la qualifica di release resta
sospesa alla batteria completa su un albero identico.

## Problema

`driver-dxf` era l'ultimo driver con la via legacy aperta: **20 usi legacy di
produzione**, penultimo crate del perimetro insieme a `plenora-io-cli`.

## Cosa cambia

Il registro autorevole (`scripts/check_errori_redatti.py`) scende **26 → 6**.
Tredici componenti su quattordici sono a zero; resta solo la CLI.

### I 20 usi diretti, per costruttore

| Costruttore legacy | usi |
|---|---:|
| `LimitExceeded` | 9 |
| `Crs` | 5 |
| `Unsupported` | 3 |
| `format` | 1 |
| `crs_unresolved` | 1 |
| `OutputExists` | 1 |
| **totale** | **20** |

### I 42 che il censimento non vedeva

`fn err(reason: impl Into<String>)` avvolgeva `PlenoraIoError::format`. Nel
registro vale **1**; i siti che ci passavano sono **42**.

Le tre categorie vanno lette separate, perché non sono sinonimi:

| Categoria | Siti |
|---|---:|
| usi legacy diretti (ciò che il gate conta) | 20 |
| chiamanti dell'helper `err` (ciò che il gate non vedeva) | 42 |
| `format!` che costruivano un messaggio | 26 |
| di cui con **testo della dipendenza** (`{e}` / `{error}`) | 10 |
| di cui con **valore dal payload** | 1 |

**«Zero chiamate dirette» avrebbe chiuso 20 siti su 61.** È la stessa lezione
delle tranche 2, 7, 10 e 12: il gate conta i costruttori, non gli helper che li
avvolgono, e l'ispezione manuale resta dovuta una volta per crate.

La firma è ora `fn err(reason: &PublicMessage)`.

## La fuga di payload

Un sito faceva uscire un valore **letto dal file DXF**:

```rust
return Err(err(format!("riferimento ciclico al blocco '{}'", insert.name)));
```

`insert.name` è il nome di un blocco, scritto da chi ha prodotto il file — cioè
esattamente la categoria che INV-10 vieta. Ora:

```rust
// Il nome del blocco non esce: e' letto dal file DXF. Resta la
// condizione, che e' cio' che il chiamante non puo' dedurre.
return Err(err(&PublicMessage::Curated("riferimento ciclico fra blocchi DXF")));
```

Chi legge l'errore perde *quale* blocco chiude il ciclo. Lo conserva chi ha il
file, che è la stessa parte che lo ha prodotto.

## Cosa esce dai messaggi

| Informazione | Prima | Ora | Esito |
|---|---|---|---|
| nome del blocco DXF ciclico | interpolato | assente | **eliminato**: viene dal payload |
| testo di errore della crate `dxf`, di `arrow`, di UTF-8 | interpolato (10 siti) | frase curata per condizione | **perduto**: era testo di dipendenza |
| identificativo del CRS non risolvibile | interpolato | assente dal messaggio | **perduto dal messaggio**, leggibile dal contratto |
| dimensionalità delle coordinate | `{dimensions:?}` | `CoordinateDimensions::nome()` | invariato nella sostanza, stabile nella forma |
| percorso di destinazione esistente | interpolato | assente | **eliminato**: è un percorso del chiamante |
| tetti: entità, annidamento INSERT, righe, vertici, colonne, byte di output e di spool | interpolati | `NumeroStrutturale::{Conteggio,Limite}` | **conservati**: sono numeri nostri |

Le sette soglie restano leggibili perché sono costanti del driver o limiti
configurati, non valori letti dal file.

## Il quartetto: invariato, e dimostrato

`scripts/check_quartetto_sito.py` confronta lo snapshot per `percorso::funzione`
in sequenza ordinata: **0 differenze**.

È la prima tranche in cui il gate lavora su una migrazione vera. Alla tranche 2
non esisteva, e la sua assenza aveva lasciato passare `code` da `Generic` a
`Schema` — un cambio sulla chiave di compatibilità ratificata, dichiarato
assente in buona fede. Qui l'invarianza è **misurata**, non asserita.

## Canali che cancellano la struttura

Cercati e **assenti** in `driver-dxf`: nessun `Result<_, String>`, nessun
`DeError::custom`, nessun `map_err(|e| e.to_string())`, nessun errore
strutturato reinserito in un `format!`.

## Ciò che resta, e perché

`crates/driver-dxf/src/lib.rs:537`

```rust
self.loss.record(&format!("attributo non rappresentato in DXF: {c}"), self.rows);
```

Porta un nome di colonna, ma in un **report di perdita**, non in un messaggio
d'errore: struttura diversa, contratto di wire diverso. È la stessa superficie
adiacente già segnalata in `driver-gpkg`. Resta **fuori dal perimetro di S9**;
segnalarla qui serve a non farla sparire.

I tre `format!("EPSG:{code}")` in `epsg_from_definition` costruiscono un
identificativo CRS — un valore di dominio, non un messaggio. Fuori perimetro.

## Impatto sui consumatori

**Il testo di `message` cambia** in tutti i siti migrati: rottura già ratificata
(decisione 2 di S9). La chiave di compatibilità è
`(category, phase, code, retry)`, ed è invariata — per snapshot, non per
ispezione.

**Lo schema di `plenora-io-error-v1` non cambia.**

## Verifica

* `scripts/check_errori_redatti.py`: **6** residui in **1** crate; tredici
  componenti migrati e a zero;
* `scripts/check_quartetto_sito.py`: **0** differenze;
* `cargo test --workspace --all-features`: verde, **31** binari;
* `check_assurance_fallbacks`: **119**, invariato;
* replay deterministico su `dxf_reader`: **5 030 input**, nessun crash;
* smoke su `dxf_reader`: nessun finding, 0 in quarantena.

Fuzz e copertura sono mirati sul target coinvolto, come previsto dalla
validazione a due livelli: la batteria completa è dovuta al checkpoint.

## Prossimo passo

Tranche 14 e ultima del perimetro: **`plenora-io-cli`**, 6 usi legacy. Poi la
rimozione dei costruttori legacy con prova di non costruibilità, i test ostili
conclusivi e il checkpoint finale con baseline `0474902`.

---

## Addendum del 2026-08-21 — riconciliazione del registro fallback 115 → 119

Il corpo di questa CIA dichiara «`check_assurance_fallbacks`: **119**,
invariato». È vero per la tranche, e **non basta**: il totale precedente era
115, e un totale che si muove senza che nessuno nomini le identità non dimostra
che l'aumento sia legittimo. Qui vengono nominate.

### Le quattro identità

Tutte in `crates/driver-common/src/wkt_lossless.rs`, tutte nella **stessa
funzione di test** `cio_che_accettiamo_da_testo_lo_sappiamo_riscrivere`, in un
modulo `#[cfg(test)]`:

| Riga | Sito | Motivazione |
|---|---|---|
| 873 | `format_wkt(geometria).unwrap_or_else(\|errore\| panic!("{testo}: accettato in lettura ma non riscrivibile: {errore}"))` | asserisce la simmetria lettura/scrittura |
| 881 | `parse_wkt(testo).unwrap_or_else(\|errore\| panic!("{testo}: {errore}"))` | il caso *deve* essere accettato |
| 883 | `format_wkt(&geometria).unwrap_or_else(\|errore\| panic!("{testo}: {errore}"))` | e *deve* essere riscrivibile |
| 885 | `parse_wkt(&riscritto).unwrap_or_else(\|errore\| panic!("{testo}: {errore}"))` | e rileggibile identico |

**Nessuna delle quattro è una degradazione a un valore di ripiego.** Sono la
forma `unwrap_or_else(|e| panic!(…))`: il modo in cui quel test dice «questo
caso doveva passare, e se non passa voglio sapere perché». La regex del gate
(`\bunwrap_or(?:_else|_default)?\s*\(`) non distingue le due cose, ed è giusto
che non lo faccia — distinguere è il lavoro del registro, non della regex.

### L'aumento non è di fallback: è di visibilità

| | |
|---|---|
| nascita dei quattro siti | `d52a8dd`, **2026-08-07** — `fix(wkt): serializza i MULTIPOLYGON con membri vuoti invece di panicare (#15)` |
| rapporto con S9 | li **precede di tredici giorni**; antenato di `0474902` |
| introdotti da questa tranche | **zero**: `driver-dxf/src/lib.rs` conta 6 prima e 6 dopo `672c416` |

Il gate testuale (`check_assurance_fallbacks.sh`) **non elencava affatto
`driver-common`**, e contava solo i crate elencati: quel crate non veniva
guardato. INFRA-4 (`71adf70`) ha aggiunto il controllo «crate presente ma non
registrato», che lo ha trovato.

### La conclusione da trarne

**115 non era il valore corretto di prima: era un valore sbagliato.** Il
workspace conteneva 119 occorrenze dal 2026-08-07, e il gate ne stampava 115
perché misurava un sottoinsieme presentandolo come totale.

È la stessa famiglia di difetto incontrata sei volte in questa serie — un verde
che riguarda meno di quanto dichiari — e qui si presentava nella forma più
insidiosa: non un passo saltato, ma un **perimetro incompleto**, che nessun
conteggio di passi avrebbe rivelato.

Il controllo che lo chiude non è il numero: è il fatto che un crate nuovo, o
dimenticato, ora renda il gate **rosso** invece che silenzioso.
