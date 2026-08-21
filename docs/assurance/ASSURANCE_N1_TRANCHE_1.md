> Documento append-only: correzioni e seguiti vanno in coda, non nel corpo.

# ASSURANCE-N1 — tranche 1: `driver-xls`, rami negativi di `open`, `create`, `validate_archive_ratio`

Data: 2026-08-21. **45 → 43** gruppi aperti.

## Che cosa è stato chiuso, e con quale prova

| Gruppo | Esito | Prova |
|---|---|---|
| `driver-xls::open` | **chiuso** | 3 rami coperti, verificato per misura LCOV |
| `driver-xls::create` | **chiuso** | 1 ramo coperto, 2 provati **irraggiungibili** |
| `driver-xls::validate_archive_ratio` | **resta aperto** | 1 ramo su 3 coperto — nessuna compensazione |

## Il difetto che la misura ha trovato

`n1_create_rifiuta_un_piano_che_non_ha_esattamente_un_layer` **passava, e provava
un'altra cosa.**

La copertura ha mostrato che il ramo di `create` (392-394) restava **scoperto**:
`validate_write` nel core ferma entrambe le classi — piano vuoto e piano con due
layer — prima che `create` le veda. L'asserzione sulla sola categoria
`Unsupported` era soddisfatta da un errore diverso.

Un test verde che non tocca il ramo che dichiara è precisamente ciò che
ASSURANCE-N1 esiste per escludere. Sarebbe stato consegnato come «gruppo chiuso»
se non fosse stata misurata la copertura riga per riga.

### Come è stato corretto

I test dicono ora ciò che misurano, e provano un contratto **più forte**: la
**precedenza**.

| Test | Asserzione decisiva |
|---|---|
| `n1_un_piano_senza_un_solo_layer_e_fermato_prima_di_create` | `code == Capability`, che è la firma di `validate_write` |
| `n1_un_geometry_encoding_non_ammesso_e_fermato_prima_di_create` | il messaggio enumera gli ammessi, firma di `valida_opzioni` |

Senza quelle asserzioni i test tornerebbero a essere soddisfatti da un rifiuto
qualunque. Il commento nel codice diceva «questo ramo è difensivo»: adesso è
**misurato**, e i due rami irraggiungibili hanno un test che spiega *perché* lo
sono.

## Che cosa resta aperto in `validate_archive_ratio`, e perché

Coperto: l'overflow del prodotto `compressed * decompression_ratio`, con un
moltiplicatore `u64::MAX` su un file normale — la classe di equivalenza della
precondizione, non un archivio patologico.

Scoperti: i due `checked_add` che sommano le dimensioni **dichiarate nel central
directory** dello zip, cioè valori controllati da chi costruisce l'archivio.
Potenzialmente raggiungibili con un ZIP64 artefatto.

**Non sono dichiarati difensivi**, perché non è stato misurato se il crate `zip`
li rifiuti prima. Dichiararlo senza prova sarebbe la stessa supposizione che
questa tranche ha appena smontato in `create`.

## Il gate è stato rafforzato due volte

### Prima: «chiuso» deve nominare una prova

Il gate accettava `chiuso` con la sola nota. Nulla verificava che un test
esistesse — il «semplice riallineamento del registro». Appena introdotto il
campo `prova`, ha preteso la prova per l'unico gruppo già chiuso, che la
nominava solo in prosa.

`strutturale` e `difensivo` **non** la richiedono, ed è voluto: dicono che il
ramo non è esercitabile da un input, quindi un test che lo esercitasse non
potrebbe esistere. La loro forza sta nella nota, che un revisore può contestare.

### Poi: la prova deve essere un test **eseguito**

Cercare `fn <nome>(` dimostra che un simbolo esiste. Un simbolo può essere un
helper senza `#[test]`, un test `#[ignore]`, un test sotto un `cfg` inattivo, o
un omonimo in un altro modulo. Nessuno dei quattro copre un ramo, e tutti e
quattro passavano.

`scripts/check_assurance_n1_prove.py` **esegue** il harness per ogni coppia
`(crate, configurazione)` distinta — deduplicata — e pretende che ogni identità
compaia una volta sola e con esito `ok`.

**Verificato**: marcando un test `#[ignore]`, il gate nuovo diventa rosso e
quello statico **resta verde**.

La configurazione fa parte dell'identità perché deve: `--all-features` abilita
`gdal-backend`, e il ramo stub di `driver-filegdb` esiste solo senza.

### `coperto` e `irraggiungibile` sono esiti distinti

Una prova di irraggiungibilità dichiara `righe` — quali restano scoperte — e
`guardia` — quale controllo rifiuta per primo. **Non compaiono come rami
coperti**: presentarle così sarebbe la compensazione che ASSURANCE-N1 esclude.

## ASSURANCE-N1 non era cablato nel checkpoint

Solo il registro fallback lo era. I gate N1 li eseguivo nelle batterie composte
a mano, e **da quando il livello 1 deriva dallo script hanno smesso di girare
del tutto**.

È la stessa lezione di `fmt`, e stavolta è costata meno solo perché il registro
non è cambiato nel frattempo. Ora sono quattro passi: sonde e integrità del
registro, sonde ed esecuzione delle prove.

## Velocità: che cosa costa davvero

Tre gruppi affrontati, due chiusi. Il tempo **non** è andato nello scrivere i
test — sono venuti al primo colpo — ma nel **determinare la raggiungibilità dei
rami**, ed è lì che stavano i difetti.

Se il rapporto reggesse, i 43 restanti sarebbero una quindicina di tranche. Ma
i 26 gruppi di `driver-shp` sono di natura diversa, quindi la proiezione va
presa per quello che è: **il costo dominante è la determinazione della
raggiungibilità, non il numero grezzo dei gruppi**, e quella non si parallelizza
leggendo i commenti.

## Verifica

* 6 test nuovi in `driver-xls`, tutti verdi;
* copertura misurata riga per riga, prima e dopo;
* `check_assurance_n1 --integrita`: 49 gruppi, 43 ancora senza copertura;
* `check_assurance_n1_prove.py`: 6 prove eseguite su 2 configurazioni, 4
  coprono un ramo, 2 provano un'irraggiungibilità;
* sonde: 16 per il registro, 15 per le prove eseguite;
* livello 1: **45 passi, 9 omessi, verde**.
