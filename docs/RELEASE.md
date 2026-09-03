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
| ultima qualificata | `985e3ee` |
| revisione misurata | `985e3ee` |
| passi del checkpoint | 80 |
| passi verdi | 80 |
| passi omessi | 0 |
| passi falliti | 0 |
| input di replay | 77 231 |
| target di replay | 15 |
| crash di replay | 0 |
| target di smoke eseguiti | 15 |
| target di smoke totali | 15 |
| finding di smoke | 0 |
| target in quarantena | 0 |
| copertura LCOV | 88,91% |
| righe coperte LCOV | 37 808 |
| righe strumentate LCOV | 42 524 |
| copertura cargo | 87,42% |
| soglia di copertura | 80,00% |
| baseline differenziale | `17875e7` |
| esito differenziale | 78.64% |
| gruppi ASSURANCE-N1 | 50 |
| gruppi ASSURANCE-N1 aperti | 0 |
| blocchi | 2 |
| capacità differite | 1 |
| S9, qualificato su | `985e3ee` |
| candidate, versione del manifesto | `1.0.1` |
| candidate, revisione congelata | `966005d6` |
| candidate, versione del workspace | `2.0.0` |
| candidate, artefatti congelati | 0 |
| candidate, tag previsto | `v1.0.1` |
| candidate, tag creato | sì |
| candidate, revisione del tag | `c490f82` |
| candidate, tag sulla candidate | no |
| candidate, assurance entro l'allowlist | no |
| candidate, release_action consentita | no |
| release_authorized | `false` |

I blocchi sono l'elenco esatto dei `release_blocking` del
[registro del contratto corrente](../assurance/registries/release-contract-current.json)
— non un riassunto:

| Blocco | Sintesi |
|---|---|
| `release.candidate-non-valida-per-head` | la candidate pendente non descrive HEAD |
| `distribuzione.artefatti-qualificati` | artefatti prodotti e verificati in prova; qualifica sullo SHA congelato assente |

Le capacità **differite** non sono blocchi chiusi: non sono richieste
da questa release e **non sono verificate**. Ciascuna dichiara che cosa
la release non promette, ed è la sola lettura autorizzata del rinvio:

| Capacità | Sintesi | La release non promette |
|---|---|---|
| `sistema.qualifica-cross-component` | differita: la catena a tre componenti non e' qualificata, e la 2.0.0 non la promette | la 2.0.0 NON promette interoperabilita' end-to-end certificata con plenora-data-tools e plenora-database-tools. La catena IO-tools -> data-tools -> database-tools non e' qualificata in nessuna delle due direzioni, su nessuna piattaforma; nessuna delle quindici proprieta' del contratto di sistema -- fra cui srid, crs_resolution, axis_order e native_metadata -- e' verificata attraverso i tre componenti; e la direzione database -> data -> IO non e' mai stata eseguita. Chi compone i tre componenti in produzione lo fa senza evidenza di conservazione dei metadati ai confini, e deve verificarla per conto proprio. |

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
nuova, ratificata su versione e tag correnti.

**Il workspace è ora a `2.0.0`, e la 1.0.1 è superata.** Il major non è una
formalità: l'insieme dei file accettati si è ristretto. Un `.dbf` con una data
malformata in una riga cancellata veniva letto saltando quella riga e ora è
rifiutato; un `.dxf` con un `BLOCK` che non arriva a `ENDBLK` teneva il lettore
occupato senza fine e ora è rifiutato. Lo schema delle sei buste non cambia — chi
riceve legge lo stesso JSON — ma chi aveva una pipeline che passava uno di quei
file vede una busta d'errore dove prima ne vedeva una di successo, e questo si
dichiara con un numero invece che con una nota che qualcuno deve leggere.

L'alternativa precedente non era «leggere di più»: nel primo caso era un panico,
nel secondo un processo che non termina. Il verso in cui si è cambiato è quello
in cui si sbaglia meglio, e resta un cambiamento osservabile.

Il divario fra `versione_manifesto` — ancora `1.0.1`, la candidate pendente — e
`versione_workspace` è visibile nel blocco generato. Non è un'incoerenza da
appianare allineando i campi: **è il blocco**, scritto in due numeri, e lo chiude
una candidate nuova sullo SHA congelato.

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
serve per uscirne e quale blocco rimuove **quando sarà chiuso**. Un punto chiuso
lo dice nel proprio titolo; per tutti gli altri il blocco è ancora in piedi.

Un punto può anche essere **differito**, e lo dice nel titolo come gli altri.
Differito non è chiuso: la capacità non è richiesta da questa release e **non è
verificata**, e il punto deve dichiarare che cosa la release smette di
promettere. È lo stesso participio del campo qui sotto, e la stessa cautela:
«differito» letto come «risolto» sarebbe l'errore che questo paragrafo esiste
per impedire.

Il campo si chiamava «Blocco rimosso», e al participio passato si legge come
fatto: una revisione l'ha letto così sul punto 5 e ne ha concluso che il blocco
della distribuzione fosse chiuso, mentre il registro lo teneva — correttamente —
aperto. Due letture possibili della stessa riga sono una riga da riscrivere.

Nessuna stima temporale è presentata come impegno. Ciò che si sa del costo è
scritto dove è stato misurato.

### 1. Chiusura dei gruppi ASSURANCE-N1 ancora aperti

