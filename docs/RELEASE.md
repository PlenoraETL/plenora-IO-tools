# Rilascio — dove siamo e dove andiamo

Lo stato di questo documento non è scritto a mano: il blocco qui sotto è
**generato** da [`assurance/current-state.json`](../assurance/current-state.json)
e dal [registro del contratto corrente](../assurance/registries/release-contract-current.json),
e `check_docset` lo confronta carattere per carattere. Due verità manuali
divergono, e divergono in silenzio.

Si rigenera con `python3 scripts/check_docset.py --riscrivi-stato`.

---

## Stato

<!-- generato da assurance/current-state.json: inizio -->

> Questo blocco è **generato**. La sua autorità è
> [`assurance/current-state.json`](../assurance/current-state.json); modificarlo
> a mano crea la seconda verità che esiste per impedire.

| Campo | Valore |
|---|---|
| baseline documentale | `2fe9b54` |
| ultima qualificata | `cdd9f72` |
| revisione misurata | `cdd9f72` |
| passi del checkpoint | 63 |
| passi verdi | 63 |
| passi omessi | 0 |
| passi falliti | 0 |
| input di replay | 51 268 |
| target di replay | 15 |
| crash di replay | 0 |
| target di smoke eseguiti | 15 |
| target di smoke totali | 15 |
| finding di smoke | 0 |
| target in quarantena | 0 |
| copertura LCOV | 86,83% |
| righe coperte LCOV | 31 541 |
| righe strumentate LCOV | 36 327 |
| copertura cargo | 85,01% |
| soglia di copertura | 80,00% |
| baseline differenziale | `1a51044` |
| esito differenziale | 95.04% |
| gruppi ASSURANCE-N1 | 49 |
| gruppi ASSURANCE-N1 aperti | 43 |
| blocchi | 5 |
| S9, qualificato su | `cdd9f72` |
| candidate, versione del manifesto | `1.0.1` |
| candidate, revisione del manifesto | `966005d6` |
| candidate, versione del workspace | `1.0.1` |
| candidate, qualifica di HEAD | no |
| candidate, tag previsto | `v1.0.1` |
| candidate, tag creato | sì |
| candidate, revisione del tag | `c490f82` |
| candidate, tag su HEAD | no |
| candidate, release_action consentita | no |
| release_authorized | `false` |

I blocchi sono l'elenco esatto dei `release_blocking` del
[registro del contratto corrente](../assurance/registries/release-contract-current.json)
— non un riassunto:

| Blocco | Sintesi |
|---|---|
| `copertura.rami-negativi` | rami d'errore negativi non tutti verificati da un test eseguito |
| `wire.loss-report` | contratto non ratificato |
| `release.candidate-non-valida-per-head` | la candidate pendente non descrive HEAD |
| `lotto.s10` | validazione completa di GeoParquet 1.1 non aperta |
| `sistema.qualifica-cross-component` | gate di sistema non superato, di proprietà esterna |

<!-- generato da assurance/current-state.json: fine -->

### Che cosa quei numeri non dicono

**Lo SHA misurato non è il commit che ne pubblica l'evidenza.** Un'evidenza sta
in un commit successivo e non eredita la misura: i numeri valgono per l'albero
misurato e per nessun altro.

Le due percentuali di copertura sono **due proiezioni dello stesso profdata**,
non due misure della stessa cosa: contano insiemi diversi di righe strumentate.
Entrambe sono richieste, e nessuna sostituisce l'altra.

Il blocco riporta anche il **conteggio** delle righe, non solo la percentuale:
due decimali arrotondano, e a questa scala una singola riga può spostare la
cifra che si legge. Il conteggio dice di quanto; la percentuale dice quanto,
arrotondato.

Un confronto fra corse diverse non sta qui, e non sta in nessun documento:
l'albero conserva **la sola evidenza corrente**, quindi un'affermazione su più
corse non sarebbe ricostruibile da ciò che il repository contiene. Le corse
precedenti sono in git.

Quella percentuale **si muoveva** fra corse su codice Rust invariato, ed era un
blocco di rilascio. Non era rumore di misura: alcuni rami si eseguono solo
quando una corsa fra thread va in un certo modo, e sono ora esercitati
deterministicamente da sonde dedicate. Il conteggio delle **esecuzioni** per
riga resta variabile, e non entra in alcuna soglia.

