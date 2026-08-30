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
| ultima qualificata | `7905d8f` |
| revisione misurata | `7905d8f` |
| passi del checkpoint | 73 |
| passi verdi | 73 |
| passi omessi | 0 |
| passi falliti | 0 |
| input di replay | 67 747 |
| target di replay | 15 |
| crash di replay | 0 |
| target di smoke eseguiti | 15 |
| target di smoke totali | 15 |
| finding di smoke | 0 |
| target in quarantena | 0 |
| copertura LCOV | 87,62% |
| righe coperte LCOV | 34 489 |
| righe strumentate LCOV | 39 361 |
| copertura cargo | 85,90% |
| soglia di copertura | 80,00% |
| baseline differenziale | `87a324f` |
| esito differenziale | 95.06% |
| gruppi ASSURANCE-N1 | 50 |
| gruppi ASSURANCE-N1 aperti | 27 |
| blocchi | 4 |
| S9, qualificato su | `7905d8f` |
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
| `release.candidate-non-valida-per-head` | la candidate pendente non descrive HEAD |
| `sistema.qualifica-cross-component` | gate di sistema non superato, di proprietà esterna |
| `distribuzione.artefatti-qualificati` | artefatti di distribuzione non prodotti ne' qualificati |

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

La diagnostica differenziale misura **le sole righe cambiate** fra la baseline
del blocco generato e la revisione misurata. Non è la copertura del componente,
che è l'altra cifra: qui un 100% è un'affermazione su dodici righe, ed è forte
proprio perché è stretta — sono le righe che questo giro ha scritto, e sono
tutte esercitate.

Il resto del delta è fatto di script Python e shell, JSON di registro e
Markdown, e nessuno di questi entra in quella misura. Dire «nessuna riga
eseguibile è cambiata» sarebbe falso, perché gli script cambiati sono
eseguibili. Semplicemente, la copertura non li osserva.

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

**Riproducibilità della misura di profondità.** L'artefatto di profondità
registrava il `conteggio` delle esecuzioni per requisito, e quel numero non era
riproducibile: su quattro corse di `scripts/fuzz-profondita.sh shp_reader`, due
delle quali su albero bit-identico, otto requisiti su trentacinque alternavano
fra **due** stati. Nessun verdetto ne dipendeva — il gate ne pretendeva il solo
segno — ma il rumore entrava in un file versionato a ogni rimisura,
indistinguibile da un fatto.

Lo schema **2** dell'artefatto registra `"raggiunto": true`, che il generatore
deriva da `conteggio > 0`: la stessa affermazione senza la parte instabile.
Cancellare il conteggio e basta non sarebbe bastato, perché il suo segno *era*
la prova di raggiungimento. Il gate pretende un booleano vero e rifiuta il campo
assente, `false`, un numero al suo posto e un'osservazione che porti ancora
`conteggio`; la versione dello schema è pretesa **esatta**, perché una misura di
schema 1 risponde a un'altra domanda. Resta tutto ciò che era stabile: identità,
famiglia, riga, simboli, input del corpus, impronta del perimetro.

La proprietà è **dimostrata e non dichiarata**: i quattro artefatti in albero
sono stati rimisurati con il generatore nuovo e coincidono byte per byte con la
conversione dei precedenti.

Allo stesso giro appartiene la selezione del binario strumentato, che si fermava
al **primo** candidato restituito da `find` per cui `llvm-cov export` riusciva —
un ordine che dipende dal filesystem, e un arresto che rendeva invisibile per
costruzione l'esistenza di un secondo binario compatibile e diverso. Le quattro
condizioni che la chiudono — enumerazione ordinata, verifica di **tutti** i
candidati, fallimento se i compatibili non sono byte-identici, scelta canonica
soltanto fra copie identiche — vivono in
[`scripts/seleziona_binario_strumentato.py`](../scripts/seleziona_binario_strumentato.py)
e non più nello shell script, perché in shell nessuna delle quattro si sarebbe
potuta violare in una prova.