**Criterio di uscita.** Ogni gruppo del registro è `chiuso`, con una prova che è
un **test eseguito**, oppure `irraggiungibile` con le righe scoperte e la
guardia che rifiuta per prima. `check_assurance_n1.py --release` diventa verde.

**Blocco che rimuove.** I rami d'errore negativi smettono di essere non verificati.

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

**Blocco che rimuove.** L'ultima superficie pubblica senza contratto ratificato ne
ha uno, e `wire.loss-report` è `verified` nel registro. Vedi [PRODUCT.md § LossReport](PRODUCT.md#lossreport--ratificato-con-il-protocollo-2).

### 3. S10, S11, S12

| Lotto | Perimetro |
|---|---|
| **S10** | validazione completa di GeoParquet 1.1 |
| **S11** | `wkb_shape` ispeziona i figli delle collection |
| **S12** | parsing bounded di WKT e GeoJSON, fuzz dedicato, capability `hostile_input_hardened` |

**Criterio di uscita.** Ciascun lotto chiuso con il proprio checkpoint di
livello 2 e la propria evidenza.

**Blocco che rimuove.** Il perimetro del componente è completo. S12 in particolare
rimuove l'ultima asimmetria fra i formati: oggi WKT e GeoJSON hanno tetti, ma
non una capability dichiarata che li renda verificabili dall'esterno.

### 4. Qualifica cross-component — **differita nella 2.0.0**

La 2.0.0 rilascia `plenora-IO-tools` come **componente autonomo**. Il perimetro
della release è il confine sui file di questo componente — lettura, scrittura,
protocollo della CLI, artefatti di distribuzione — e non la catena di sistema.

**Che cosa la release non promette.** La 2.0.0 **non** promette
interoperabilità end-to-end certificata con `plenora-data-tools` e
`plenora-database-tools`. La catena `IO-tools → data-tools → database-tools`
non è qualificata in nessuna delle due direzioni, su nessuna piattaforma;
nessuna delle quindici proprietà del contratto di sistema — fra cui `srid`,
`crs_resolution`, `axis_order` e `native_metadata` — è verificata attraverso i
tre componenti; e la direzione `database → data → IO` non è mai stata eseguita.
Chi compone i tre componenti in produzione lo fa **senza evidenza** di
conservazione dei metadati ai confini, e deve verificarla per conto proprio.

Il rinvio è una scelta di perimetro, non un giudizio sull'esito: la qualifica
non è stata tentata e fallita, **non è stata eseguita**.

**Perché è differita e non chiusa.** Il perimetro e l'harness sono di
**proprietà esterna**: questo repository non contiene né esegue test che
compilino gli altri due componenti, e alla data del rinvio l'harness non
esisteva da nessuna parte — `plenora-contracts/conformance` è una specifica,
nessuna delle quattordici fixture richieste esiste, e la revisione dell'ICD
dichiarata nel gate non risolve. Bloccare un componente già verificato in attesa
di una macchina di prova che non esiste avrebbe fermato il rilascio senza
aggiungere una sola verifica.

**Che cosa non cambia.** [`release/system-rc-gate.json`](../release/system-rc-gate.json)
resta `status: not_satisfied` ed `evidence.status: not_run`. Il rinvio non tocca
l'artefatto dell'owner: toglie il blocco, non fabbrica l'evidenza. Il registro
del contratto porta la voce in stato `differita`, che non è `verified` e non lo
diventa cambiando una parola — `verified` continua a pretendere che l'artefatto
dica `passed`, e `differita` esige che **non** lo dica.

**Che cosa la riporta a essere pretesa.** L'owner esterno consegna harness e
fixture, la corsa produce evidenza, e allora `release/system-rc-gate.json` passa
a `status: satisfied`, `evidence.status: passed`, `open_blockers: []` e le tre
revisioni fissate. A quel punto la voce torna `verified` **per derivazione
dall'artefatto**, e la sonda che oggi la tiene onesta diventa quella che
impedisce di lasciarla differita mentre l'evidenza esiste.

**Gli input esatti da consegnare.**

| Input | Stato |
|---|---|
| revisione di `plenora-IO-tools` | lo SHA congelato della 2.0.0 |
| revisione di `plenora-data-tools` | da fissare dall'owner alla corsa |
| revisione di `plenora-database-tools` | da fissare dall'owner alla corsa |
| revisione dell'ICD | da ri-fissare: `v2.0-rc14` non esiste in `plenora-contracts` |
| harness eseguibile della catena | da consegnare |
| 14 fixture × 2 varianti × 2 direzioni × 2 piattaforme | da consegnare |
| bundle di evidenza: comandi, ambiente, hash delle fixture, esiti | da consegnare |

I tre blocchi che l'owner dichiara ancora aperti sono nel gate: la direzione
`database → data → IO` mai eseguita, l'esecuzione nativa Windows della catena
non coperta, e il runner esterno che non consuma `read_loss` e lascia la
dichiarazione R4.6.1 come obbligo non verificato.

**Blocco che rimuove.** Nessuno, finché resta differita. La readiness di sistema
resta **non verificata**, e resta distinta dalla readiness del componente:
nessuna delle due implica l'altra, ed è esattamente per questo che rilasciare
la seconda senza la prima è possibile — purché lo si dica.

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

**Blocco che rimuove.** `distribuzione.artefatti-qualificati`.

**Perché è un blocco e non un desiderio.** Era previsto e non contrattuale: la
prosa lo nominava, il registro no. Un obbligo che vive solo nella prosa non
ferma nessuno, e sarebbe stato possibile azzerare il debito di copertura
negativa, chiudere la candidate e trovarsi autorizzati a rilasciare **senza
avere niente da rilasciare**.

**Costo.** Non più ignoto. Ciò che la CI produceva era codice provato e nessun
oggetto installabile; ora i costruttori esistono per tutte e due le piattaforme,
gli oggetti esistono e sono stati misurati. La release 2.0.0 non richiede una
firma di piattaforma: non esiste quindi materiale di firma esterno da ottenere.
Il gate di sistema resta di proprietà esterna.

**Stato al 2 settembre 2026: la macchina è pronta e provata, la qualifica
finale non c'è.** Sono due cose diverse e vanno lette separatamente.

Ciò che esiste ed è stato eseguito: entrambe le piattaforme costruiscono
entrambi i profili — quattro artefatti — in un workflow che gira su runner
nativi; ciascun job produce referti strutturati; un job `gate` li riconta contro
la matrice e pretende ventiquattro referti, che è il numero che la matrice
implica e non quello che si è ricevuto. L'ultima corsa verde è `d9bc47d`:
runtime, licenze, smoke sull'artefatto installato, relocation da una directory
diversa da quella di costruzione, digest del manifesto ricalcolati sull'albero
estratto, provenance. Gli artefatti sono di **prova** — `non_release: true` nel
manifesto — e il gate rifiuta ovunque si pretenda una candidate.

Ciò che manca, ed è il motivo per cui il blocco resta: nessuna di quelle corse
è girata sullo SHA che verrà congelato. Un artefatto qualificato è prodotto
**dalla revisione qualificata**, e finché quella revisione non esiste la catena
dimostra di funzionare senza dimostrare nulla su ciò che si consegnerà. La
candidate Windows sarà intenzionalmente unsigned e lo dichiarerà nel manifesto;
restano obbligatori upload, checksum, provenance e verifica post-upload dei
deliverable sullo SHA finale.

**Perché la distinzione conta.** «La macchina funziona» e «l'artefatto è
qualificato» si somigliano abbastanza da essere scambiate, e la seconda è
l'unica che il contratto pretende. Il registro tiene il blocco aperto proprio
qui.

**macOS è fuori dal perimetro della v1.** Non è una piattaforma non ancora
costruita: è una decisione di prodotto, registrata sotto
`piattaforme_non_distribuite`. Vedi «Il perimetro» qui sotto.

Quel che segue e' il runbook: descrive **cio' che e' stato eseguito**, e i
numeri sono il referto di una corsa, non un contratto. Il contratto sta nel
lock, e le misure le produce il controllo.

#### Linux x86_64

##### Che cosa contiene l'artefatto

```
plenora-io-<versione>-linux-x86_64-filegdb/
  bin/plenora-io            il binario
  lib/                      le librerie native, per SONAME
  lib/gdalplugins/          i plugin di GDAL (vuota: i driver sono nel core)
  share/gdal/               i dati di GDAL
  share/proj/               le griglie di PROJ
  LICENSES/PROVENIENZA.json quale pacchetto ha messo quale file, e con che licenza
  MANIFEST.json             identita', canale, file spediti, normalizzazioni
  SBOM.spdx.json            i pacchetti che hanno contribuito ai file spediti
```

L'albero e' **spostabile**: nessun percorso assoluto lo lega alla directory in
cui e' stato costruito. Le librerie si risolvono con un `RPATH` radicato in
`$ORIGIN`, e i dati di GDAL e di PROJ li trova il binario stesso, derivandoli
dal proprio percorso (`crates/plenora-io-cli/src/radici.rs`).

##### La base di costruzione

Ubuntu 22.04, che porta glibc 2.35 — la soglia dichiarata nella matrice.
L'immagine di sviluppo e' Debian 12 (glibc 2.36) e **non** va usata: darebbe un
artefatto che non gira su Ubuntu 22.04, per un accidente dell'ambiente di
lavoro invece che per una proprieta' del prodotto.

```sh
docker build -f Dockerfile.build-linux -t plenora-io-build-linux .
```

##### Il runtime GDAL

Non si risolve nulla al momento della costruzione: `scripts/linux-gdal-lock.json`
fissa i pacchetti con URL, dimensione e sha256, e lo strumento che li
materializza -- micromamba -- e' fissato allo stesso modo.

```sh
docker run --rm -v "$PWD":/work -v /percorso/lavoro:/A -w /work \
  plenora-io-build-linux \
  python3 scripts/install-linux-gdal.py --prefisso /A/runtime --lavoro /A/cache
```

Il lock si **rigenera** soltanto quando si cambia versione di GDAL:

```sh
python3 scripts/genera-gdal-lock.py --lavoro /A/nuovo --subdir linux-64
```

Rigenerarlo invalida ogni misura fatta sul precedente. La versione è 3.9 e non
l'ultima disponibile: `gdal-sys 0.10.0` spedisce binding pre-costruiti soltanto
fino a 3.9, e le due alternative sono peggiori.

La feature `bindgen` genera i binding a build time, e `bindgen` non è una
libreria in più: è un **generatore di codice** che va fissato e qualificato
insieme a ciò che gli serve — `libclang`, gli header di GDAL, e la versione di
clang che li interpreta. Sono tre nuovi input di costruzione, ciascuno capace
di cambiare i binding senza che nessuna riga del repository cambi. I binding
pre-generati tolgono quel problema alla radice: sono byte nel crate, già dentro
il perimetro fissato.

Dichiarare a `gdal-sys` una versione diversa da quella spedita compila invece
l'ABI di una serie contro la libreria di un'altra: funziona finché funziona, e
quando smette non lo dice.

##### La costruzione

```sh
docker run --rm -v "$PWD":/work -v /percorso/lavoro:/A -w /work \
  -e CARGO_HOME=/A/cargo plenora-io-build-linux \
  python3 scripts/costruisci-artefatto-linux.py \
    --prefisso /A/runtime --uscita /A/uscita --versione <versione>
```

Il canale predefinito e' `prova`: l'artefatto porta `non_release: true` nel
manifesto, e non e' una candidate. Per una candidate serve `--canale candidate`,
e va fatto sulla revisione definitiva.

Il costruttore fa tre cose che vanno sapute:

- **parte dal binario vero.** La chiusura `DT_NEEDED` si calcola da
  `bin/plenora-io`, non da `libgdal.so`: la domanda e' che cosa serve
  all'artefatto, e non che cosa serve a GDAL.
- **copia per SONAME.** Nel prefisso `lib/libgdal.so.35` e' un symlink al file
  versionato, e `libgdal.so.35` e' il nome che il loader cerchera'. Copiare il
  file risolto e basta produce un albero che contiene la libreria e non la
  trova.
- **normalizza i `DT_NEEDED` assoluti.** `libgdal.so.35` di conda-forge
  dichiara `libsqlite3.so` per percorso assoluto — il placeholder del prefisso,
  che la rilocazione di conda sostituisce — e senza riscriverlo l'artefatto
  smetterebbe di caricarsi appena quella directory non esiste. Le riscritture
  sono elencate in `MANIFEST.json` sotto `normalizzazioni`, perche' modificare
  un binario di terze parti va detto.

##### Le verifiche

**Runtime, sull'albero assemblato.**

```sh
python3 scripts/check-linux-gdal-runtime.py \
  --prefisso /A/uscita/plenora-io-<versione>-linux-x86_64-filegdb \
  --radice bin/plenora-io \
  --prefisso-di-costruzione /A/runtime
```

I due prefissi non sono la stessa cosa, e confonderli e' un falso verde gia'
capitato: `--prefisso` e' dove i file stanno adesso, `--prefisso-di-costruzione`
e' cio' che i binari nominano dentro di se'. Se non se ne trova nessuno, il
controllo diventa rosso invece di dichiarare che non ce ne sono.

**Licenze.**

```sh
python3 scripts/check-licenze-artefatto.py --albero /A/uscita/plenora-io-<versione>-linux-x86_64-filegdb
```

Pretende che ogni componente che mette **byte** nell'artefatto abbia accanto il
testo della propria licenza — non il nome, non l'identificatore: il testo. Un
elenco di licenze non è ciò che una licenza obbliga a distribuire.

Tre pacchetti spediscono byte senza portare il proprio testo: `libgcc`
(`libgcc_s.so.1`), `libstdcxx` (`libstdc++.so.6`) e `libsqlite`
(`libsqlite3.so`). Per quelli il testo viene dall'autorità dell'identificatore
SPDX che dichiarano, fissato nel lock per URL, dimensione e SHA-256 come tutto
il resto. Per `GPL-3.0-only WITH GCC-exception-3.1` sono **due** testi: la
seconda è ciò che rende distribuibile un binario linkato alla prima.

Un componente che spedisce file e non riesce a procurarsi un testo **ferma la
costruzione**. Prima veniva nominato in `PROVENIENZA.json`, il che evitava il
silenzio e non consegnava la licenza. Un pacchetto che non mette file
nell'albero non è un componente e non gli si chiede nulla; che la distinzione
resti il criterio — «ha messo un file qui» — e non diventi un'esenzione lo
verifica una sonda.

**Relocation smoke.**

```sh
bash scripts/relocation-smoke-linux.sh <archivio.tar.gz> <directory-A> [/smoke]
```

Costruisce in A, archivia, **cancella A**, estrae in una B di lunghezza diversa,
esegue da una terza directory senza ambiente conda e senza `LD_LIBRARY_PATH`,
con `GDAL_DATA`, `PROJ_DATA` e `GDAL_DRIVER_PATH` preimpostate a sentinelle
inesistenti. Scrive e rilegge un FileGDB con un CRS, traccia gli accessi ai
file e rifiuta qualunque tocco ad A, e verifica che ogni libreria fuori
dall'allowlist ABI venga da B.

Porta con se' la propria controprova: rinomina `share/gdal` in una copia e
pretende che il comando **fallisca**. Senza, un verde direbbe soltanto che il
comando riesce, non che le sentinelle fossero letali.

Quello che dimostra sono i percorsi **effettivamente attraversati**. I percorsi
TLS, XML, terminfo e Kerberos che non esercita restano governati dalla loro
classificazione strutturale, e non diventano provati perche' lo smoke e' verde.

##### Esito misurato

Su GDAL 3.9.3, con l'artefatto costruito e verificato in locale. Il prefisso di
costruzione lo dichiara il manifesto dell'artefatto, cosi' che il controllo non
debba farselo dire a mano: passarne uno sbagliato non trova nessun percorso
assoluto, e senza la guardia che rende rosso lo zero sembrerebbe un artefatto
pulito.

| misura | valore |
| --- | --- |
| dipendenze interne, dal binario | 56 |
| ELF spediti | 56 |
| dipendenze esterne | 6, esattamente le attese |
| GLIBC massima | 2.34 (soglia 2.35) |
| ELF con `DT_NEEDED` assoluti | 0 |
| percorsi assoluti classificati | 29 su 29 |
| RPATH radicati in `$ORIGIN` e interni | 56 su 56 |
| componenti con il testo di licenza | 43 su 43 |
| di cui con il proprio testo | 40 |
| di cui con il testo canonico dell'identificatore | 3 |
| componenti con la sola dichiarazione | 0, ed e' fatale |
| relocation smoke | verde, con controprova |

I numeri stanno qui come **referto di una corsa**, non come contratto: il
contratto e' nel lock, e le misure le produce il controllo. Se divergono, e'
questa tabella a essere vecchia.

---

#### I due profili, e i quattro artefatti

Due piattaforme per due profili sono **quattro** artefatti, e i due profili sono
due prodotti con due promesse opposte.

`filegdb` promette che FileGDB funzioni, e lo si dimostra scrivendone uno e
rileggendolo. `base` promette che FileGDB **manchi**, ed è la promessa che ci si
dimentica di verificare: un base costruito per sbaglio con la feature attiva
sarebbe più grande di sessanta megabyte, porterebbe un runtime GDAL che il suo
contratto non prevede — con una superficie e una licenza che chi lo installa non
ha accettato — e nulla nel nome lo direbbe.

```sh
python3 scripts/smoke-profilo.py --albero <albero> --lavoro <tmp> --referto <referto.json>
```

Lo smoke legge il profilo dal manifesto e pretende l'esito giusto: per `base`,
che aprire un FileGDB sia rifiutato con categoria `unsupported` e un messaggio
che nomini il tier mancante — un rifiuto per la ragione sbagliata passerebbe un
controllo che guardasse solo il codice d'uscita — e che la conversione senza
GDAL funzioni comunque.

Esito misurato su Linux: il profilo base pesa 8,5 MB contro i 64 del profilo
pieno, e dimostra FileGDB assente.

#### Il gate finale

```sh
python3 scripts/check-distribuzione-completa.py --referti <dir> --canale candidate
```

Riconta. Non chiede a nessuno com'è andata: legge i referti e pretende che ci
siano tutti quelli che devono esserci, che ciascuno porti le misure che quella
verifica deve produrre, e che le misure dicano ciò che il contratto promette.

Serve perché «il job è verde» non è un'evidenza verificabile. Un job verde è
un'affermazione fatta da chi doveva essere verificato, e le affermazioni si
sbagliano in silenzio: un passo saltato per una condizione mai vera, un `||
true` di troppo, uno smoke che non ha trovato l'artefatto e non ha guardato
niente. Nessuna di queste cose fa rosso.

I verificatori restano **nativi e separati** — `DT_NEEDED` e `GLIBC_*` non
esistono su Windows, un `LC_VERSION_MIN_MACOSX` non esiste altrove, e scriverne
uno solo vorrebbe dire verificare il minimo comune ovunque. Comune è la forma
del risultato.

#### Il perimetro

La v1 distribuisce **due** piattaforme:

- Linux x86-64
- Windows x86-64

**macOS, ARM64 incluso, è fuori dal perimetro di distribuzione supportato.** Il
codice può continuare a compilare ed essere provato su macOS — nessuno lo
impedisce — ma non si promettono artefatti, installazione, supporto operativo né
qualifica. È una decisione sullo scope di questa release, non un'affermazione
sul mondo: server macOS esistono, e il prodotto è destinato ai deployment server
su cui la v1 si concentra.

Due piattaforme per due profili fanno **quattro** artefatti.

Il gate finale deriva il perimetro dalla decisione registrata, non dai job che
esistono: un artefatto in meno perché qualcuno ha tolto un job è un buco, un
artefatto in meno perché una piattaforma è fuori scope è una scelta. Nel
conteggio si somigliano e per il resto non si somigliano per niente, e una sonda
pretende che ogni piattaforma conosciuta stia in una delle due liste con una
motivazione scritta. Toglierne una senza dichiararlo ferma il gate.

Con macOS escono anche `scripts/macos-gdal-lock.json`, il verificatore Mach-O e
le sue sonde. Un verificatore che nessun artefatto esercita invecchia senza che
nessuno se ne accorga: quando servisse, gli strumenti Apple e il formato saranno
cambiati, e darebbe l'impressione di un punto di partenza che non è. La storia
git lo conserva.

Riportare macOS nel perimetro è una decisione di prodotto — non un runner che si
libera.

#### La firma

| canale | Linux | Windows |
| --- | --- | --- |
| `prova` | non richiesta | non richiesta |
| `candidate` | non richiesta | non richiesta |

La release 2.0.0 non richiede una firma di piattaforma. Su Linux non esiste un
meccanismo equivalente che il sistema verifichi normalmente all'esecuzione; su
Windows la scelta è deliberata: il PE viene distribuito senza Authenticode.
Windows può quindi mostrare «editore sconosciuto» e una policy aziendale può
impedirne l'esecuzione. SHA-256, digest del manifesto e provenance restano
obbligatori e verificano byte e origine della costruzione, ma non attribuiscono
al binario un editore riconosciuto da Windows.

Il manifesto conserva il blocco `firma` con stato `non_richiesta` su entrambe le
piattaforme e in entrambi i canali. Il campo esplicito distingue questa decisione
da una firma dimenticata. Il workflow non legge PFX, certificati o secret di
firma.

Developer ID, notarizzazione e la questione dello stapling sono usciti insieme a
macOS. Erano il pezzo più costoso della catena — un certificato Apple, un
servizio esterno da interrogare, e una ricevuta che sul nostro deliverable non
si poteva nemmeno attaccare al file.

#### L'ordine delle operazioni

Otto passi, uguali su entrambe le piattaforme distribuite. Ognuno dipende dai byte
prodotti dal precedente, e invertirne due produce un artefatto le cui verifiche
parlano di un file diverso da quello che si consegna.

1. **payload** — assemblare l'albero: binario, librerie, dati, licenze.
2. **firma** — dichiarare esplicitamente che la release non richiede una firma
   di piattaforma; il passo non modifica i byte.
3. **manifesto** — generarlo dai byte finali del payload.
4. **archivio** — creare il contenitore (`tar.gz` su Linux, `zip` altrove).
5. **notarizzazione** — nessuna piattaforma del perimetro la richiede; il passo
   resta perché l'ordine è uno solo e la posizione è ciò che va fissata.
6. **checksum** — sui byte *finali*.
7. **smoke** — sull'oggetto finale, non su una sua versione precedente.
8. **provenance** — legata a *quel* checksum.

Il campo `firma` resta nei manifesti con stato `non_richiesta`. Nessun
certificato o secret è richiesto per costruire una candidate.

#### Windows x86_64 — fissata e verificata in canale prova

Il lock c'è ed è coerente: `scripts/windows-gdal-lock.json`, conda-forge, GDAL
3.9.3 con binding della stessa serie. La catena precedente era OSGeo4W e
dichiarava una libreria 3.10.3 con binding 3.6.0, mentre l'installatore forzava
la versione per farla compilare — si compilava l'ABI di una serie contro la
libreria di un'altra, e nessun gate lo vedeva perché la forzatura mascherava
proprio la condizione che avrebbe fermato la build.

Il costruttore (`scripts/costruisci-artefatto-windows.py`) e il verificatore
nativo (`scripts/check-windows-runtime.py`) sono stati eseguiti su entrambi i
profili. L'ultima distribuzione verde, su `d9bc47d`, ha costruito gli archivi,
ricalcolato i digest sull'albero estratto, verificato import e delay-import,
eseguito lo smoke e il relocation smoke e consegnato al gate tutti i referti
attesi. Erano artefatti di prova e non qualificano la revisione corrente.

##### Come è stato derivato il contratto Windows

L'insieme delle DLL di sistema attese **non è stato scritto a tavolino**:
dipende da come conda-forge ha compilato GDAL per win-64, e l'unico modo di
saperlo era guardare un artefatto vero. Scriverlo a mente avrebbe significato
inventare una soglia e poi verificarla.

**Prima corsa — scoperta.** Ha costruito, misurato e scritto
`windows-runtime-discovery.json`, lo ha caricato come artefatto ed è terminata **rossa**.
Non tocca il lock né il repository. Il rosso non è un difetto trovato: è
l'assenza di una revisione umana. Una corsa di scoperta che potesse diventare
verde da sola scriverebbe il proprio contratto, e un contratto scritto da ciò
che deve verificare non verifica niente.

Il rilievo registra, separatamente per `base` e per `filegdb`: import normali e
ritardati, DLL interne, API-set, DLL esterne, percorsi del prefisso di
costruzione incorporati, architettura di ogni PE — e da dove viene la misura:
runner, sistema, SHA sorgente, digest del lock.

**Fra le due corse.** Ogni dipendenza è stata classificata in una delle quattro
classi:

| classe | significa |
| --- | --- |
| `interna` | spedita dentro l'artefatto |
| `api_set` | nome virtuale che il caricatore risolve |
| `abi_windows` | DLL che il sistema garantisce, nell'insieme atteso |
| `inattesa` | nessuna delle tre — **blocca** |

`inattesa` non significa «probabilmente va bene»: significa che nessuno ha
deciso che cosa sia. Non si ammettono insiemi larghi — `C:\Windows\*`, il
`PATH`, «qualunque DLL Microsoft» — perché un insieme largo non si accorge di
ciò che smette di essere spedito e viene preso dal sistema, che è esattamente
il difetto per cui l'insieme esatto esiste.

Un **commit successivo**, e non la corsa, ha messo nel lock l'insieme esatto per
profilo e il digest del rilievo da cui la decisione viene. Il digest è ciò che
lega il contratto alla misura.

**Seconda corsa.** Ha confrontato il reale con l'atteso ed è diventata verde.
Il relocation smoke Windows costruisce in A, archivia, cancella A, estrae in B,
usa una directory corrente estranea e un ambiente ostile, quindi scrive e
rilegge davvero un FileGDB.

Ogni job carica separatamente due cose: i referti che qualificano l'oggetto e il
deliverable vero — archivio, sidecar `.sha256` e provenance. I referti non sono
un sostituto dell'oggetto: prima di questa separazione gli archivi restavano in
`RUNNER_TEMP` e sparivano insieme al runner. Il job finale riscarica i quattro
deliverable dal servizio artifact e ricalcola i checksum su quei byte, poi li
confronta con sidecar e provenance insieme a revisione, lock, piattaforma,
profilo e canale. Verificare prima dell'upload dimostrerebbe un oggetto diverso
da quello che chi scarica riceve.

#### Installazione, aggiornamento, rollback e recovery

Gli archivi sono installazioni **affiancate**, non aggiornamenti in-place. La
directory estratta è immutabile: modificarne un file invalida manifesto,
checksum e qualifica.

**Installazione.** Scaricare insieme archivio, file `.sha256` e provenance;
verificare il checksum prima di estrarre. Su Linux, dalla directory che contiene
entrambi, usare `sha256sum -c <file>.sha256`. Su Windows confrontare il valore
nel sidecar con `(Get-FileHash -Algorithm SHA256 <file>).Hash.ToLowerInvariant()`:
stampare l'hash senza confrontarlo non è una verifica. Estrarre in una directory
temporanea sullo stesso filesystem della destinazione e verificare i digest:

```sh
python3 scripts/check-digest-manifesto.py --albero <directory-estratta>
```

Controllare che `MANIFEST.json` riporti versione, piattaforma, profilo e canale
attesi. Eseguire `<directory-estratta>/bin/plenora-io[.exe] --version` e `catalog`;
per una candidate usare inoltre i referti dello smoke prodotto dal workflow,
non sostituirli con una prova nell'albero sorgente. Solo dopo queste verifiche
rinominare la directory temporanea col nome definitivo. Non estrarre mai sopra
una versione già presente.

Su Windows controllare che `MANIFEST.json` dichiari
`firma.stato: non_richiesta`. L'assenza di Authenticode è intenzionale; chi
opera sotto una policy che ammette soltanto editori firmati deve distribuire il
binario attraverso un canale aziendale approvato oppure non installarlo.

**Attivazione e aggiornamento.** Il programma non installa un servizio e non
gestisce un collegamento `current`: il supervisore del deployment deve puntare
al percorso assoluto del binario scelto. Per aggiornare, installare la nuova
versione affiancata, verificarla, fermare le nuove invocazioni, cambiare quel
solo percorso e riavviare. Conservare la directory precedente fino alla fine
della finestra di osservazione. Profilo e versione sono parte dell'identità:
passare da `base` a `filegdb` non è una sostituzione trasparente.

**Rollback.** Fermare le nuove invocazioni, ripristinare nel supervisore il
percorso assoluto della directory precedente, riavviare e ripetere `--version`
e `catalog`. Non copiare file dalla nuova installazione nella vecchia e non
riusare il runtime GDAL di un'altra versione. La CLI è stateless: il rollback
del binario non migra né ripara gli output già pubblicati.

**Recovery dei dati.** Una busta con `remote_effect: none` non ha lasciato una
destinazione visibile e il comando può essere ripetuto dopo aver corretto la
causa. Con `remote_effect: partial` e `retry: requires_recovery` non rilanciare
alla cieca: isolare la destinazione, inventariare i companion già visibili e
applicare la procedura Shapefile di `docs/PRODUCT.md` prima di ritentare. Dopo
un kill forzato vale la stessa cautela, perché non esiste una busta che possa
certificare lo stato. Gli spool non richiedono pulizia: sono file senza nome;
lo staging ordinario viene rimosso al rientro cooperativo, ma non va assunto
dopo la terminazione forzata.

### 5-bis. Il congelamento, e le due revisioni

**Perché esiste questa sezione.** Il modello ne aveva una sola, e con una sola
era **impossibile**. Il contratto pretendeva che `v<versione>` puntasse a HEAD;
la decisione di rilascio è `release_authorized: true` dentro un file versionato,
e l'evidenza del livello 2 è un altro file versionato. Registrarle crea un
commit, quel commit sposta HEAD, e il tag smette di puntarci: soddisfare
`decisione-scritta` rompeva `candidate-coerente`, e viceversa.

Non era un blocco da chiudere — era una condizione che nessuna release poteva
soddisfare. Si sarebbe visto all'ultimo passo, con il tag già creato, e la via
d'uscita più comoda sarebbe stata spostare il tag sul commit dell'evidenza: cioè
far puntare la release a un albero che contiene l'attestazione di se stesso.

**Le due revisioni.**

| | Che cos'è | Come si ottiene |
|---|---|---|
| `revisione_candidate` | lo SHA **congelato**: da lì escono binari, SBOM e provenance, e lì punta `v<versione>` | si **scrive** nello stato, una volta, al congelamento |
| `revisione_assurance` | il commit che registra evidenza e decisione | è **HEAD**: si **deriva**, e non si scrive da nessuna parte |

La seconda non si scrive per una ragione che non è di stile. Un campo che
dovesse contenere lo SHA del commit che lo contiene non è compilabile nel
momento in cui lo si scrive: l'unico modo di riempirlo sarebbe una cifra
inventata, e una cifra inventata accanto a una qualifica è ciò che questo
registro esiste per impedire. Una sonda pretende che quel campo **non** compaia
nello stato.

**Che cosa verifica il gate.** Cinque cose, congiunte:

1. la versione del manifesto è quella del workspace;
2. `v<versione>` punta alla **revisione congelata**, non a HEAD;
3. i quattro artefatti del perimetro sono fissati con nome, digest, dimensione e
   revisione — e la revisione è quella congelata;
4. fra la revisione congelata e HEAD è cambiato **solo** ciò che l'assurance
   produce;
5. `release_action.allowed` è consentita.

**L'allowlist dopo il congelamento.** Quattro voci, e nient'altro. È
un'allowlist e non una denylist perché una denylist dimentica la famiglia che
nasce domani, e la dimentica in silenzio.

| Percorso ammesso | Perché |
|---|---|
| `assurance/current-state.json` | lo stato e la decisione: è il prodotto dell'assurance |
| `assurance/evidence/` | l'evidenza della corsa di livello 2 |
| `assurance/registries/release-contract-current.json` | è lì che gli invarianti passano a `verified` quando l'evidenza arriva |
| `docs/RELEASE.md` | il blocco generato, che segue lo stato |

Ciò che **non** vi è, e che quindi rende tutto rosso se cambia dopo il
congelamento: `crates/`, `Cargo.toml`, `Cargo.lock`, `.github/workflows/`,
`scripts/` — costruttori, verificatori e il gate stesso —, `vendor/`,
`release/`, e `assurance/registries/distribuzione-matrice.json`. Cioè il codice,
il lock, la macchina che costruisce, quella che verifica e il contratto di
distribuzione. Se si muovessero dopo il congelamento, l'albero qualificato e
quello da cui gli artefatti sono usciti sarebbero due, e il secondo non sarebbe
stato misurato da nessuno.

Il registro del contratto **è** ammesso, ed è la voce che merita una ragione:
ammetterlo non apre nulla, perché le sue affermazioni non si autocertificano —
le riesegue `check_release_contract.py`, che sta in `scripts/` ed è congelato.

**Gli stessi byte, non una ricostruzione.** `check-deliverable.py` confronta
ogni archivio con i **propri** sidecar: un insieme ricostruito da capo è
internamente coerente e passa, perché ogni checksum descrive fedelmente
l'archivio che gli sta accanto. La domanda a cui non risponde è se quegli
archivi siano gli **stessi** su cui è girata la qualifica.

Non è una distinzione teorica: due costruzioni della stessa revisione possono
differire di un byte — un timestamp dentro l'archivio basta — e allora ciò che
si è misurato e ciò che si consegna sono due insiemi diversi, «equivalenti» nel
senso che nessuno ha verificato. I digest fissati al congelamento sono l'unico
riferimento esterno che rende la differenza visibile, e li confronta
`check-deliverable.py --contro-la-candidate`.

**La sequenza di pubblicazione.** In quest'ordine, e l'ordine è il punto:

1. si congela lo SHA e lo si scrive in `revisione_candidate`;
2. si esegue **Distribuzione** in canale `candidate` su quello SHA;
3. si fissano in `candidate_release.artefatti` i quattro nomi con digest,
   dimensione e revisione, presi dalla provenance della corsa;
4. si crea `v<versione>` **sulla revisione congelata**;
5. si esegue il livello 2 e si registra l'evidenza; si scrive
   `release_authorized`. Questi commit sono l'assurance, e toccano solo
   l'allowlist qui sopra;
6. si pubblicano nel canale di release **gli artefatti di quella corsa**, non
   una ricostruzione;
7. si **riscaricano** dal canale e si esegue:

```
python3 scripts/check-deliverable.py \
  --directory <scaricati> --versione <versione> --canale candidate \
  --revisione <revisione_candidate> \
  --contro-la-candidate assurance/current-state.json
```

Il passo 7 è ciò che chiude la promessa: verifica che i byte pubblicati abbiano
esattamente i digest congelati al passo 3. Senza, «gli stessi artefatti» resta
una parola.

### 6. Decisione finale di rilascio

**Criterio di uscita.** Tutti i punti precedenti chiusi **o dichiaratamente
differiti** — il punto 4 è differito, e la release non promette ciò che quel
rinvio ritira; `check_release_contract.py --release` verde, cioè nessun
invariante `release_blocking` e ogni rinvio ben dichiarato; un checkpoint di
livello 2 su un albero pulito, con SHA e impronta invariati; un **soak mirato**
su `dxf_reader` e `shp_reader`, oltre alla smoke ordinaria; l'evidenza in un
commit separato.

Un punto differito non alleggerisce questo criterio: lo sposta. Ciò che non è
verificato resta non verificato dopo l'autorizzazione esattamente come prima, e
la riga che `--release` stampa lo ripete a ogni corsa.

**Perché un soak mirato, e perché su quei due.** La smoke esiste da sempre, ma
per un periodo ha costruito zero target: il job installava la toolchain e non
GDAL, e `filegdb_reader` faceva morire la costruzione prima che il primo target
esistesse. Rimessa in funzione, in due corse consecutive ha trovato due difetti
seri e di famiglie diverse — un ciclo che non termina in `dxf_reader`, un panico
raggiungibile in `shp_reader` — con sessanta secondi per target.

Due su due non è una statistica, ma è abbastanza per non trattare quei due
lettori come esplorati. Un soak più lungo sui soli due dice se la resa era il
recupero di un arretrato o un ritmo: se non trova più niente, l'arretrato era
finito; se trova, l'avremmo saputo dopo il rilascio invece che prima. Va eseguito
**sullo SHA congelato**, insieme al resto della campagna, perché è di quello che
si vuole sapere.

Solo allora `release_authorized` può diventare `true`, e sarà una decisione
scritta — non la conseguenza automatica di sei caselle verdi.