Il conteggio dei passi è **riconciliato dagli identificatori** — distinti, senza
duplicati — e non accettato dal rapporto che lo strumento stampa su se stesso.

La diagnostica differenziale è **n/d**, e la baseline contro cui è stata
misurata è nel blocco generato: la prosa ne nominava un'altra, perché era
scritta a mano. Nessuna **riga Rust** si è mossa fra le due revisioni, né
eseguibile né commento. Il resto del delta è fatto di script Python e shell,
JSON di registro e Markdown — nessuno dei quali entra in quella misura.

Non è una misura mancata, ed è la ragione per cui la formulazione conta: dire
«nessuna riga eseguibile è cambiata» sarebbe falso, perché gli script cambiati
sono eseguibili. Semplicemente, la copertura non li osserva.

### Chiuso

**S9 — errori strutturati.** Il censimento dei costruttori che accettano testo
libero è a **zero** su quattordici componenti: produzione, test, doctest e
target di fuzzing. I costruttori non esistono più, quindi la garanzia è
l'assenza della funzione e non una convenzione sorvegliata.

La revisione su cui la chiusura è qualificata è nel blocco generato: è quella
della corsa di livello 2 registrata come ultima misura, e l'evidenza di quella
corsa contiene il passo che misura il censimento. Una qualifica è una corsa che
esiste, non una revisione che qualcuno ricorda.

**Riproducibilità della misura di copertura.** La copertura misurata su un
sorgente Rust strumentato invariato oscillava fra corse. La causa non era rumore
di misura: tre famiglie di rami si eseguono solo quando una corsa fra thread va
in un certo modo — i bracci `Err` dei cicli compare-exchange, la backpressure su
canale pieno, un osservatore che parte prima del produttore. Sono ora esercitati
**deterministicamente**, e sette campagne sulla stessa revisione coincidono riga
per riga in entrambe le modalità di esecuzione.

Resta variabile il numero di **esecuzioni** per riga, che non entra in alcuna
soglia: renderlo deterministico significherebbe serializzare ciò che il codice fa
in parallelo per compiacere una misura. Il verbale è in
[`assurance/campagne-copertura.json`](../assurance/campagne-copertura.json).

**Fuzzing del reader `.shp`/`.dbf`.** `shp_wkb` converte fra WKB e forme ESRI
in memoria; non apre un file, non legge un header, non interpreta una tabella
`.dbf`. Contarlo come copertura del formato sarebbe stato falso, ed è la ragione
per cui il blocco è rimasto aperto mentre `shp_wkb` esisteva.

Il target `shp_reader` legge il formato. Che lo legga davvero **non** è dedotto
dall'assenza di crash — un bundle rifiutato all'apertura non fa crashare niente,
ed è indistinguibile da uno letto per intero: lo dice una misura di copertura del
replay deterministico, verificata da `scripts/check_profondita_fuzz.py`
contro i requisiti di
[`assurance/registries/profondita-fuzz-shapefile.json`](../assurance/registries/profondita-fuzz-shapefile.json).
Sono raggiunti l'apertura del driver, l'inferenza dello schema, l'intestazione
`.shp` e quella di record, l'indice `.shx`, intestazione, descrittori e valori
del `.dbf`, un punto e una polilinea decodificati, il drenaggio in batch e i
**due** rami di rifiuto dei conteggi disallineati — quello all'apertura, che
l'indice rende possibile, e quello che emerge a lettura avviata quando l'indice
non c'è.

La misura porta l'impronta del perimetro che la determina, quindi invecchia
quando quel codice cambia invece che mai.

**Il target ha trovato difetti veri fin dalla prima campagna**, che è la
ragione per cui esisteva il blocco. Entrambe le librerie del formato trattano i
valori dichiarati nel file come se li avessero scritti loro:

| Dove | Che cosa dichiara il file | Che cosa succede |
|---|---|---|
| `dbase` | offset del primo record | due sottrazioni non controllate — una in più per i file dichiarati Visual FoxPro, sui 263 byte di backlink |
| `dbase` | terminatore dei descrittori | preteso con un `debug_assert_eq!` |
| `dbase` | larghezza di un campo | i tipi a dimensione fissa vengono affettati senza verificarla |
| `dbase` | valore di un campo data | affettato per indice di byte, senza guardare lunghezza né confini di carattere |
| `dbase` | valore di un campo data-e-ora | il giorno giuliano entra in un'aritmetica `i32` che trabocca; un parola-tempo negativo trabocca passando da `u32` |
| `shapefile` | scostamenti dell'indice `.shx` | raddoppiati dentro un `i32`; e una voce può puntare in mezzo a un record, dove otto byte qualunque diventano una testa |
| `shapefile` | conteggio di parti e punti di un record | prenota i vettori **prima** di leggere, e non è legato alla dimensione del record |
| `shapefile` | indice delle parti | la differenza fra due voci diventa un numero di punti da leggere, anche quando è negativa |

Gli esiti sono tre, e nessuno è un errore: un panico, un'asserzione di debug, o
una richiesta di memoria che il processo non sopravvive — una campagna ha
chiesto **4,3 GB** per un file da trecento byte. Sotto `libfuzzer-sys` il panico
è un `abort()` che nessun `catch_unwind` vede. I due casi con `debug_assert!`
sono peggiori in **release**, dove l'asserzione sparisce e resta il numero
sbagliato.

`driver-shp` faceva già alcuni di questi controlli, ma **dopo** aver costruito
il reader: il panico arrivava prima. La prevalidazione è ora una coppia di
funzioni a sé, e `scripts/check_prevalidazione_decoder.py` pretende che preceda
ogni costruzione di `ShapeReader` e di `dbase::Reader`, con la stessa regola di
presenza, esclusività e ordine già in vigore per `arrow-ipc` e `parquet`. Ogni
input che ha prodotto un finding è un seme versionato, quindi il replay lo
rigioca a ogni corsa.

La verifica strutturale non pretende di essere il decoder: un file che la passa
può ancora essere rifiutato dal parsing, ed è giusto così. Garantisce che il
rifiuto sia un `Err`. Il costo è una lettura in più dell'intestazione e della
catena dei record; la scansione dei valori tocca solo i campi data, che sono
l'unico tipo il cui **contenuto** può fermare il lettore.

**Spike FileGDB.** Dei due esiti ammessi — target reale, oppure impossibilità
tecnica dimostrata — l'esito è il **primo**. `filegdb_reader` attraversa
l'entry point con `gdal-backend`, il catalogo, lo schema e le righe; dodici
requisiti di profondità sono raggiunti dai soli semi versionati.

Un FileGDB non è un file ma una **directory** di tabelle che si citano per
GUID, e il formato è proprietario: costruirne uno da un blob significherebbe
riscrivere `OpenFileGDB` e produrre file validi rispetto alla nostra idea del
formato invece che a quella di GDAL. Il target parte perciò da una fixture
**vera**, prodotta da `ogr2ogr` da un GeoJSON versionato, e ne sostituisce una
parte per volta. La riproducibilità della fixture non è dichiarata ma
**dimostrata**: rigenerandola due volte, gli unici byte che cambiano sono i tre
GUID che GDAL conia per il dataset, e la tolleranza del gate è esattamente
quell'insieme — un byte stabile e diverso è rosso.

Il limite va detto con la stessa precisione del risultato, ed è misurato in
[`assurance/asan-filegdb.json`](../assurance/asan-filegdb.json): `libgdal.so` è
di sistema e **non strumentata**. Un solo modulo porta contatori, zero file
C/C++ compaiono nella copertura. AddressSanitizer copre per intero il nostro
codice e mantiene l'intercettazione dell'allocatore al confine; **non** rileva
gli accessi interni a GDAL, e il fuzzer non è guidato da ciò che accade oltre.
Una campagna verde dice che il percorso Rust regge input ostili e che GDAL non è
stato portato a un crash osservabile — non che GDAL sia stato esplorato.

`fuzz.filegdb-confine-asan` tiene quel confine **gated**: se un giorno GDAL
fosse costruita con la strumentazione, il gate diventerebbe rosso e la prosa
dovrebbe cambiare con il fatto, invece di sopravvivergli.

### La candidate `1.0.1` non qualifica HEAD

Il manifesto di candidate è legato a una revisione che non è HEAD, con
`release_action.allowed` non consentita. Il tag `v1.0.1` **esiste** e punta a
un commit che non è HEAD — i valori esatti sono nel blocco generato, dove
vengono riletti da `Cargo.toml` e da git a ogni corsa del contratto.