**Le clausole di comportamento del contratto.** La ratifica di
`wire.loss-report` aveva trovato quattro clausole di
[`cli-protocol-v2.json`](../release/cli-protocol-v2.json) che descrivevano un
codice cambiato sotto di loro. L'invariante era `verified` lo stesso, e per una
ragione precisa: gli undici **numeri** del manifesto sono confrontati con il
codice, mentre la prosa non la guarda nessuno.

Un gate generale prosa-contro-comportamento non è realistico. Tre clausole però
si strutturano, e sono ora campi confrontati come i numeri:

| campo del manifesto | ricavato da |
|---|---|
| `determinismo.ordine_canonico.ragioni` e `.esempi` | i campi che `FidelityReason::chiave()` e `LossExample::chiave()` compongono |
| `troncamento.identita_delle_respinte.ragioni` e `.esempi` | il tipo dell'elemento dei due `BTreeSet` che conservano le respinte |
| `troncamento.omesse_per_byte.fonti` | i siti che incrementano il contatore, ciascuno dei quali deve ricadere in una delle due fonti e in una sola |

Il gate **ricava** il comportamento dal codice invece di confrontare due copie
del manifesto, che divergerebbero insieme. Ciò che sa da sé è *dove* guardare —
il tipo, il campo, il nome della funzione — e se quel posto sparisce diventa
rosso; se cambia ciò che c'è dentro, diventa rosso il confronto col manifesto.
È rosso anche su un campo ripetuto, su una clausola tornata prosa, su un campo
in più che nessuno confronta, su un tipo di respinta che non sa nominare e su un
sito del contatore ambiguo.

**Leggere il codice non basta: bisogna leggerlo abbastanza stretto.** Il gate
ha attraversato **quattro giri di revisione** prima della ratifica, e le otto
vie di falso verde trovate colpivano tutte l'affermazione che l'invariante fa —
non un dettaglio attorno. Hanno **due radici sole**, e ogni giro ne ha chiusa
una su una parte mentre il successivo la trovava sull'altra.

**Cercava su testo che non è codice.** Un corpo canonico scritto dentro un
commento o dentro un letterale veniva trovato dall'espressione regolare prima
di quello vero, che non veniva mai guardato. I commenti si toglievano dopo aver
cercato e solo quelli di riga; poi le stringhe non si mascheravano affatto; poi
si mascheravano le stringhe ordinarie e non le *raw string*, di cui lo scanner
non conosceva la sintassi — e lì sbagliava in due modi insieme, perché `r#""`
gli sembra una stringa aperta e subito chiusa (espone come codice ciò che
segue) mentre il terminatore `"#` gli sembra una stringa nuova (maschera il
codice vero che viene dopo). Quale dei due effetti prevalga lo decideva la
parità delle virgolette, cioè niente che avesse a che fare con il codice.

Ora il sorgente si ripulisce **prima di cercare**, in un passo solo: via i
commenti, di riga e a blocco annidati, e via il contenuto dei letterali —
stringhe, byte string, raw string con qualunque numero di cancelletti, e
caratteri, perché `'"'` esiste e una virgoletta dentro un carattere aprirebbe
una stringa che non c'è. Su una forma che lo scanner non sa leggere, o su un
letterale non terminato, si **fallisce chiusi**: ciò che non si sa mascherare
non lo si sa nemmeno interpretare.

**Ammetteva per segno ciò che va ammesso per forma.** Un segno ammette tutto
ciò che comincia con quel segno, e ogni ammissione troppo larga ammetteva una
scrittura:

| forma | perché passava |
|---|---|
| `self.chiave()` cercato come sottostringa | ordine invertito, criterio in più dopo la chiave, menzione in un commento |
| `troncamento.omesse_per_byte += …` | il censimento cercava il solo `=` |
| `troncamento.omesse_per_byte.clone_from(&nuova)` | il `.` ammetteva ogni metodo, e questo scrive per auto-borrow |
| `scrivi!(troncamento.omesse_per_byte, nuova)` | la `,` ammetteva ogni chiamata, e una macro assegna ciò che riceve |
| `assert_eq!(troncamento.omesse_per_byte.clone_from(&nuova), ())` | la chiamata ammessa autorizzava tutta l'espressione racchiusa |

