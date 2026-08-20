> Documento append-only: correzioni e seguiti vanno in coda, non nel corpo.

# Change impact analysis — S9: test ostili conclusivi

Data: 2026-08-21. Sigla: **S9 / test ostili**.
Baseline: `e2c7f8f` (limite di provenienza di `'static`).
`plenora-io-error-v1` **invariato, incluso l'insieme esatto delle chiavi**.

**Validazione di livello 1.** Verificato, non *release-qualified*.

## Problema

L'enforcement di S9 è completo: il censimento è a zero, i costruttori legacy
non esistono, i gate coprono produzione, test, doctest e `fuzz/`. Tutto questo
prova che **il codice non può** far uscire testo runtime.

Non prova che **non lo faccia**. Un driver può comporre un messaggio curato che
contiene comunque ciò che non deve, o propagare il testo di una dipendenza per
una via che nessun gate guarda. La differenza è fra una proprietà del sorgente e
una proprietà del comportamento.

## Che cosa è stato costruito

`crates/plenora-io-cli/tests/ostili.rs`: un **test d'integrazione**, cioè un
crate separato che dei driver vede soltanto l'API pubblica. Un test dentro un
driver potrebbe chiamare un helper interno e verificare una busta che nessuno
costruisce davvero.

Le chiamate attraversano gli entry point veri — `FormatDriver::open`,
`OpenDatasetHandle::open_layer_reader`, `LayerReader::next_batch`,
`FormatDriver::create` — e in un caso il **processo**: `CARGO_BIN_EXE_plenora-io`
eseguito come sottoprocesso, con la busta letta da stderr.

La busta è **serializzata**, non ispezionata via `Display`: sono due superfici
diverse, e `Display` può essere innocuo mentre `message` porta il payload.

### Le fixture

Quattordici file sotto `tests/fixtures/ostili/`, byte-esatti e **versionati**,
ciascuno con il marcatore `ZZ-MARCATORE-PAYLOAD-9F3A-ZZ` in chiaro.

Versionati e non generati: una fixture generata cambia con la libreria che la
genera, e un test ostile che cambia da solo non prova niente.

## I test, e che cosa ciascuno prova

| Test | Fase | Proprietà |
|---|---|---|
| `il_marcatore_e_davvero_nelle_fixture` | — | **premessa**: senza, l'intera batteria sarebbe vacua |
| `nessun_driver_fa_uscire_il_payload_in_apertura` | apertura | 10 driver, file non nel formato dichiarato |
| `nessun_driver_fa_uscire_il_percorso_di_una_sorgente_assente` | apertura | 10 driver, percorso inesistente col marcatore nel nome |
| `una_lettura_fallita_non_consegna_righe_ne_payload` | lettura | guasto a metà stream, **zero righe consegnate** |
| `una_scrittura_rifiutata_non_lascia_destinazione_ne_payload` | scrittura | piano non rappresentabile, **nessun file lasciato** |
| `una_destinazione_esistente_non_viene_sovrascritta_...` | scrittura | il contenuto preesistente è invariato |
| `filegdb_senza_feature_e_uno_stub_tipizzato` | apertura | `cfg(not(gdal-backend))` |
| `filegdb_con_gdal_non_fa_uscire_il_testo_della_dipendenza` | apertura | `cfg(gdal-backend)` |
| `la_busta_del_binario_ha_le_sei_chiavi_...` | processo | le sei chiavi v1, exit, stdout vuoto |
| tre sonde `la_verifica_diventa_rossa_...` | — | **l'harness non è cieco** |

Ogni errore passa da `verifica`, che controlla marcatore, testo di dipendenza,
tetto del messaggio e `remote_effect`.

### `remote_effect` è il quinto asse, e va guardato

Un guasto locale che dichiarasse `RemoteEffect::Unknown` farebbe ritentare al
chiamante un'operazione che non è mai partita. `verifica` lo pretende `None` su
tutti i driver, che sono tutti locali; il binario lo verifica anche sul wire.

## Tre cose che la costruzione ha corretto

### 1. Il nome della dipendenza non è testo della dipendenza

La prima stesura vietava `"parquet"` e `"GDAL"`. Entrambi facevano fallire
messaggi **corretti**:

* `"parquet"` matcha `"driver":"geoparquet"` nella busta serializzata;
* `"apertura GDAL fallita"` è un letterale scritto da noi, che dice al lettore
  quale via è stata presa — informazione sul nostro build, non sul payload.

L'elenco vieta ora ciò che GDAL **produce** — `CPLE_`, `ERROR 4:`, `OGRErr` — e
i frammenti si cercano su `message`, non sulla busta intera che contiene per
forza il nome del driver.

### 2. Il marcatore in `field` non è una fuga, ed è stato reso esplicito

Con un contratto ostile il nome del campo *è* il marcatore, e finisce nello slot
tipizzato `field`. È la decisione della tranche 2 — spostarlo lì invece di
interpolarlo — ed è **ciò che rende il messaggio curato**.

`field` **non è sul wire v1**: `err_doc` emette sei chiavi e quella non c'è.

Il test fa quindi due affermazioni distinte invece di una vaga:

1. `message` non contiene mai marcatore né testo di dipendenza. Nessuna
   tolleranza;
2. se il marcatore compare nel tipo Rust serializzato, può stare **solo** in
   `field`, **una volta sola**.

