# FileGDB: projection, determinismo e matrice GDAL

Stato al 2026-07-30: **projection Arrow esatta, estrazione indicizzata,
pushdown nativo delle colonne, determinismo semantico e GDAL nativo Windows
verificati**.

## Risultato tecnico

Il reader costruisce esclusivamente builder e array per i campi richiesti e
non invoca `Geometry::wkb` quando la geometria è esclusa. Per gli attributi
risolve l'indice OGR una volta dallo schema e usa gli accessor tipizzati per
indice: non ripete più la ricerca per nome e la costruzione della relativa
`CString` per ogni cella. Poiché il worker riapre il dataset, prima di leggere
confronta nome e tipo OGR di ogni indice selezionato con lo snapshot osservato
da `open`; una modifica concorrente dello schema fallisce chiusa.

Lo schema e il `RecordBatch` prodotti coincidono esattamente con la projection,
compreso il caso senza colonne. Il test con proiezione non contigua del campo
stringa, valori nulli e tipi numerici protegge la corrispondenza fra `FieldId`,
indice OGR e array Arrow.

Il pushdown nativo usa ora `OGR_L_SetIgnoredFields` tramite un wrapper safe
nel fork governato della crate `gdal 0.17.1`. Il worker calcola il complemento
degli attributi richiesti e usa il nome speciale `OGR_GEOMETRY` quando la
projection esclude la geometria. La lista viene installata dopo la verifica
TOCTOU dello schema e prima della prima `Feature`.

Non sono state adottate due scorciatoie:

- chiamare direttamente `gdal-sys` introdurrebbe `unsafe` nel crate, vietato
  dal profilo;
- sostituire il layer con una query OGRSQL `SELECT` cambia identità del layer,
  FID, comportamento della geometria e piano del driver; non è equivalente a
  ignorare campi e richiederebbe una nuova matrice semantica e prestazionale.

Il componente non contiene nuove chiamate `unsafe`: il confine FFI resta nella
dipendenza governata. Checksum, revisione upstream, delta e regola di
aggiornamento sono registrati in `vendor/gdal/PLENORA_FORK.md`.

## Evidenza prestazionale dell'estrazione indicizzata

La campagna del 2026-07-29 usa una fixture OpenFileGDB da 50.000 righe e 64
campi `Int32`, build release, sette coppie baseline/candidato alternate e un
warm-up per processo. La baseline di prodotto è
`ca39d6272b06e290f727b62200ea36cc25d6f826`; i due binari contengono lo stesso
harness e differiscono soltanto nel percorso di estrazione.

| Proiezione | Baseline mediana | Indicizzata mediana | Delta throughput |
|---|---:|---:|---:|
| geometria + 64 attributi | 63.522 righe/s | 354.959 righe/s | **+458,8%** |
| 3 attributi non contigui | 436.643 righe/s | 656.880 righe/s | **+50,4%** |

I checksum completi sono identici in tutte le esecuzioni. Il candidato supera
il veto (nessuna coppia è più lenta) e non modifica API pubblica, capability,
formato su disco o dipendenze. Queste misure caratterizzano throughput, non
worst-case execution time né schedulabilità real-time.

## Determinismo

`read_determinism` e `write_determinism` restano correttamente
`semantic`, non `byte_for_byte`: un directory dataset FileGDB contiene
artefatti e ordinamenti interni controllati da GDAL.

Il test feature-on
`repeated_filegdb_writes_are_semantically_deterministic`:

1. pubblica due `.gdb` indipendenti dallo stesso piano e dallo stesso batch;
2. riapre entrambi con due handle separati;
3. confronta nome layer, dimensioni, tipi, CRS, axis order e geometria
   decodificata;
4. non usa l'uguaglianza byte-per-byte come oracolo improprio.

Esecuzione locale su Linux x86_64, Rust 1.92.0 e GDAL 3.10.3: superata.

## Matrice osservata

| Ambiente | GDAL | Lettura/scrittura | Crash publish | Determinismo semantico | Pushdown nativo |
|---|---:|---|---|---|---|
| Linux locale corrente | 3.10.3 | verificata | verificato | verificato | verificato |
| Linux evidenza storica | 3.6.2 | verificata | verificato | non rieseguito con il nuovo test | no |
| CI Ubuntu | versione pacchetto runner | verificata a ogni run | verificato | verificato | verificato |
| Windows runner | 3.10.3 pinnato | verificata | verificato | verificato | verificato |
| macOS runner | nessun GDAL nativo | publish core soltanto | non applicabile | non verificato | non verificato |

La voce “versione pacchetto runner” non è sufficiente per una baseline
avionica: prima del freeze qualificato l'immagine GDAL Linux deve essere
immutabile e identificata da digest. La matrice minima residua è:

- GDAL 3.8 LTS-1, 3.10.3 corrente e 3.12 corrente;
- Windows x64 con un pacchetto GDAL/OpenFileGDB identificato e ridistribuibile;
- NTFS e almeno un filesystem Linux qualificato;
- medesimo corpus FileGDB, inclusi crash, projection vuota, CRS/axis, Z/M e
  doppia scrittura semantica.

Finché questa matrice non è verde, FileGDB resta capability opzionale e non va
incluso nel perimetro operativo aeronautico congelato.

## Riesame RC3

Il riesame del 2026-07-29 ha confermato sulla dipendenza pinnata
`gdal 0.17.1` che `OGR_L_SetIgnoredFields` non è esposto da una API safe.
L'accesso diretto tramite `gdal-sys` richiederebbe `unsafe` e violerebbe il
profilo corrente. La decisione e le condizioni di riapertura sono registrate
in `CHANGE_IMPACT_2026-07-29_RC3_FILEGDB_PUSHDOWN.md`; lo stato resta
`design_constraint_open`, senza claim di pushdown nativo.

Il 2026-07-30 è stato costruito e verificato anche un candidato
`gdal 0.19.0` con GDAL/OpenFileGDB 3.12.1 bundled. Build, round-trip e safety
lint sono verdi, ma la projection narrow interlacciata ha registrato
74,310 ms con il pin 0.17.1 e 97,918 ms col candidato (+31,77%). Il veto del
5% ha imposto il rollback completo; la matrice operativa continua a riferirsi
a `gdal 0.17.1` e Windows resta aperto.

## Riesame RC4

RC4 soddisfa il prerequisito del fork governato senza cambiare versione della
crate Rust né binding FFI. Sul benchmark interlacciato contro
`f9e098082be087881272c665c0a4768d93c906b2`, il narrow da tre attributi passa
da 86,171 a 53,311 ms (−38,13%); il full passa da 168,039 a 165,394 ms
(−1,57%). Righe e checksum restano identici in tutti i campioni.

I test FileGDB feature-on coprono projection senza geometria, campi non
contigui richiesti in ordine inverso e projection vuota. La convalida di nome
e tipo degli indici selezionati resta precedente alla chiamata di pushdown,
quindi la protezione contro uno schema cambiato fra `open` e reader non viene
indebolita.

La matrice Windows GDAL/OpenFileGDB 3.10.3 è riproducibile tramite 49 pacchetti
verificati per digest. La CI `30550393598`, sul commit
`c87a5801794f7e95833bf20a41dc87d8a3848fbd`, ha superato suite FileGDB
nativa, benchmark narrow col veto del 5%, cross-volume e test workspace. Il
job ha pubblicato l'artefatto machine-readable
`windows-filegdb-narrow-benchmark`; i numeri non vengono duplicati qui senza
averne verificato direttamente il contenuto.