Ora la forma dei corpi delegati è **esatta** — e `PartialOrd` è verificato con
gli altri due, perché `<` e `>` passano da lì; una chiamata di metodo si
ammette per **nome intero**; un'assegnazione che compare dopo il contatore
nella stessa istruzione lo rende scritto anche quando non lo segue un `=`; e
dentro una chiamata servono **due condizioni insieme** — che la chiamata sia
`assert_eq!`, `assert_ne!` o `assert!`, **e** che il contatore le sia passato
come argomento intero o consumato come valore. Un'asserzione ammette che il
contatore le sia passato, non che dentro di lei gli si faccia qualunque cosa.

Su ciò che resta si fallisce **chiusi**: distinguere una lettura da una
scrittura senza un parser non si può fare per esaustione, e distinguere una
macro che legge da una che scrive vorrebbe l'espansione, non i token. La sola
alternativa onesta a un censimento incompleto è il rosso, e una forma legittima
nuova si aggiunge all'elenco deliberatamente — è precisamente ciò che deve
costare a chi tocca il contatore.

Ciascuna via ha la propria sonda negativa, e così i versi opposti, che pesano
quanto le altre: un commento legittimo dentro la forma canonica, un `=` dentro
una stringa di formato, una stringa che si limita a nominare il contatore, le
forme di letterale conosciute e le tre asserzioni devono restare **verdi**. Una
stretta che si paga in rossi che nessuno sa leggere non è una stretta.

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

### 2. Ratifica e implementazione di `LossReport` — **chiuso**

**Criterio di uscita, soddisfatto.** Le cinque decisioni sono ratificate e
implementate: struttura delle categorie, limiti — cardinalità, byte per stringa,
byte totali —, politica di redazione, comportamento deterministico al limite,
versionamento della busta.

La superficie era già sul wire, quindi la scelta è stata un cambio di contratto
e ha richiesto una versione nuova: il **protocollo 2**, con il v1 congelato e
selezionabile solo da un'opzione che dice nel nome che cosa si sceglie.

**Blocco rimosso.** L'ultima superficie pubblica senza contratto ratificato ne
ha uno, e `wire.loss-report` è `verified` nel registro. Vedi [PRODUCT.md § LossReport](PRODUCT.md#lossreport--ratificato-con-il-protocollo-2).

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

### 5. Artefatti di distribuzione

**Criterio di uscita.** Sei condizioni separabili, e nessuna implica le altre:
artefatti riproducibili per le piattaforme dichiarate, prodotti dalla revisione
qualificata; la variante `gdal-backend` con il proprio runtime GDAL **fissato**,
perché un artefatto che carica una libgdal qualunque non ha un'identità stabile;
checksum e SBOM pubblicati accanto; una provenance che leghi artefatto,
revisione e costruzione, verificabile da chi riceve; uno smoke test
sull'artefatto **installato**, perché un test che gira nell'albero di build non
prova che il pacchetto funzioni; un runbook di installazione, aggiornamento,
rollback e recovery.

**Blocco rimosso.** `distribuzione.artefatti-qualificati`.

**Perché è un blocco e non un desiderio.** Era previsto e non contrattuale: la
prosa lo nominava, il registro no. Un obbligo che vive solo nella prosa non
ferma nessuno, e sarebbe stato possibile azzerare il debito di copertura
negativa, chiudere la candidate e trovarsi autorizzati a rilasciare **senza
avere niente da rilasciare**.

**Costo.** Ignoto e non stimato qui. Ciò che si sa è che la CI oggi costruisce e
prova il codice ma non produce nulla che qualcuno possa installare, quindi non
esiste un oggetto di cui verificare identità e contenuto — e finché non esiste,
il gate che lo qualificherebbe non si può scrivere.

### 6. Decisione finale di rilascio

**Criterio di uscita.** Tutti i punti precedenti chiusi;
`check_release_contract.py --release` verde, cioè nessun invariante
`release_blocking`; un checkpoint di livello 2 su un albero pulito, con SHA e
impronta invariati; l'evidenza in un commit separato.

Solo allora `release_authorized` può diventare `true`, e sarà una decisione
scritta — non la conseguenza automatica di sei caselle verdi.