**Quel manifesto non qualifica il codice corrente**, e aggiornarne lo SHA
fingendo che lo faccia sarebbe una qualifica fabbricata. Serve una candidate
nuova, ratificata su versione e tag correnti, oppure la dichiarazione esplicita
che la 1.0.1 è superata.

### Le condizioni sono congiunte

Le condizioni dell'autorizzazione sono **quelle dichiarate** in
[`autorizzazione_di_release`](../assurance/registries/release-contract-current.json),
e `check_release_contract.py --release` le esegue tutte: sono congiunte, nessuna
implica le altre, e un verde parziale non è un verde. Riscriverle qui creerebbe
la seconda rappresentazione che il registro esiste per evitare — e una che
resterebbe a cinque voci il giorno in cui ne nascesse una sesta.

Una sola merita di essere richiamata, perché non è un esito che un gate possa
derivare: `release_authorized` è una **decisione scritta**, non la conseguenza
automatica di caselle verdi.

---

## Roadmap

L'ordine è quello di lavoro, non di importanza. Ogni punto dichiara che cosa
serve per uscirne e quale blocco rimuove.

Nessuna stima temporale è presentata come impegno. Ciò che si sa del costo è
scritto dove è stato misurato.

### 1. Chiusura dei gruppi ASSURANCE-N1 ancora aperti

**Criterio di uscita.** Ogni gruppo del registro è `chiuso`, con una prova che è
un **test eseguito**, oppure `irraggiungibile` con le righe scoperte e la
guardia che rifiuta per prima. `check_assurance_n1.py --release` diventa verde.

**Blocco rimosso.** I rami d'errore negativi smettono di essere non verificati.

**Costo.** Il costo dominante non è scrivere i test ma **determinare quali rami
siano raggiungibili**: in un gruppo su tre affrontati finora, un solo ramo su
tre lo era. Quella determinazione non si parallelizza e non si fa leggendo i
commenti.

### 2. Ratifica e implementazione di `LossReport`

**Criterio di uscita.** Le cinque decisioni sono ratificate e implementate:
struttura delle categorie, limiti — cardinalità, byte per stringa, byte totali —,
politica di redazione, comportamento deterministico al limite, versionamento
della busta.

La superficie è già sul wire, quindi qualunque scelta è un cambio di contratto e
richiede una nuova versione.

**Blocco rimosso.** L'ultima superficie pubblica senza contratto ratificato ne
acquista uno. Vedi [PRODUCT.md § LossReport](PRODUCT.md#lossreport--non-ratificato).

### 3. S10, S11, S12

| Lotto | Perimetro |
|---|---|
| **S10** | validazione completa di GeoParquet 1.1 |
| **S11** | `wkb_shape` ispeziona i figli delle collection |
| **S12** | parsing bounded di WKT e GeoJSON, fuzz dedicato, capability `hostile_input_hardened` |

**Criterio di uscita.** Ciascun lotto chiuso con il proprio checkpoint di
livello 2 e la propria evidenza.

**Blocco rimosso.** Il perimetro del componente è completo. S12 in particolare
rimuove l'ultima asimmetria fra i formati: oggi WKT e GeoJSON hanno tetti, ma
non una capability dichiarata che li renda verificabili dall'esterno.

### 4. Qualifica cross-component

**Criterio di uscita.** La catena `IO-tools → data-tools → database-tools` è
qualificata in **entrambe le direzioni**, su fixture con revisioni, piattaforma,
comandi ed esiti registrati.

Il perimetro e l'harness sono di **proprietà esterna**: questo repository non
contiene né esegue test che compilino gli altri due componenti. La definizione
è in [`release/system-rc-gate.json`](../release/system-rc-gate.json).

**Blocco rimosso.** La readiness di sistema smette di essere non verificata.
Resta distinta dalla readiness del componente: nessuna delle due implica
l'altra.

### 5. Decisione finale di rilascio

**Criterio di uscita.** Tutti i punti precedenti chiusi;
`check_release_contract.py --release` verde, cioè nessun invariante
`release_blocking`; un checkpoint di livello 2 su un albero pulito, con SHA e
impronta invariati; l'evidenza in un commit separato.

Solo allora `release_authorized` può diventare `true`, e sarà una decisione
scritta — non la conseguenza automatica di sei caselle verdi.