### 3. La fase di lettura è provata da un driver, non da tre — misurato

`csv` e `kml` falliscono in **apertura**, per ragioni verificate:

* `csv` — `infer_wkt_geometry` parsa **ogni** WKT del file durante l'inferenza,
  quindi una geometria rotta è colta in apertura qualunque riga occupi. Nessuna
  fixture può spostare quel guasto alla lettura: non è un limite della fixture,
  è come il driver è fatto;
* `kml` — le coordinate non valide sono rilevate nella scansione che precede il
  reader.

Il fail-fast è un comportamento **migliore**, non peggiore. Ma cambia che cosa
il test copre, e restano entrambi nell'elenco con la ragione scritta: toglierli
farebbe sparire il fatto.

## Contro la vacuità

Un test con rami `continue` e `Ok(_)` può essere verde senza aver misurato
niente. È la classe di difetto che questa serie ha incontrato sette volte.

| Contatore | Asserzione |
|---|---|
| `aperture_riuscite` | almeno un'apertura è riuscita, altrimenti la lettura non è mai stata raggiunta |
| `letture_fallite` | almeno una lettura è fallita **dopo** un'apertura riuscita |
| `rifiuti` | almeno un driver ha rifiutato il piano ostile |

I due primi sono separati perché rispondono a domande diverse, e il secondo non
implica il primo per caso.

E tre sonde verificano che **`verifica` diventi rossa** quando deve: su
marcatore nel messaggio, su testo di dipendenza, su `remote_effect` diverso da
`None`. Il messaggio avvelenato si costruisce con
`PublicMessage::Curated(MARCATORE)` — il marcatore è un `&'static str`, quindi
**nessuna scorciatoia** è stata introdotta: non esiste in quel file una via di
costruzione degli errori che non esista anche in produzione.

## Due gate hanno reagito, ed è il perimetro esteso che funziona

Il file nuovo è finito sotto `crates/`, quindi entrambi i gate lo hanno visto.

### Registro fallback: 28 → 32

Quattro `unwrap_or` nel test. **Eliminati invece che registrati**:

* l'estensione della destinazione sta ora nella struttura `Caso` invece di
  essere dedotta dal nome della fixture con un ripiego `"dat"`. Un ripiego che
  non scatta mai è peggio di inutile: nasconderebbe una fixture rinominata
  dietro un nome plausibile;
* `read_to_string(...).unwrap_or_default()` è diventato `expect`: un ripiego a
  stringa vuota trasformerebbe «la destinazione è stata cancellata» in «il
  contenuto è cambiato», cioè in una diagnosi sbagliata.

Registro di nuovo a **119**.

### Quartetto: tre siti nuovi, e una lacuna vera

Il gate del quartetto **esclude già** il codice di test — ma `righe_di_test`
riconosce i moduli `#[cfg(test)]`, e un test d'integrazione non ne ha: è tutto
codice di test **per posizione**. Una lacuna accidentale, visibile solo quando
qualcuno scrive il primo test d'integrazione.

Colmata con `e_test_per_posizione`, che riconosce
`crates/<crate>/{tests,benches,examples}/`, applicata **solo** al gate del
quartetto. Il censimento continua a contare i test: è la decisione presa alla
rimozione legacy, e le due regole restano distinte perché rispondono a domande
diverse.

Senza quella riga ogni test nuovo sarebbe diventato un aggiornamento di
contratto — e un contratto che cambia a ogni test smette di essere letto.

Sonda aggiunta con sei casi, **due dei quali negativi** (`src/lib.rs` e
`src/tests/aiuto.rs` non sono test per posizione). Quartetto invariato a 28 file
e 131 funzioni.

## FileGDB: due prove separate

| Configurazione | Prova |
|---|---|
| default (stub) | `ErrorCategory::Unsupported`, cioè una capability mancante — **non** un guasto d'ambiente. Uno stub che dicesse «GDAL non trovato» farebbe cercare una libreria da installare, quando la verità è che questo binario non è stato costruito per parlare con GDAL |
| `gdal-backend` | il driver parla davvero con GDAL, e il testo che GDAL produce non arriva nella busta |

Sono `cfg`-gated, quindi ciascuna corre nella propria configurazione e nessuna
delle due può passare per assenza.

## Verifica

* `cargo test -p plenora-io-cli --test ostili`: **11 verdi**;
* `... --features gdal-backend`: **11 verdi**;
* clippy pulito con e senza feature;
* censimento 0, quartetto 28/131 a zero differenze, fallback 119, `niente_leak`
  con la sola attestazione, integrità N1 verde;
* 15 sonde derivate da `scripts/s9-checkpoint.sh`;
* `cargo +nightly fuzz build` verde.

## Che cosa resta

Il **checkpoint finale** con `S9_CHECKPOINT_BASE=0474902`. Se passa, S9 è chiuso.

Restano separatamente, e bloccano la release:

| | dove |
|---|---|
| 45 gruppi di copertura negativa | `ASSURANCE_N1_copertura_negativa.json` |
| fuzz target per il reader `.shp`/`.dbf` | ASSURANCE-N1 |
| spike di fattibilità fuzz per FileGDB | da aprire |
| contratto dei report di perdita | `DEBITO_contratto_report_di_perdita.md`, non ancora valutato |
