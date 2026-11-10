# Rilascio — dove siamo e dove andiamo

Lo stato di questo documento non è scritto a mano: il blocco qui sotto è
**generato** da [`assurance/current-state.json`](../assurance/current-state.json)
e dal [registro del contratto corrente](../assurance/registries/release-contract-current.json),
e `check_docset` lo confronta carattere per carattere. Due verità manuali
divergono, e divergono in silenzio.

Si rigenera con `python3 scripts/check_docset.py --riscrivi-stato`.

---

## Perimetro del sorgente congelato

Se la candidate dichiara `plenora-io-<versione>-src.tar.gz` fra gli artefatti,
anche il sorgente entra nella qualifica: nome, digest, dimensione e revisione
sono verificati dal gate e il preparatore lo include nel manifesto pubblico.
I due pacchetti Python restano parte della qualifica degli artefatti.

L'omissione del sorgente fallisce quando il workflow `distribuzione.yml` alla
`revisione_candidate` contiene la costruzione dell'archivio sorgente. Il gate
e il preparatore leggono quel workflow da Git; una lettura fallita blocca.
Non e' prevista un'esclusione implicita di un artefatto che la distribuzione
produce. I verbali storici senza sorgente restano validi se il loro workflow
congelato precede questa costruzione: il workflow corrente non estende il
perimetro passato e nessun digest viene inventato retroattivamente.

## Stato

<!-- generato da assurance/current-state.json: inizio -->

> Questo blocco è **generato**. La sua autorità è
> [`assurance/current-state.json`](../assurance/current-state.json); modificarlo
> a mano crea la seconda verità che esiste per impedire.

| Campo | Valore |
|---|---|
| baseline documentale | `2fe9b54` |
| ultima qualificata | `ee69336` |
| revisione misurata | `ee69336` |
| passi del checkpoint | 111 |
| passi verdi | 111 |
| passi omessi | 0 |
| passi falliti | 0 |
| input di replay | 50 404 |
| target di replay | 15 |
| crash di replay | 0 |
| target di smoke eseguiti | 15 |
| target di smoke totali | 15 |
| finding di smoke | 0 |
| target in quarantena | 0 |
| copertura LCOV | 88,77% |
| righe coperte LCOV | 31 344 |
| righe strumentate LCOV | 35 309 |
| copertura cargo | 86,63% |
| soglia di copertura | 80,00% |
| baseline differenziale | `2ddfb58cb21975544f51188cf081fc6d6228e082` |
| esito differenziale | 100.00% |
| gruppi ASSURANCE-N1 | 50 |
| gruppi ASSURANCE-N1 aperti | 0 |
| blocchi | 0 |
| capacità differite | 2 |
| S9, qualificato su | `ee69336` |
| candidate, versione del manifesto | `7.0.2` |
| candidate, stato | pubblicata |
| candidate, revisione congelata | `ee69336f8aed84767d1a1cd1bf5b0d526e3422e2` |
| candidate, versione del workspace | `7.0.2` |
| candidate, artefatti congelati | 7 |
| candidate, tag previsto | `v7.0.2` |
| candidate, tag creato | sì |
| candidate, revisione del tag | `ee69336f8aed84767d1a1cd1bf5b0d526e3422e2` |
| candidate, tag sulla candidate | sì |
| candidate, assurance entro l'allowlist | sì |
| candidate, release_action consentita | no |
| release pubblicate | 10 |
| release_authorized | `true` |

I blocchi sono l'elenco esatto dei `release_blocking` del
[registro del contratto corrente](../assurance/registries/release-contract-current.json)
— non un riassunto:

| Blocco | Sintesi |
|---|---|

Le capacità **differite** non sono blocchi chiusi: non sono richieste
da questa release e **non sono verificate**. Ciascuna dichiara che cosa
la release non promette, ed è la sola lettura autorizzata del rinvio:

| Capacità | Sintesi | La release non promette |
|---|---|---|
| `sistema.qualifica-cross-component` | differita: la catena a tre componenti non e' qualificata, e la 2.0.0 non la promette | la 4.0.0 NON promette interoperabilita' end-to-end certificata con plenora-data-tools e plenora-database-tools. La catena IO-tools -> data-tools -> database-tools non e' qualificata in nessuna delle due direzioni, su nessuna piattaforma; nessuna delle quindici proprieta' del contratto di sistema -- fra cui srid, crs_resolution, axis_order e native_metadata -- e' verificata attraverso i tre componenti; e la direzione database -> data -> IO non e' mai stata eseguita. Chi compone i tre componenti in produzione lo fa senza evidenza di conservazione dei metadati ai confini, e deve verificarla per conto proprio. Che questo componente produca e accetti, alla 4.0.0, entrambe le serializzazioni Arrow che i quattro archi `direct` della matrice di composizione nominano riguarda la **condizione a nostro carico** su quei collegamenti: e' conformita' al contratto dichiarato, non interoperabilita' verificata con gli altri componenti, e non riduce di nulla cio' che questo rinvio lascia scoperto. |
| `distribuzione.licenza-first-party` | differita: dalla 4.1.1 gli artefatti portano la licenza proprietaria decisa dal titolare; manca la sua conferma esterna | Fino alla 4.1.0 compresa gli archivi non portano termini first-party. Dalla 4.1.1 portano `LICENSE` con la licenza proprietaria; la prova esterna che questo invariante pretende -- `release/licenza-first-party.json`, la conferma del titolare del testo e della propria denominazione legale -- non c'e', e l'invariante resta differito e **non verificato**: il testo negli artefatti non dimostra da solo di essere l'atto del titolare. Le licenze dei componenti di terzi restano incluse e applicabili. Il classificatore Private :: Do Not Upload e mantenuto nei pacchetti Python; nessun pacchetto e pubblicato su PyPI. |

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

**Soak della candidate 7.0.2 registrati.** Soak mirato chiuso su dfc259e84: dxf_reader monotonic/boottime/realtime 3602,011246833/3602,011246922/3602,011247158 s, 12962389 esecuzioni (libFuzzer 3601 s); shp_reader monotonic/boottime/realtime 3601,888934065/3601,888934070/3601,888934135 s, 19169809 esecuzioni (libFuzzer 3601 s). Entrambi exit 0, nessun finding, durata dimostrata, esito completa. Due campagne in parallelo sulla VM di qualifica Linux; CPU diagnostica senza soglia. Verbale con referti originali e digest in assurance/evidence/checkpoint-ee69336.json.

Il tempo wrapper include preparazione/build. Hash dei log e del misuratore,
campioni e tutti i campi del giudizio sono verificati localmente con
`scripts/soak_misurato.py`, senza rieseguire le campagne. La concordanza
degli orologi indica solo nessuna discontinuita rilevata. Il verbale smoke
L2 resta intatto; i soak non lo sostituiscono.

**Soak della candidate 7.0.1 registrati.** Sulla revisione di assurance
`679d94c`, figlia della candidate `2ddfb58`, il soak mirato e chiuso.
Soak mirato chiuso su 679d94c: dxf_reader monotonic/boottime/realtime 3602,163649988/3602,163650300/3602,163650513 s, 13394138 esecuzioni; shp_reader 3601,926774601/3601,926774600/3601,926774502 s, 11591105 esecuzioni. LibFuzzer dichiara 3601 s per bersaglio; entrambi exit 0, nessun finding, durata dimostrata, esito completa. Due campagne in parallelo sulla VM a 8 vCPU; CPU diagnostica senza soglia. Verbale con referti originali e digest in assurance/evidence/checkpoint-2ddfb58.json.

Ubuntu 24.04, 8 vCPU, 16 GB, nightly-2026-07-21 e cargo-fuzz 0.13.2;
checkout pulito prima dell avvio, come dichiarato dal titolare.
Il tempo wrapper include preparazione/build; i 3601 s sono la durata dichiarata
da libFuzzer. La concordanza degli orologi indica solo nessuna discontinuita
rilevata. Hash dei log e del misuratore, campioni e giudizi sono verificati
localmente senza rieseguire le campagne.

Come per la 7.0.0, `fuzz-smoke-ultima.json` resta la copia completa del livello 2
(15/15, 60 s): il controllo `--verifica-campagna` pretende tutti i bersagli.
La copia VM sovrascritta dai soak paralleli riporta solo `dxf_reader`; percorso
e digest sono conservati nel [verbale](../assurance/evidence/checkpoint-2ddfb58.json),
insieme ai referti separati di entrambi i soak. Non si fondono i verbali.
I permessi `release_authorized` e `release_action_allowed` restano `false`.

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
dal proprio percorso (`crates/plenora-io-tools/src/radici.rs`).

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
l'ultima disponibile: `gdal-sys 0.12.0` spedisce binding pre-costruiti soltanto
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

### Le licenze di terze parti negli artefatti

**Che cosa coprono.** Gli archivi della CLI (Linux e Windows, profili `base`
e `filegdb`) portano in `LICENSES/`:

- per ogni componente nativo (il runtime GDAL e le sue librerie, dai pacchetti
  conda): il testo di licenza del pacchetto, come gia' prima;
- per ogni crate Rust di terzi linkato nel binario, **dalla 4.2.0**: i propri
  file in `LICENSES/crate-rust/<nome>-<versione>/` -- licenza, `NOTICE`,
  copyright, cioe' l'avviso di copyright di quel crate e il `NOTICE` di un
  crate Apache-2.0 -- accanto al testo generico per identificatore
  (`LICENSES/crate-rust/<id>.txt`), che resta perche' un'espressione
  `MIT OR Apache-2.0` vuole entrambi i testi. `CRATE.json` elenca per ogni
  crate i file e il loro sha256.

I file vengono dai sorgenti dei crate che cargo ha verificato col checksum di
`Cargo.lock` (`cargo metadata`, `scripts/avvisi_crate.py`). Quindici crate del
grafo non portano un file di licenza nel pacchetto pubblicato (fra questi
`rstar` in sei versioni, `flatbuffers`, `gdal-sys`, `geo-traits`): per loro il
testo e' quello del repository upstream al commit di `.cargo_vcs_info.json`,
versionato in `assurance/licenze-crate/` con sha256 e blob git
(`assurance/registries/avvisi-crate.json`).

**I gate.** `scripts/avvisi_crate.py` (CI, job `rust` sul grafo Linux e
`windows` sul grafo Windows, entrambi i profili) fallisce se un crate linkato
non ha un avviso proprio, se un testo del registro non ha l'impronta registrata
o se una voce del registro non serve piu'. `check-licenze-artefatto.py`, sul
pacchetto costruito, fallisce se un crate di `CRATE.json` non porta i propri
file o se un file non e' quello dichiarato.

**Che cosa non coprono, e perche'.**

- `pdqselect` 0.1.0 (`Apache-2.0/MIT`) non ha un testo di licenza da nessuna
  parte: ne' nel pacchetto, ne' in un repository (non ne dichiara uno), ne'
  negli avvisi dei sorgenti. Porta un avviso generato dal suo manifesto (nome,
  versione, autori, licenza) accanto ai testi canonici dei due identificatori.
  Rientro: quando esce dal grafo o pubblica un testo.
- Il pacchetto Python dell'SDK (wheel e sdist) non contiene codice di terzi:
  nessun binario e nessuna dipendenza impacchettata; `pyarrow`, se installato,
  porta le proprie licenze.
- La valutazione di `cargo-about`: fa lo stesso lavoro, ma e' un binario in piu'
  da fissare, e per i crate senza testo nel pacchetto lo cerca in rete al
  momento della costruzione. La costruzione dipenderebbe da un servizio esterno
  e il testo spedito non sarebbe fissato nel repository; qui i testi mancanti
  sono versionati e verificati.

### L'attestazione del profilo pubblico

Dalla 4.1.0 l'autorizzazione ha una condizione in piu', e comporta un passo
operativo: **prima** di eseguire `check_release_contract.py --release`, ogni
artefatto `cli` del manifesto va attestato.

```
python3 scripts/check_public_contracts.py \
  --contracts .plenora-contracts \
  --artefatto dist/plenora-io-X.Y.Z-linux-x86_64-base.tar.gz \
  --attestazione assurance/evidence/profilo-pubblico/linux-x86_64-base.json
```

`--artefatto` non e' una comodita' rispetto a `--cli`: cambia che cosa viene
provato. Il binario non lo indica chi lancia il comando, lo si **estrae
dall'archivio**, e il digest che finisce nell'attestazione e' quello dei byte su
cui le sonde hanno girato. La condizione confronta quel digest con quello che il
manifesto distribuisce, e un'attestazione che parlasse di altri byte non e' una
verifica parziale di questi: non dice niente di questi.

Le due opzioni si escludono, per la stessa ragione: `--cli` interroga un binario
senza dire da dove venga, ed e' quello che serve in CI; ammetterli insieme
vorrebbe dire poter attestare un archivio interrogandone un altro.

**Su ogni piattaforma.** Un binario Windows non si esegue in una corsa Linux,
quindi la sua attestazione si produce dove quel binario gira, e si versiona
insieme alle altre. Non c'e' una via differita: sarebbe la stessa cosa che
manca. `assurance/evidence/` resta scrivibile dopo il congelamento proprio
perche' evidenze come questa nascono li'.

**Che cosa non fa.** Non sostituisce lo smoke sull'artefatto installato, che e'
un'altra delle sei condizioni di uscita: risponde a «i requisiti pubblici sono
verificati su questi byte», non a «il pacchetto si installa e funziona».

---

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

**Superare una candidate va dichiarato nello stesso commit.** È uno spigolo
trovato pagandolo. `assurance_entro_l_allowlist` è **derivato**: vale vero
quando HEAD è la revisione congelata, o la contiene senza aver toccato altro che
l'allowlist. Un commit che corregge il codice dopo un congelamento rende quindi
quel campo falso — tocca `scripts/` — e se lo eredita `true` dal commit
precedente il contratto va rosso, correttamente, e il livello 2 su quella
revisione non passa.

La cura non è ricordarsi di aggiornare il campo: è che **il commit che supera
una candidate porti `assurance_entro_l_allowlist: false`**. Quel valore si può
calcolare mentre lo si scrive, perché la diff è contro il congelamento
**vecchio**, che è noto. Solo il congelamento **nuovo** non si può scrivere da
dentro il commit che lo produce — è la stessa impossibilità di
`revisione_assurance` — e per questo lo fissa il commit seguente, che tocca solo
l'allowlist.

Ne segue che il livello 2 misura la **revisione di assurance**, non sempre la
candidate: fra le due può starci il solo `assurance/current-state.json`, che è
nell'allowlist proprio perché non può influire sugli artefatti. L'evidenza
registra entrambe le revisioni, e il tag va sulla candidate.

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
3. si esegue il livello 2 sullo stesso SHA;
4. si fissano in `candidate_release.artefatti` i quattro nomi con digest,
   dimensione e revisione, presi dalla provenance della corsa;
5. si prepara una **draft release** associata a `v<versione>` e vi si caricano
   esattamente quegli archivi, senza ricostruirli. Una draft non materializza
   il tag: GitHub lo crea solo alla pubblicazione;
6. si **riscaricano** dalla draft gli stessi byte e si verificano (comando qui
   sotto);
7. si registrano evidenza del livello 2 e qualifica della distribuzione, nei
   soli file ammessi dopo il congelamento. `release_authorized` resta `false`;
8. **autorizzazione esplicita**: la decisione, presa da chi rilascia;
9. si crea `v<versione>` **sulla revisione congelata**;
10. si aggiorna l'assurance con `release_authorized: true`;
11. si esegue il **gate finale** — `check_release_contract.py --release` —, che
    ora ha tag, artefatti e decisione da confrontare;
12. si pubblica la draft **immutata**;
13. si riscaricano i byte dalla release pubblicata e si ripete la verifica.

**Perché l'autorizzazione precede la pubblicazione, e non la segue.** Due
versioni precedenti di questa sequenza sbagliavano, in due modi opposti.

La prima metteva `release_authorized` al passo 5, prima della pubblicazione e
prima della verifica dei byte: autorizzare senza aver verificato che i byte
pubblicati siano quelli qualificati rende la verifica una formalità — se il
confronto fallisse, la release sarebbe già autorizzata, e l'unica mossa rimasta
sarebbe ritirarla.

La seconda la metteva **dopo** la pubblicazione, e sbagliava di più: rendere
pubblica una release prima di averla autorizzata significa che il pubblico la
vede prima che qualcuno abbia deciso di rilasciarla. Un ritiro, a quel punto,
non è una correzione ma una smentita.

L'ordine giusto ha l'autorizzazione **fra** la verifica e la pubblicazione: si
verifica tutto ciò che si può verificare su una bozza, si decide, si fissa la
decisione, il gate la ricontrolla, e solo allora la release diventa pubblica.
La verifica dopo la pubblicazione resta, ed è l'ultima: dice che pubblicare non
ha cambiato i byte.

Il comando del passo 6:

```
python3 scripts/check-deliverable.py \
  --directory <scaricati> --versione <versione> --canale candidate \
  --revisione <revisione_candidate> \
  --contro-la-candidate assurance/current-state.json
```

Il passo 6 è ciò che chiude la promessa: verifica che i byte scaricati abbiano
esattamente i digest congelati al passo 4. Senza, «gli stessi artefatti» resta
una parola. Si ripete al passo 13, sui byte della release pubblicata: la draft
dice che si è caricato ciò che si è misurato, la release pubblicata che si è
pubblicato ciò che si era caricato.

### 6. Decisione finale di rilascio

**Precedente storico: soak della 7.0.0.** Su `fc98634`, `dxf_reader` misura
3823,438642146 s monotonic, 3823,438642754 s boottime e 3823,438643456 s
realtime; `shp_reader` misura rispettivamente 3823,181138732 s,
3823,181138840 s e 3823,181138992 s. Il wrapper include preparazione e build;
libFuzzer dichiara 3601 s per bersaglio, con 10.242.106 e 5.749.588 esecuzioni.
Entrambi hanno exit 0, nessun finding, durata dimostrata ed esito `completa`.
Il [verbale](../assurance/evidence/checkpoint-fc98634.json) conserva digest dei log,
provenienza e riferimenti ai referti originali; la verifica locale ha ricalcolato
hash e giudizi senza rieseguire le campagne.

I due soak sono girati in parallelo sulla stessa VM Ubuntu 24.04 a 4 vCPU,
con nightly-2026-07-21 e checkout pulito della candidate, come dichiarato dal
titolare. Il criterio e il misuratore non richiedono esclusività o saturazione
CPU: la CPU dei figli è diagnostica e include la build. La misura resta
inalterata; la concordanza degli orologi indica soltanto nessuna discontinuità
rilevata. Questo e il precedente storico della 7.0.0, gia pubblicata; per
la release 7.0.1 vale la sezione dedicata: pubblicata, con decisione del
titolare registrata e permesso operativo consumato.

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

## Verso la 3.0.0 — primo passo concordato

Il primo passo è far integrare le modifiche dei tre fork nei rispettivi
progetti upstream, quindi tornare alle dipendenze ufficiali. La decisione
è stata confermata dal titolare il 2026-09-07. Il resto del perimetro della
3.0.0 non è fissato da questa sezione.

Il confronto dei sorgenti upstream del 2026-09-07 dà questo punto di partenza;
è un censimento statico, non una nuova qualifica:

| Fork locale | Revisione upstream esaminata | Contributi da preparare |
|---|---|---|
| `dxf` 0.6.1 | [`ixmilia/dxf-rs`, `main`, `c63c560`](https://github.com/ixmilia/dxf-rs/tree/c63c560c758179b8ff9971ea965b2bc944605545) | separare la protezione dal ciclo senza avanzamento in `BLOCK` dalla lettura progressiva e dall'API `DrawingEntityReader` |
| `gdal` 0.17.1 | [`georust/gdal`, `master`, `c0a6266`](https://github.com/georust/gdal/tree/c0a6266b987a2c2d44aea9956c2cae1ed71445cf) | proporre separatamente `set_ignored_fields` e le API per leggere e impostare i percorsi di ricerca PROJ |
| `shapefile` 0.6.0 | [`tmontaigu/shapefile-rs`, `master`, `646ec58`](https://github.com/tmontaigu/shapefile-rs/tree/646ec58298c8757723dedf6cac122d66de9985da) | proporre la scrittura di `NullShape` e completare la validazione delle lunghezze dei record; `checked_mul(2)` è già presente upstream, quindi quella parte non va riproposta |

Ogni contributo va adattato al ramo corrente del progetto originale e
accompagnato da test di regressione autonomi. Le modifiche locali di packaging
e le soppressioni di warning non si trasferiscono automaticamente upstream.
Per i reader si portano anche i casi minimali che hanno motivato le correzioni;
per DXF si misura inoltre la conservazione del comportamento e della lettura
progressiva, e per GDAL la rilocabilità dei dati PROJ.

**Criterio di chiusura, per ciascun fork:** modifiche accettate upstream,
release ufficiale che le contiene, dipendenza e lock aggiornati in IO-tools,
test dei driver e verifiche pertinenti superati senza la patch locale.
Solo a quel punto la dipendenza torna al nome upstream su crates.io e si
elimina la copia in
`vendor/` e la sua infrastruttura di governo. Una PR aperta o un merge non
ancora distribuito non soddisfano il criterio. L'accettazione e la pubblicazione
upstream dipendono dai manutentori dei progetti originali.

## 7.0.2 — 2026-11-09 (candidate, non rilasciata)

### Draft verificata e passi dopo la decisione

La draft **439565237**, [verificata prima della pubblicazione](https://github.com/PlenoraETL/plenora-IO-tools/releases/tag/untagged-fae27862383bd18231f2),
resta non pubblicata e non autorizzata. Il verbale completo e i 24 nomi, dimensioni
e SHA-256 ricalcolati sono in `assurance/evidence/checkpoint-ee69336.json`,
`release_verification.draft`. I 23 asset della Distribuzione candidate
42637007133 sono accompagnati da `adoption-manifest.json`, aggiuntivo rispetto
alla 7.0.0. Il gate legge **il manifesto locale**
`contracts/adoption-manifest.json`: questa registrazione lo confronta byte per
byte con quello riscaricato. Il manifesto non entra nella directory passata
a `check-deliverable.py`; viene verificato separatamente sui cinque archivi.

Resta la decisione esplicita del titolare sui byte verificati e sui limiti
residui. I comandi seguenti sono istruzioni future, **non eseguite** da questa
registrazione. Usare il checkout di assurance aggiornato nell'ambiente Linux
di qualifica, con contratti al pin e manifesto locale identico al verificato.
Non ricostruire gli asset. Dopo la decisione autentica, conservarne data,
titolare, testo e oggetto in
`assurance/evidence/autorizzazione-7.0.2.txt` (senza inventare la risposta).

```bash
git tag -a v7.0.2 ee69336f8aed84767d1a1cd1bf5b0d526e3422e2 -m "Plenora IO-tools 7.0.2"
git rev-parse 'v7.0.2^{commit}'
python3 - <<'PY'
import json
from pathlib import Path
p = Path('assurance/current-state.json')
s = json.loads(p.read_text(encoding='utf-8'))
c = s['aperto']['candidate_release']
decisione = Path('assurance/evidence/autorizzazione-7.0.2.txt').read_text(encoding='utf-8').strip()
assert decisione and c['revisione_candidate'] == 'ee69336f8aed84767d1a1cd1bf5b0d526e3422e2'
s['release_authorized'] = True
c.update(tag_creato=True, tag_sulla_candidate=True,
         tag_revisione=c['revisione_candidate'], release_action_allowed=True)
c['nota'] += '\n\nAutorizzazione esplicita del titolare:\n' + decisione
p.write_text(json.dumps(s, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
PY
python3 scripts/check_docset.py --riscrivi-stato
```

Registrare evidenza e autorizzazione in un commit di assurance separato, nei
soli percorsi dell'allowlist, quindi eseguire il gate completo. Le note che
descrivono i permessi falsi in questa registrazione restano fatti precedenti
alla decisione; la decisione successiva ne dichiara il superamento.

```bash
python3 scripts/check_release_contract.py --release
# Proseguire soltanto con exit code 0 del gate completo.
git push origin refs/tags/v7.0.2
gh api --method PATCH repos/PlenoraETL/plenora-IO-tools/releases/439565237 -F draft=false
```

Prima del PATCH ricontrollare che ID, target, tag e tutti i 24 asset della
draft coincidano con il verbale. Il PATCH cambia soltanto `draft`: nessun
upload, sostituzione di asset o modifica delle note. Dopo la pubblicazione
riscaricare in una directory nuova:

```bash
gh release download v7.0.2 --repo PlenoraETL/plenora-IO-tools --dir target/rel702/pubblicata-riscaricata
```

Confrontare tutti i 24 nomi, dimensioni e SHA-256 con `release_verification.draft.asset`;
ripetere `check-deliverable.py --contro-la-candidate assurance/current-state.json`
sui 23 file escluso il manifesto, con versione `7.0.2`, canale `candidate` e
revisione `ee69336f8aed84767d1a1cd1bf5b0d526e3422e2`; ripetere anche la verifica
del manifesto riscaricato sui cinque archivi. Registrare l'esito in un blocco
separato `release_verification.published`, aggiornare lo stato a pubblicata e
riportare `release_action_allowed` a `false`: il permesso e' speso.

Allineamento comune: serde_json 1.0.151 → 1.0.154 (feature invariate),
thiserror/thiserror-impl 2.0.19 → 2.0.21 e time 0.3.54 → 0.3.55 nei lock
che li contengono. Il ramo thiserror 1.0.69 resta richiesto dai suoi
consumatori. Aggiornati tutti e tre i lock, incluso quello autonomo del
vendor. Maturin passa dal requisito `>=1.15,<2.0` e dal pin CI 1.15.0 a
`==1.16.0` / 1.16.0; il packaging copia lo stesso pyproject.
Workspace e SDK passano a 7.0.2. Rust resta 1.98.1, Arrow 60.1.0,
calamine 0.38.0 e PyO3 0.29.2.

Il fork `plenora-fork-parquet` conserva base 60.0.0 e decoder. Come nella
politica di data-tools PR #154, il requisito Arrow autonomo diventa
`>=60.1.0, <61` anche per dipendenze opzionali e di sviluppo; i pin del
workspace restano esatti. Patch: `assurance/parquet-arrow-minimo.patch`,
con provenienza nel registro del fork. `scripts.test_allineamento_dipendenze`
verifica minimo, lock e feature.

**DIFETTO SILENZIOSO CORRETTO - GeoParquet accettava dati corrotti.**
Versioni colpite: tutte le stabili **1.0.0-7.0.1**, incluse 1.0.1, 4.1.0,
4.1.1 e 5.0.1, e le prerelease **0.1.0-rc.1-rc.4, 1.0.0-rc.1-rc.2**.
Verifica statica di `git show <tag>:Cargo.toml` su tutti i 18 tag: feature
`crc` assente, `default-features=false`; dalla 5.0.0 anche
`git show <tag>:vendor/parquet/Cargo.toml` conferma `crc` opzionale, non
incluso dalle feature abilitate. Nei tag precedenti il vendor non esiste:
si usavano parquet 59.1.0 e 59.3.0 con CRC disabilitato.
La riproduzione indipendente sulla 7.0.1 ha alterato una coordinata WKB
mantenendo il CRC originale: successo, una riga e fedelta' lossless,
senza segnalare la corruzione. Non era un limite dichiarato del formato.
La correzione abilita `crc`: una pagina con checksum
errato, anche se contiene una geometria valida, ora è rifiutata; checksum
corretto e checksum assente restano leggibili. La prova
`il_driver_verifica_crc_presente_e_ammette_crc_assente` in
`crates/driver-geoparquet/tests/parquet_decoder.rs` confronta i tre casi e
verifica il rifiuto senza panico, anche modificando la coordinata da 1 a 3
conservando il CRC originale. Controllare gli output gia' prodotti rispetto
alle sorgenti originali e rigenerarli con la versione corretta; un vecchio
successo/lossless non attesta integrita'. L'assenza di CRC resta ammessa e
non offre una verifica d'integrita'. I codec snap, zstd, brotli,
flate2-rust_backend e lz4 restano abilitati esplicitamente. CRC usa
crc32fast già presente nel grafo, senza una nuova dipendenza diretta.

### SQLite: avviso di sicurezza e difetto silenzioso corretto

**RUSTSEC-2026-0459 / CVE-2026-119375.** SQLite bundled fino a 3.53.2
(`libsqlite3-sys` fino a 0.38.2) puo' restituire memoria non inizializzata
della page cache come dati di righe se un database viene aperto insieme a
un WAL di terzi con page size diversa. L'effetto puo' essere un successo
con dati falsi o divulgazione di memoria, non soltanto un crash.
Aggiornati i lock radice e fuzz a `rusqlite =0.41.1`, che richiede
`libsqlite3-sys ^0.39.0`: risolto **0.39.1, SQLite 3.54.1**.
`hashlink` resta 0.12.2; nessun nodo aggiunto o rimosso, nessun'altra
transitiva cambiata da questo aggiornamento. Nessuna eccezione advisory.
Il registro e' `assurance/registries/dependency-exceptions.json`.

**Release pubblicate esposte:** tutte le stabili **1.0.0-7.0.1** e le sei
prerelease **0.1.0-rc.1-rc.4, 1.0.0-rc.1-rc.2**. La verifica statica dei
18 tag (`Cargo.lock`, `crates/driver-gpkg/src/lib.rs`) trova libsqlite3-sys
0.37.0 fino alla 2.0.0, 0.38.2 dalla 3.0.0. Fino alla 5.0.0 il reader
usava `Connection::open` (read-write/create e URI di default); dalla 5.0.1
usa `READ_ONLY | NO_MUTEX`, senza URI ne' immutable. In nessun tag copia
difensiva o controllo dei companion `-wal`/`-shm`. `journal_mode=MEMORY`
riguarda solo il writer sullo staging, non il lettore. DEFENSIVE,
TRUSTED_SCHEMA=false, cell_size_check e mmap disabilitato non escludono il
WAL: la sola lettura puo' applicarlo. CLI, SDK e API Rust che leggono GPKG
raggiungono questo percorso. Questa e' esposizione accertata del percorso,
non una riproduzione locale della fuga di memoria della CVE.

La prova `companion_wal_non_fidato_non_cambia_le_righe_in_silenzio` crea
cinque righe nel file e una cancellazione soltanto nel WAL: i vecchi flag
leggono zero righe, il driver corretto rifiuta esplicitamente prima di
SQLite. `companion_sqlite_anche_vuoto_o_malformato_viene_rifiutato` copre
anche companion vuoti e malformati. Le prove sono in
`crates/driver-gpkg/src/tests.rs`; controllano che i companion non cambino.
Il reader ora rifiuta `-wal`, `-shm` e `-journal` (anche link pendenti o
errori di accesso), poi apre una URI codificata con `immutable=1`: nessun
companion viene letto o creato. I companion sono ricontrollati alla fine del layer, prima del successo.
Un companion comparso e ancora presente produce un errore esplicito.
Anche un journal freddo e' rifiutato: il file va consolidato dal produttore.
La prova del journal caldo e quella del file WAL chiuso regolarmente
verificano rifiuto e lettura del file consolidato. Il file sorgente deve
restare stabile durante la lettura: questa misura non e' una snapshot
contro scrittori concorrenti dell'insieme principale/companion. Un companion
creato e rimosso durante la lettura puo sfuggire ai due controlli. Regola,
ambito, hazard e condizione di rientro sono nel registro Limitazioni di prodotto di PRODUCT.md e in ENGINEERING.md.
Su Windows il nome non esteso controllato e quello aperto sono identici:
normalizzazione Win32 invariata e canonicalizzazione allo stesso file sono
obbligatorie. Punti/spazi finali, nomi riservati e componenti normalizzati
sono rifiutati; i percorsi di almeno 260 unita UTF-16 sono esplicitamente
non supportati, anche su sistemi con long paths abilitati.

Conservare gli originali e confrontare gli output precedenti con sorgenti
fidate; rigenerare dopo il consolidamento con un motore corretto. Non
cancellare un WAL per far passare il controllo: potrebbe contenere dati
committati non ancora nel database. Non si ricostruiscono valori affidabili
dalla memoria o dai dati corrotti gia' restituiti.

La fixture che oscura rowid usa ora `writable_schema` per costruire lo
schema esterno: SQLite 3.54.1 rifiuta gia' `ALTER TABLE ADD COLUMN rowid`.
Il rifiuto del driver resta provato, senza cambiare il mapping dei dati.
La misura di 56602 righe di prodotto e il tetto (+200, da 56500 a 56700)
registrano la difesa aggiunta, senza comprimere codice per eludere il gate.

### Coerenza dei pin e misure da rifare

Il consumatore esterno usa serde_json =1.0.154 e una prova confronta il
requisito con quello del workspace. Allineata anche la copia thiserror
2.0.21 nel registro della closure GeoParquet. Le occorrenze precedenti nei
censimenti datati del 7 e 15 settembre e in PIANO-4.1.0 sono evidenza storica,
non requisiti attivi; non vengono riscritte come misure odierne. Maturin
1.16.0, time 0.3.55 e i lock restano verificati dalle sonde di allineamento.

Le quattro profondita' fuzz e il confine ASan FileGDB sono stati rimisurati
sull'albero `ba93ea4` nella VM di qualifica **Ubuntu 24.04**, con **GDAL di
sistema**, toolchain fissata **nightly-2026-07-21** e **cargo-fuzz 0.13.2**.
I requisiti raggiunti sono 35 per `shp_reader`, 12 per `filegdb_reader`, 9 per
`wkt_parse` e 9 per `geojson_reader`. La misura ASan nativa conta **180150
contatori su 1 modulo**, zero simboli del runtime in `libgdal.so.34`: GDAL non
e' strumentata. I gate `--qualifica` sono verdi sulla VM. Queste rimisure non
sono un livello 2 ne' un soak della 7.0.2; nessuna misura della 7.0.1 viene
attribuita alla candidate 7.0.2.

### Congelamento della candidate

La candidate 7.0.2 e' congelata sulla revisione
`ee69336f8aed84767d1a1cd1bf5b0d526e3422e2` con i sette deliverable della
Distribuzione candidate [42637007133](https://github.com/PlenoraETL/plenora-IO-tools/actions/runs/42637007133),
workflow `distribuzione.yml`, conclusa con successo secondo l'esito della corsa.
SHA-256, dimensioni e revisioni sono registrati nel
[verbale di congelamento](../assurance/current-state.json).
I 32 referti e i sette archivi scaricati dalla corsa sono verificati su Windows
con `check-distribuzione-completa.py` e `check-deliverable.py --contro-la-candidate`.
Questo confronto riguarda i byte della corsa Actions; non e' una verifica
della draft o della release.

### Registrazione del livello 2

Il livello 2 completo (`scripts/s9-checkpoint.sh`) e' superato sulla candidate
`ee69336f8aed84767d1a1cd1bf5b0d526e3422e2`, con `S9_CHECKPOINT_BASE=2ddfb58cb21975544f51188cf081fc6d6228e082`:
111/111 passi, zero omessi o falliti, albero pulito all'avvio, revisione e
impronta invariate. L'[evidenza](../assurance/evidence/checkpoint-ee69336.json) e' generata
dai log di `target/rel702/l2-ee69336f8aed84767d1a1cd1bf5b0d526e3422e2-attempt-2` con `scripts/registra-evidenza-s9.py`.
La cartella originale resta intatta: il registratore usa la copia
`target/rel702/verifica-l2/corsa-contratto`, identica byte per byte salvo
`console.log`, esterno ai passi ammessi dal contratto e conservato con
percorso e digest nella provenienza dell'evidenza.
Il replay conta 50404 input su 15 target senza crash;
smoke 15/15, 60 secondi ciascuno, nessun finding o quarantena.
Copertura LCOV 88.77% (31344/35309),
cargo 86.63%, soglia 80.0%.
Differenziale di libreria: 41 righe cambiate eseguibili,
41 coperte, 0 scoperte, 318 non eseguibili,
esito `100.00%`. La diagnostica CLI conta zero righe cambiate eseguibili
e non eseguibili, esito `n/d`.

Attempt-1 SCARTATO, senza valore di qualifica: il kernel della VM ha registrato corruzione di memoria (BUG: Bad page map, Oops) durante la corsa, con crash fuzz gpkg_reader non riproducibile dopo il riavvio (0 su 205 corse dello stesso input) e un .pyc corrotto. Causa dal log del kernel della VM, riproduzioni eseguite sulla VM dopo il riavvio; nessuna misura di quel tentativo viene usata.

Soak mirato chiuso su dfc259e84: dxf_reader monotonic/boottime/realtime 3602,011246833/3602,011246922/3602,011247158 s, 12962389 esecuzioni (libFuzzer 3601 s); shp_reader monotonic/boottime/realtime 3601,888934065/3601,888934070/3601,888934135 s, 19169809 esecuzioni (libFuzzer 3601 s). Entrambi exit 0, nessun finding, durata dimostrata, esito completa. Due campagne in parallelo sulla VM di qualifica Linux; CPU diagnostica senza soglia. Verbale con referti originali e digest in assurance/evidence/checkpoint-ee69336.json. Le quattro attestazioni del profilo pubblico 7.0.2 sono registrate in b4db138. Draft 439565237 verificata: https://github.com/PlenoraETL/plenora-IO-tools/releases/tag/untagged-fae27862383bd18231f2, target ee69336f8aed84767d1a1cd1bf5b0d526e3422e2, tag previsto v7.0.2, draft true. Registrati in assurance/evidence/checkpoint-ee69336.json, release_verification.draft, i 24 asset riscaricati con dimensioni e SHA-256 ricalcolati: tutti identici agli originali e ai metadati GitHub forniti dall operatore. I sette deliverable coincidono con la candidate. Il manifesto di adozione e verificato sui cinque archivi riscaricati: 14 contratti dichiarati conformi, 1 non applicabile, 8 deviazioni. Il gate legge il manifesto locale contracts/adoption-manifest.json, verificato identico al riscaricato. Resta soltanto la decisione esplicita del titolare; poi tag annotato v7.0.2 sulla candidate, registrazione dell autorizzazione e dell assurance, gate finale --release, pubblicazione della draft immutata e verifica dopo la pubblicazione. release_authorized e release_action_allowed restano false.
I sette archivi congelati restano quelli della distribuzione 42637007133.
Le evidenze storiche della 7.0.1 restano attribuite alla propria revisione;
ultima qualifica e ultima misura ora indicano `ee69336`.
Il contratto completo dopo questa registrazione va eseguito sulla VM.

Verifiche Windows della registrazione L2: check_docset.py, completezza e struttura del contratto, validate_stato_corrente, coerenza dell'evidenza e allowlist verdi. Verificati i digest dei 116 file del manifest contro gli originali e il verbale smoke byte per byte. Suite Python pertinente: 394 test, inizialmente 14 errori per python3 assente e 3 skip; con adattatore temporaneo sotto target che risolve python3 all'interprete disponibile, 388 passati, 3 fallimenti ASan/FileGDB e 3 skip. Le sonde ASan richiamano una build Cargo che fallisce sui download di libsqlite3-sys 0.39.1 e calamine 0.38.0 per TLS SEC_E_NO_CREDENTIALS. Skip: due sonde symlink senza privilegio Windows e una sonda senza release corrente. Nessuna qualifica ASan dedotta da queste sonde. Contratto completo da eseguire sulla VM. Comandi e log in target/rel702/verifica-l2.

## 7.0.1 — pubblicata

### Esito dei passi dopo la decisione

La [release v7.0.1](https://github.com/PlenoraETL/plenora-IO-tools/releases/tag/v7.0.1)
e' stata pubblicata il **2026-11-07T05:07:47Z** dalla draft **437334505**
con il solo PATCH `draft=false`, senza modificare gli asset. Il tag annotato
`v7.0.1` punta alla candidate `2ddfb58cb21975544f51188cf081fc6d6228e082`.
La decisione autentica del titolare e' conservata nella nota della release
in `assurance/current-state.json`.

Prima della pubblicazione, `check_release_contract.py --release` e' passato
sulla VM su `63898a226602a53b47366018852511f4fdc56901`: 37 invarianti, nessun
blocco, 9 condizioni verificate, come comunicato dal titolare. Restano i due
rinvii dichiarati: qualifica cross-component e conferma esterna della licenza
first-party.

La verifica post-pubblicazione e' registrata in
`assurance/evidence/checkpoint-2ddfb58.json`, `release_verification.published`;
`release_verification.draft` conserva la verifica precedente. I **24 asset**
in `target/rel701/pubblicata-riscaricata` coincidono per nomi, dimensioni,
SHA-256 e byte con la draft riscaricata e verificata. Sui 23 file escluso
`adoption-manifest.json`, `check-deliverable.py --contro-la-candidate` conferma
che i byte pubblicati sono identici ai **sette artefatti congelati**.
Il manifesto riscaricato, identico a `contracts/adoption-manifest.json`, e'
verificato sui cinque archivi: 14 contratti dichiarati conformi, uno non
applicabile e otto deviazioni; le deviazioni non contano come conformita'.

La 7.0.1 figura in `chiuso.release_pubblicate`, con tag, revisione e data.
La voce corrente e' `pubblicata`; `release_authorized` conserva la decisione
storica, mentre **`release_action_allowed: false`** registra il permesso speso.
`commit_di_assurance` indica `63898a2`, il commit del gate finale; questa
registrazione successiva non crea un nuovo commit. Una nuova corsa L2 sulla
candidate `2ddfb58`, pulita e invariata, ha superato 111/111 passi con baseline
`fc98634` e contratti al pin `7f82263`. Misure e manifest dei log sono in
`checkpoint-2ddfb58.json`; il documento precedente `checkpoint-679d94c.json`
e' integrato integralmente in `evidenza_precedente.documento`, con soak,
attestazioni e verifiche draft/post-pubblicazione conservati sulle revisioni
originali. Il precedente esito rosso resta nella cronaca; i nuovi controlli
Windows sono registrati separatamente nel checkpoint. I tre controlli di stato
corrente, release storiche e release pubblicata ora sono tutti superati.

Aggiornamento di sicurezza: tutti i componenti Arrow Rust sono fissati a
`=60.1.0`, che corregge RUSTSEC-2026-0442 in `arrow-data` (validazione della
copertura logica di RunEndEncoded). Workspace e SDK dichiarano `7.0.1`, come
il bump nella PR della 7.0.0. Il congelamento corrente e' registrato nel blocco generato e nel
[verbale](../assurance/current-state.json); non attribuisce
alla patch le misure della release precedente.

### Esposizione per superficie

La tabella distingue il difetto specifico dell'advisory da altri errori di
layout: la prevalidazione IO non sostituisce ogni controllo semantico Arrow.
I percorsi Rust sono relativi a `crates/`. Dove indicato, la prova e' statica
(costruttori e tipi del reader), non una riproduzione dinamica dell'advisory.

| Ingresso | Percorso di un RunEndEncoded non valido e prova |
|---|---|
| IPC file e stream | La copertura insufficiente dei run e i run assenti con lunghezza positiva sono rifiutati **prima** del decoder Arrow. Altri difetti semantici possono arrivare alla validazione fallibile Arrow. `driver-common/src/prevalida_arrow/run_ends.rs`; prova `driver-ipc/src/tests/layout_ipc.rs::run_ends_devono_coprire_la_lunghezza_logica`, per Int16/32/64, file, stream e dizionari. |
| Parquet e GeoParquet | Le pagine fisiche non rappresentano REE. Un hint REE in `ARROW:schema` incompatibile viene ignorato e non costruisce un array REE: `driver-geoparquet/tests/run_end_encoded.rs::uno_schema_run_end_incorporato_non_costruisce_array_run_end`. Entrambi i formati usano questo driver. Non e' un rifiuto dell'intero file: la prova rilegge i valori Int32. |
| GeoPackage | Il reader costruisce array dai valori SQLite, senza importare buffer o layout REE; nessun REE esterno arriva alla validazione Arrow. Evidenza statica: `driver-gpkg/src/lib.rs::next_batch`, `driver-gpkg/src/schema.rs`; prove di storage in `driver-gpkg/src/tests/storage_schema.rs`. |
| CSV | Il reader costruisce colonne scalari dal testo, non array REE. Evidenza statica: `driver-csv/src/lib.rs::finish_batch`; prove in `driver-csv/src/tests/presenza.rs`. |
| Shapefile | Shape e DBF diventano array scalari e WKB tramite builder; nessun layout REE e' importato. Evidenza statica: `driver-shp/src/lib.rs::finish_batch`; prove in `driver-shp/src/tests/dbf_lettura.rs`. |
| DXF | Entita' e attributi vengono materializzati in colonne e WKB, senza importazione REE. Evidenza statica: `driver-dxf/src/lib.rs::build_batch_cancellable`; prove in `driver-dxf/src/tests.rs`. |
| GDAL / FileGDB | Il backend legge feature OGR e usa builder scalari, non lo stream Arrow C di GDAL. Nessun REE esterno arriva ad Arrow. Evidenza statica: `driver-filegdb/src/lib.rs` (builder e reader); prove feature-on nello stesso crate. |
| CLI | Non riceve array in memoria: seleziona i reader dei formati. Per un file IPC vale il rifiuto anticipato sopra; per gli altri formati valgono i percorsi della tabella. Evidenza statica: dispatch in `plenora-io-tools/src/lib.rs`; prove IPC sopra sul driver effettivo. |
| SDK Python | `_arrow.py::scrivi_ipc` acquisisce lo schema dal reader prima di aprire il file e serializza con PyArrow; il successivo ingresso Rust e' IPC e applica la prevalidazione. Non usa `arrow-pyarrow` ne' importa direttamente un array C in Rust. Un produttore C non valido resta anche nel perimetro di fiducia di PyArrow prima della serializzazione; la patch Rust non lo valida in Python. Prove in `sdk/python/tests/test_arrow_60_1.py` e `test_confini_esterni.py`, piu' le prove IPC sopra. |
| API Rust | Un chiamante puo' costruire array usando Arrow direttamente: la validazione `ArrayDataBuilder::build` e' raggiungibile, e ora rifiuta lunghezze/offset oltre l'ultimo run. Prova `driver-ipc/src/tests/arrow_60_1.rs::array_del_chiamante_non_puo_dichiarare_righe_senza_run`. Le API unsafe del chiamante non acquisiscono garanzie da questa patch. |

### Comportamenti osservabili e dipendenze

* Il flag `keys_sorted` di Map resta conservato nello SDK attraverso IPC su
  piu' batch: `sdk/python/tests/test_arrow_60_1.py`. IO non usa il bridge
  `arrow-pyarrow` corretto a monte, quindi questa e' una garanzia di
  conservazione verificata, non un nuovo cambiamento del bridge in IO.
* Arrow 60.1.0 scrive una bitmap di validita' vuota per colonne senza null:
  a parita' di ingresso i byte IPC possono cambiare, mentre schema e valori
  restano uguali. Prova `ipc_senza_null_scrive_bitmap_vuota_e_rilegge_gli_stessi_valori`
  in `driver-ipc/src/tests/arrow_60_1.rs`. Non si promette identita' dei byte
  fra versioni.
* Il decoder Arrow restituisce un errore esplicito per una bitmap troppo
  corta, invece del precedente panico intercettato. Prova diretta
  `arrow_60_1_bitmap_corta_restituisce_errore_senza_panico` nello stesso
  `layout_ipc.rs`. Il percorso IO gia'
  prevalidava questo layout prima del decoder: prova
  `driver-ipc/src/tests/layout_ipc.rs::bitmap_delle_viste_in_file_flussi_dizionari_e_delta`.
  La correzione upstream non rende accettabile un input prima rifiutato.

Parquet resta il fork 60.0.0 con i delta di sicurezza del decoder: i requisiti
Arrow sono compatibili con 60.1.0 e vengono ristretti a pin esatti anche nel
manifesto e nel lock autonomi del vendor. Nessun sorgente vendor e' cambiato.
Motivazione e provenienza: `assurance/registries/vendor-parquet-fork.json` e
`scripts/parquet-fork-lock.json`; prova del decoder nell'intera suite
`driver-geoparquet`, incluso `tests/fork_parquet.rs`.

Anche la sola dipendenza di test `arrow-array` del fork GDAL e' fissata a
`=60.1.0`; non entra nel reader OGR di IO. Il delta di packaging e la nuova
impronta sono registrati in `assurance/registries/vendor-gdal-fork.json` e
`scripts/gdal-fork-lock.json`.

Aggiornato anche `calamine` a `=0.38.0` per RUSTSEC-2026-0436, senza nuove
eccezioni. IO usa Xlsx e non il parser CFB di Xls/Xlsb/VBA interessato
dall'avviso; l'aggiornamento chiude comunque l'advisory nei due lock.

**Correzione di lettura XLSX osservabile:** gli escape OOXML `ST_Xstring`
`_xHHHH_` vengono ora decodificati su tutte le unita' UTF-16, comprese le
coppie surrogate: `_x03A9_` diventa `Ω` e `_xD83D__xDE00_` diventa `😀`.
Il cambiamento attraversa il reader calamine e lo spool IO e riguarda sia
i valori delle celle sia i nomi delle colonne ricavati dalle intestazioni,
nelle stringhe inline, condivise e nei risultati testuali delle formule.
Corregge un comportamento silenzioso della 7.0.0 (calamine 0.37.0) e delle
versioni precedenti con lo stesso decoder limitato a `_x00HH_`: e' verificabile
gia' nel primo tag IO `v0.1.0-rc.1`, con calamine 0.36.1, quindi anche nella
prima stabile 1.0.0. Evidenza storica: `Cargo.toml` e il passaggio diretto di
`Data::String` in `crates/driver-xls/src/lib.rs` in quei tag; il pin 0.36.1
resta fino all'aggiornamento 0.37.0 del commit `01cda7b`.
Un'intestazione prima letterale puo' dunque cambiare nome e influire sulle
proiezioni e sulle opzioni che selezionano colonne per nome.

La prova `crates/driver-xls/src/tests/escape_ooxml.rs` genera ZIP XLSX con
ordine, timestamp e XML fissi, senza far riscrivere gli escape da un writer.
Gli attesi conformi sono letterali derivati da ECMA-376, parte 1,
22.9.2.19 (`ST_Xstring`), non calcolati con calamine; include `_x_` letterale
e `_x005F_x03A9_`, che resta `_x03A9_` senza doppia decodifica.
Le prove fissano inoltre una sola politica per i malformati, senza panico:
escape non esadecimali o incompleti restano testo; surrogate isolate o in
ordine inverso diventano U+FFFD, una sostituzione per unita' non accoppiata.
Questa tolleranza e' comportamento osservato, non una prescrizione OOXML
per UTF-16 malformato. Celle e intestazioni sono verificate in tre aperture
indipendenti e su due batch di una riga.

Gli altri effetti di calamine 0.38.0 hanno questi confini in IO:

* L'arrotondamento a mezzanotte corretto in `to_ymd_hms_milli()` non entra
  nella conversione IO: `date_native::convert` parte da `as_f64()` e applica
  il proprio arrotondamento al millisecondo e la propria epoca.
* Le correzioni XLS (nomi stream CFB, testo BIFF5, formule), XLSB (date RK
  intere e record BrtName malformati) e ODS (celle errore, tab e a capo)
  sono fuori dal driver, che istanzia `Xlsx` e non `open_workbook_auto`.
* Il supporto upstream a una parte workbook rinominata non diventa supporto
  IO: `NativeStyles::read` in `crates/driver-xls/src/date_native.rs` cerca
  ancora `xl/workbook.xml` e `xl/_rels/workbook.xml.rels`. La fixture con
  `xl/renamed.xml`, relazioni e content type coerenti viene letta da calamine
  ma IO la rifiuta con `metadati temporali XML XLSX non validi`, prima di
  consegnare valori. La prova copre la rinomina senza copie residue delle
  parti originali; non dimostra correttezza per pacchetti con parti discordanti.
* Le nuove API di stili e proprieta' personalizzate non sono esposte da IO:
  il driver usa `worksheet_cells_reader` e il proprio lettore degli stili
  temporali, non le nuove API di presentazione del workbook. Questo non
  esclude il parser degli stili dal percorso: `Xlsx::new` esegue comunque
  `read_styles`, ora ampliato; non si dichiara identita' di costi o di
  accettazione per ogni XML di stile malformato.

### Ricognizione degli altri aggiornamenti

Il confronto dei lock principale e fuzz prima/dopo questa PR individua, oltre a calamine,
soltanto `arrow-array`, `arrow-buffer`, `arrow-cmp`, `arrow-data`, `arrow-ipc`,
`arrow-schema` e `arrow-select` da 60.0.0 a 60.1.0; le altre variazioni di
versione nel lock principale sono i crate IO. Nel lock autonomo del vendor
Parquet cambiano anche arrow, arrow-arith/cast/csv/json/ord/row/string e
atoi 3.1.0 -> 3.1.1, per la risoluzione dei suoi strumenti e dev-dependency:
non entrano nel lock IO. Vi ricadono, per esempio, le correzioni upstream
dei cast Timestamp -> Date64, dell'overflow Int16 e degli aggregati sui
null dei dizionari. I fork Parquet e GDAL cambiano solo pin/packaging, come
dichiarato sopra. Confrontati il
[changelog Arrow 60.1.0](https://github.com/apache/arrow-rs/blob/60.1.0/CHANGELOG.md)
e i sorgenti delle due versioni effettivamente risolte.

Oltre agli effetti gia' descritti, la ricognizione trova queste correzioni
osservabili prima non dichiarate:

* **IPC, liste affettate contenenti union dense o sparse:** il writer ora
  applica l'offset ai buffer dell'union e ai figli sparse, evitando di
  serializzare valori di altre posizioni. E' raggiungibile dal writer Rust
  IO, non soltanto da un'API Arrow isolata. Prova con attesi numerici
  indipendenti: `liste_affettate_di_union_conservano_i_valori_attraverso_io`
  in `crates/driver-ipc/src/tests/arrow_60_1.rs`.
* **Array Arrow del chiamante Rust:** la conversione da `ArrayData` rispetta
  ora offset e lunghezza del figlio `run_ends` e l'offset delle union sparse;
  le union dense usano gli id dei tipi, non la posizione dei campi, nel
  calcolo dei null logici. La validazione dei valori delle union e quella
  degli id di `UnionFields` sono piu' rigorose. Sono cambiamenti delle
  dipendenze pubblicamente scambiate dall'API Rust; la prevalidazione IPC IO
  gia' controlla gli id delle union esterne. Evidenza statica nei sorgenti
  `arrow-array` (`run_array.rs`, `union_array.rs`), `arrow-data/data.rs` e
  `arrow-schema/fields.rs`, non una prova dinamica di ogni variante in IO.
* **IPC, slice REE vuote:** il writer normalizza a zero run fisici anche una
  slice vuota con offset non nullo (`into_zero_offset_run_array`). E' una
  correzione distinta dalla validazione di copertura dell'advisory; la
  raggiungibilita' dal writer IO e' statica.
* **Confronti Arrow e dizionari IPC:** l'uguaglianza di `ArrayData` corregge
  gli offset dei run e gli indici dei null nelle viste. Non e' soltanto
  un'operazione per le prove: `DictionaryTracker` la usa per decidere se
  riemettere un dizionario, accettarlo come identico o rifiutarne la
  sostituzione. Il collegamento al writer IO e' verificato nei sorgenti;
  non e' stata eseguita una matrice dinamica di dizionari annidati.

Non vengono attribuite a IO le correzioni del nuovo Parquet upstream, dato
che resta il fork 60.0.0, ne' quelle di arrow-cast, arrow-row, arrow-csv,
arrow-arith o Flight, assenti dal lock principale. `StreamDecoder` (messaggi spezzati)
non e' il `StreamReader` usato da IO. `arrow-select` arriva attraverso il
fork Parquet: le sue correzioni di trasformazione degli offset e di
`FixedSizeBinary(0)` possono interessare i kernel Arrow usati dal fork,
mentre `take_record_batch`, `zip` e le nuove API non sono chiamati dal
codice IO. Non si estende questa ricognizione statica a una garanzia su
ogni combinazione di tipi annidati. `arrow-cmp` ottimizza il confronto dei
dizionari di stringhe/byte mantenendo l'ordinamento dei null; non e' stata
individuata una diversa semantica IO in quel delta. Le nuove operazioni
i256 di `arrow-buffer` non sono chiamate da IO.

Le quattro profondita' fuzz e il confine ASan FileGDB sono stati rimisurati
sull'albero `47eae3b` nella VM di qualifica **Ubuntu 24.04**, con **GDAL di
sistema**, toolchain fissata **nightly-2026-07-21** e **cargo-fuzz 0.13.2**.
I requisiti raggiunti sono 35 per `shp_reader`, 12 per `filegdb_reader`, 9 per
`wkt_parse` e 9 per `geojson_reader`. I quattro rinvii di profondita' e il
rinvio ASan sono ritirati; le impronte dei JSON legano le misure al perimetro.

La misura ASan valida e' quella **nativa della VM**: **179952 contatori su
1 modulo**, zero simboli del runtime in `libgdal.so.34`. GDAL non e'
strumentata: ASan non verifica gli accessi interni alla libreria. La misura
del container con `libgdal.so.32` e' **scartata**. Gli esiti verdi dei gate
sulla VM sono stati comunicati dal titolare; i log di profondita' sono in
`target/rel701/rimisure-attempt-2` (container dev), mentre l'esito ASan nativo
e' riportato nella consegna di registrazione.

Queste rimisure non sono un nuovo livello 2 ne' un soak della 7.0.1:
checkpoint, copertura e campagna storici restano attribuiti a `fc98634`.
La candidate e' congelata con i sette deliverable della distribuzione
42206421101, dopo il livello 1 sulla VM (99 passi su 99), come registrato nel
[verbale di congelamento](../assurance/current-state.json).
Il livello 2 e' ora superato sulla revisione di assurance
`679d94c44837f0d884c2777defa9d945915faaa6`, figlia della candidate
`2ddfb58cb21975544f51188cf081fc6d6228e082`: 111/111 passi, zero file
sporchi all'avvio, revisione e impronta invariate su Ubuntu 24.04.
L'[evidenza della corsa](../assurance/evidence/checkpoint-2ddfb58.json)
registra replay 145 input su 15 target senza crash, smoke 15/15 senza finding,
LCOV 88,75% (31302/35271) e cargo 86,62%, soglia 80%.
Differenziale rispetto a `fc98634`: zero righe cambiate eseguibili,
432 non eseguibili, percentuale `n/d`; la diagnostica CLI conta zero in entrambe.
Il contratto prima del livello 2 sulla VM risulta verde: 37 invarianti,
35 verificati, zero bloccanti, due differiti (esito comunicato dal titolare).

**Cronaca precedente alla decisione e alla pubblicazione.** Soak mirato chiuso su 679d94c: dxf_reader monotonic/boottime/realtime 3602,163649988/3602,163650300/3602,163650513 s, 13394138 esecuzioni; shp_reader 3601,926774601/3601,926774600/3601,926774502 s, 11591105 esecuzioni. LibFuzzer dichiara 3601 s per bersaglio; entrambi exit 0, nessun finding, durata dimostrata, esito completa. Due campagne in parallelo sulla VM a 8 vCPU; CPU diagnostica senza soglia. Verbale con referti originali e digest in assurance/evidence/checkpoint-2ddfb58.json. Le quattro attestazioni del profilo pubblico 7.0.1 sono registrate (32/32 ciascuna, esigenti e conformi, al pin 7f82263921132226814f9829b0669ef34eae5624, digest coincidenti con gli archivi congelati). Draft 437334505 verificata: https://github.com/PlenoraETL/plenora-IO-tools/releases/tag/untagged-4e48e3cb3bf843994a85, target 2ddfb58cb21975544f51188cf081fc6d6228e082, tag previsto v7.0.1, draft true. Registrati in assurance/evidence/checkpoint-2ddfb58.json, release_verification.draft, i 24 asset riscaricati con dimensioni e SHA-256 ricalcolati: tutti identici agli originali e ai metadati GitHub. I sette deliverable coincidono con la candidate. Rispetto ai 23 asset della 7.0.0 si aggiunge adoption-manifest.json: verifica sui cinque archivi riscaricati verde, 14 contratti dichiarati conformi, 1 non applicabile, 8 deviazioni. Il gate legge il manifesto locale contracts/adoption-manifest.json, verificato identico al riscaricato. Resta soltanto la decisione esplicita del titolare; poi tag annotato v7.0.1 sulla candidate, registrazione dell autorizzazione e dell assurance, gate finale --release, pubblicazione della draft immutata e verifica dopo la pubblicazione. release_authorized e release_action_allowed restano false.
**Esito successivo:** 7.0.1 pubblicata il 2026-11-07T05:07:47Z; verifica dei 24 asset superata, autorizzazione storica conservata e permesso operativo consumato. Vedi `release_verification.published` nel checkpoint.

I soak su `fc98634` e le attestazioni storiche non sono misure della 7.0.1.

Le quattro attestazioni sono gia registrate in `fa007ae`; il verbale del
checkpoint ne riporta riferimenti e digest, verificati contro gli archivi
congelati. Non occorre ripeterle per questa registrazione.

## 7.0.0 — candidate

**Cambiamento incompatibile CSV:** il testo e' sempre quotato, il null non e'
quotato e i terminatori sono CRLF. Stringhe vuote e spazi restano valori;
i campi quotati forzano Utf8. I consumatori devono distinguere quoting e
presenza; la decisione del titolare e' quindi una release major, **7.0.0**.

La 7.0.0 contiene le correzioni dei difetti silenziosi segnalati nell'avviso
alla [release 6.0.0](https://github.com/PlenoraETL/plenora-IO-tools/releases/tag/v6.0.0).
E' ancora una candidate iniziale, senza data di pubblicazione, revisione o
artefatti congelati. L'avviso del 31/10/2026 va riferito alla correzione nella
**7.0.0**, non a una generica prossima release; le verifiche successive hanno
inoltre accertato il difetto delle geometrie miste GPKG descritto sotto,
rettificando l'esclusione iniziale nell'avviso.

**Versioni colpite e controllo degli output:** CSV, DBF e GPKG hanno percorsi
coinvolti dai tag 0.1.0-rc.1 fino alla 6.0.0; XLSX dalle prerelease fino alla
6.0.0, incluse tutte le release stabili dalla 1.0.0. I temporali condivisi
sono coinvolti dalla 2.0.0 alla 6.0.0. Le sezioni seguenti distinguono evidenza
statica e riproduzioni, elencano i tag e danno i controlli specifici.
Conservare gli originali e confrontare valori e schema con la sorgente:
null sostitutivi, cifre troncate e testo perduto non si recuperano dal solo
output. Rigenerare dalla sorgente con la versione corretta dopo aver gestito
i rifiuti espliciti. Un comando riuscito non certifica la fedelta' dei dati.

Queste note sono la bozza per il corpo della GitHub Release 7.0.0; verranno
trasferite alla pubblicazione. Le note storiche della 6.0.0 restano nella
sua GitHub Release.

### ZIP: dipendenze aggiornate

Aggiornati `zip` a 9.0.3, `calamine` a 0.37.0 e `rust_xlsxwriter` a
0.100.0; i grafi del workspace e di fuzz usano la sola versione ZIP 9.0.3.
Ritirata l'eccezione per ZIP 8.6.0 yanked, senza ampliare le eccezioni di
sicurezza. Restano le verifiche dei limiti e la prevalidazione dei contenitori.

### Shapefile/DBF: fedelta' dei valori

Corretto un difetto silenzioso della scrittura Shapefile: il testo oltre 254
byte veniva troncato, anche nel mezzo di un carattere UTF-8; i numeri potevano
perdere cifre passando a `f64`, essere arrotondati a otto decimali o troncati
alla larghezza del campo. L'operazione restituiva successo senza dichiarare
la perdita originaria. Ora questi valori sono rifiutati prima della scrittura
del batch; anche dopo batch precedenti validi, `finish` non pubblica nulla.
L'header dichiarava gia' UTF-8 (LDID 0xF0); ora viene pubblicato anche il
`.cpg` con contenuto `UTF-8`. Anche un `Date64` non multiplo
di un giorno viene rifiutato anziche' perdere la frazione temporale.

Sono inoltre rifiutati i Character vuoti, con spazi iniziali/finali o NUL;
i Float32 non diadici (per esempio 0.1f32), la cui promozione esatta a f64
non torna identica dopo il rendering a otto decimali, sono rifiutati.
In lettura N/F vengono confrontati con il testo decimale originale: F non
passa piu' da f32 e una perdita decimale comporta rifiuto, senza promozione
implicita a Decimal. Gli spazi iniziali Character sono conservati, quelli
finali sono padding; il NUL e' rifiutato. La decodifica usa `.cpg` prima
dell'LDID, senza replacement; codepage sconosciute o byte non decodificabili
sono rifiutati. LDID 0 senza `.cpg` ammette solo ASCII; 0x57 usa Latin-1.

Versioni verificate con `git show <tag>:crates/driver-shp/src/lib.rs` e
`git show <tag>:Cargo.lock`: v0.1.0-rc.1, v0.1.0-rc.2, v0.1.0-rc.3,
v0.1.0-rc.4, v1.0.0-rc.1, v1.0.0-rc.2, 1.0.0, 1.0.1, 2.0.0, 3.0.0, 4.0.0, 4.1.0,
4.1.1, 5.0.0, 5.0.1 e 6.0.0. Tutte hanno larghezze 254, 18/0 e 20/8 e
conversione numerica tramite `as_f64`. Il troncamento della dipendenza e'
confermato per 3.0.0–6.0.0 (`dbase 0.8.0`, `writing.rs`, `field/types.rs`);
per 1.x–2.x (`dbase 0.5.0`) resta presunto, non riprodotto. La perdita di
precisione della conversione intera e' gia' visibile nel codice di tutti i tag.

Per controllare file gia' scritti, confrontare con la sorgente originale i
byte dei campi DBF, usando offset e larghezze dei descrittori nell'header:
decodificare il testo nella codepage dichiarata, confrontare gli interi come
testo decimale esatto e i numeri a scala otto con i valori originali. Controllare
in particolare testi lunghi/multibyte, interi oltre 2^53, frazioni oltre otto
decimali e numeri al limite della larghezza (segno e punto compresi).
Un campo pieno o UTF-8 invalido e' un indizio, non una prova completa: senza
la sorgente non si possono ricostruire i suffissi o le cifre perdute. Il solo
round-trip e il report di rischio in lettura non certificano la fedelta' della
scrittura precedente. Rigenerare dalla sorgente con la versione corretta.

### CSV e temporali condivisi: cambiamento incompatibile

Difetto silenzioso: null e stringa vuota erano scritti come lo stesso campo;
in lettura il trim applicato prima del ramo testuale trasformava anche soli
spazi in null. L'inferenza reinterpretava testo numerico e booleano senza
confrontarne la grafia, perdendo per esempio zeri iniziali e spazi.

Verifica statica ripetuta con `git show <tag>:crates/driver-csv/src/lib.rs`
e `git show <tag>:crates/driver-common/src/lib.rs`: presenti in **tutti i tag**
locali: v0.1.0-rc.1–rc.4, v1.0.0-rc.1–rc.2, v1.0.0, v1.0.1, v2.0.0,
v3.0.0, v4.0.0, v4.1.0, v4.1.1, v5.0.0, v5.0.1 e v6.0.0.
L'evidenza è il ramo null del writer (`write_field("")`, o stringa vuota nel
convertitore) e `trimmed.is_empty()`/`trim().is_empty()` del builder condiviso;
La riproduzione dinamica a valore e' stata eseguita su v6.0.0; per gli altri
tag citati l'evidenza resta statica, non una riproduzione dinamica.

**Altri difetti silenziosi preesistenti (v2.0.0-v6.0.0, riprodotti a valore
su v6.0.0):** Date64 non allineati al giorno perdevano i millisecondi;
`testo_orario` rendeva orari fuori intervallo come testo invalido. Per esempio,
Date64=1 diventava `1970-01-01`, Time32s=86400 `24:00:00` e Time64ns=-1
`00:00:-1.999999999`, senza errore anche in IPC -> XLSX, GeoJSON e KML.
La correzione e' nel convertitore comune usato anche da DXF, SHP e CSV,
non piu' nel solo CSV. NaN/inf e Float16 nel writer CSV potevano produrre
valori non fedeli (anche null dal fallback), anziche' un rifiuto esplicito;
i nuovi rifiuti correggono questi difetti, non sono soltanto incompatibilita'.
Anche le coordinate CSV X/Y non finite sono ora rifiutate: `NaN,NaN`
veniva codificato come WKB NaN/NaN, cioe' POINT EMPTY, perdendo il valore
originale. La verifica degli altri ingressi testuali ha riprodotto lo stesso
problema in KML sul sorgente corrente: ora sono controllate anche X/Y/Z
di punti, linee e anelli KML. XLSX, WKT, GeoJSON e DXF hanno gia' controlli
per le coordinate non finite. Le prove distinguono attributi testuali `"NaN"`, tuttora conservati,
da numeri o coordinate non rappresentabili.

**Incompatibile:** testo sempre quotato, null non quotato, terminatori CRLF;
le stringhe vuote e di soli spazi non diventano più null, i campi quotati
forzano Utf8 e l'inferenza numerica esige identità lessicale. Quoting malformato,
vuoti geometrici presenti e separatori virgolette/CR/LF sono rifiutati. I tipi
Arrow privi di codifica dichiarata sono rifiutati già nel piano; gli scalari e
i temporali scrivibili dichiarano la perdita del tipo originale (anche del
fuso per i timestamp resi in UTC). Date64 non allineati al giorno e orari
fuori intervallo sono rifiutati anziché troncati. NaN e inf vengono rifiutati senza pubblicazione. Anche `InferredColumnBuilder::append_csv_cell` richiede che il chiamante
decida la presenza e chiami `append_null`: vuoti e spazi testuali sono valori. Il contratto
e l'inventario completo sono in [PRODUCT.md](PRODUCT.md#csv-presenza-testo-e-tipi).

**File già prodotti:** conservare gli originali, senza aprirli e risalvarli
in un foglio elettronico. Confrontare con la sorgente tipizzata validità e
valore di ogni cella testuale, inclusi campi vuoti/di soli spazi, identificativi
con zeri iniziali e notazione numerica. Nel CSV vecchio un campo vuoto non
consente di stabilire se fosse null oppure stringa vuota: il file da solo non
permette il recupero. Occorre rigenerarlo dalla sorgente o dal backup. Se è
stata effettuata una precedente lettura/conversione, confrontare anche schema,
zeri iniziali, spazi, notazione e booleani; aggiornare il reader non ripristina
informazione già persa. Per scambi che devono mantenere anche lo schema usare
Arrow IPC. Le due fixture canoniche CSV e il loro generatore ora codificano la geometria null
con campo vuoto non quotato, aggiornandone i digest. Il registro dei
quartetti aggiunge il rifiuto anticipato Unsupported/Validate dei tipi
attributo non rappresentabili. Il sito condiviso `validate_temporal_cell`
aggiunge il quartetto `Schema/Validate/Schema/Never`: e' il rifiuto dei
Date64 e degli orari non rappresentabili prima della conversione in testo,
ora registrato anche dopo l'unione delle correzioni CSV e XLSX. L'SDK Python
espone il campo opzionale `LossExample.type_class` dichiarato dal protocollo,
conservando la classe del tipo negli esempi di perdita CSV. Gli indici degli
esempi restano opzionali come in `Posizione`: le nuove normalizzazioni dello
schema GPKG senza campo specifico si leggono anche tramite SDK, senza
inventare un indice e senza rifiutare una risposta valida. Le prove nuove confrontano byte attesi indipendenti dal writer,
validità/valore, loss report e mancata pubblicazione per i rifiuti.

Il builder condiviso invalida le misure `wkt_parse` e `shp_reader`; inoltre
il bump 7.0.0 modifica i lock nel perimetro di tutti e quattro i bersagli.
Prima del congelamento occorre rieseguire `scripts/fuzz-profondita.sh` per
`filegdb_reader`, `shp_reader`, `geojson_reader` e `wkt_parse`, e
`bash scripts/asan-filegdb.sh`, nell'ambiente Linux di qualifica con toolchain fissata.
Non si aggiornano le impronte senza una nuova misura.

### GeoPackage: perdite silenziose di valori e schema

Versioni colpite: **0.1.0-rc.1, 0.1.0-rc.2, 0.1.0-rc.3, 0.1.0-rc.4,
1.0.0-rc.1, 1.0.0-rc.2, 1.0.0, 1.0.1, 2.0.0, 3.0.0, 4.0.0, 4.1.0, 4.1.1,
5.0.0, 5.0.1 e 6.0.0**. Verifica statica con
`git show <tag>:crates/driver-gpkg/src/lib.rs` su ciascuno dei sedici tag:
i rami che scartavano storage incompatibili, `String::from_utf8_lossy`,
l'esclusione della PK, gli attributi sempre nullable, la nuova `fid` e
`geometry_types.first()` sono presenti in tutti. Le prove eseguibili della
correzione stanno in `crates/driver-gpkg/src/tests/storage_schema.rs` e
`crates/driver-gpkg/src/tests/affinity.rs`.

- Lo zero negativo `-0.0` perdeva silenziosamente il segno: in scrittura
  Float16/32/64 verso REAL (anche FLOAT/DOUBLE), per la memorizzazione SQLite
  come intero zero; in lettura REAL verso Int64, anche per BOOLEAN e gli altri
  tipi interi. I sedici tag contengono il binding REAL senza controllo del
  segno e il cast `r as i64` senza segnalazione per `-0.0`. Ora la scrittura
  rifiuta il valore; la lettura conta `gpkg_integer_column_real_truncated`
  anche per il segno dello zero perso. `+0.0` resta accettato senza perdita.
  Il segno gia' perso non e' recuperabile dal risultato: confrontare i bit
  con la sorgente originale.

La conservazione di `gpkg.declared_type` introdotta dalla PR #82 aveva inoltre
un difetto nuovo, non attribuito ai tag: DATE/DATETIME su Utf8 applicavano
affinita' NUMERIC, convertendo testi come `007`, `1e3` o `1.50` in numeri.
Ora ogni valore non nullo viene controllato, anche con metadati minuscoli:
la dichiarazione si conserva solo per testi che SQLite lascia TEXT; un
letterale numerico ben formato causa il rifiuto della scrittura. La scelta
evita di modificare lo schema a meta' di uno stream o di dichiarare conservato
un tipo normalizzato. Non si usa CAST, che accetterebbe anche prefissi numerici.
La matrice nei test copre tutte le tredici dichiarazioni ammesse e tutti i tipi
Arrow scrivibili, con le cinque affinita' SQLite; gli abbinamenti incompatibili
sono rifiutati prima del DDL. Le fixture verificano storage e valori con SQLite
diretto, incluso REAL `-0.0` ottenuto senza conversione tramite `writable_schema`.

- Storage SQLite incompatibili con il tipo dichiarato diventavano NULL senza
  report. Ora ogni valore scartato conta in
  `gpkg_incompatible_storage_discarded`; un NULL originale non conta. Se il
  vincolo NOT NULL rende impossibile il batch, la lettura fallisce.
- Testo SQLite con UTF-8 invalido veniva sostituito senza report. Ora conta in
  `gpkg_invalid_utf8_replaced`; un U+FFFD realmente presente resta distinto
  nel report. Gli esempi riportano posizione e causa, mai dati delle righe.
- La lettura espone ora anche la PK INTEGER come attributo non nullable con
  `gpkg.primary_key=true`; la riscrittura ne conserva i valori (anche 10/100),
  anziche' generarne altri. Conserva NOT NULL e i tipi dichiarati riconosciuti
  tramite `gpkg.declared_type`, validato prima di creare il file. BOOLEAN usa
  Int64 nel batch e DATE/DATETIME Utf8, mantenendo la dichiarazione SQL nel
  metadato. I tipi non riconosciuti e le parti SQL non ricostruite (default,
  vincoli ulteriori, indici, trigger) producono `gpkg_schema_normalized` con
  ragione strutturale; anche il riordino (geometria prima, PK ultima) viene
  dichiarato quando avviene. Le colonne generate conservano i valori, con
  perdita della regola SQL dichiarata. Chi consumava uno schema senza PK deve tenere conto
  della colonna aggiuntiva. La chiave composta o diversa da un singolo
  INTEGER viene rifiutata. Senza metadato di PK nel contratto il writer genera
  una chiave nuova, scegliendo un nome che non collide con gli attributi.
- Un contratto con piu' tipi geometrici scrive `GEOMETRY` in
  `gpkg_geometry_columns`, senza dichiarare il solo primo tipo. La rilettura
  ricava i tipi osservati. Resta il rifiuto delle geometrie multiple per tabella:
  un insieme di tipi nella stessa colonna non e' un insieme di colonne.

Le normalizzazioni gia' dichiarate per Boolean/interi stretti/unsigned/float
stretti in scrittura non sono nuovi difetti e restano segnalate dal modello
comune delle perdite pianificate.

**Controllare i file gia' prodotti.** Conservare una copia e confrontarla con
la sorgente originale: NULL e U+FFFD sostitutivi, chiavi rigenerate, dichiarazioni
e vincoli eliminati non sono ricostruibili dal solo risultato. Per ogni feature
table leggere `PRAGMA table_xinfo(...)`, i valori della PK e `typeof(...)` degli
attributi; confrontare NULL, testo e byte con gli originali. In particolare,
TEXT/BLOB in colonne numeriche, BLOB in TEXT e valori non BLOB in BLOB richiedono
una nuova lettura con il driver corretto e controllo del report. Un NULL nel
vecchio risultato da solo non prova che fosse NULL nella sorgente. Verificare
il testo originale come byte, per distinguere UTF-8 invalido da `EF BF BD`.
Confrontare anche default, CHECK, UNIQUE, chiavi esterne, indici e trigger nel
catalogo SQLite: NOT NULL e PK da soli non descrivono tutti i vincoli.

Per geometrie miste, confrontare **tutte** le geometrie con
`gpkg_geometry_columns.geometry_type_name`: l'intestazione del vecchio file puo'
affermare POINT pur contenendo LineString. Riconvertire dalla sorgente con un
contratto che dichiara tutti i tipi; correggere soltanto il metadato non recupera
attributi o identita' gia' perduti. Controllare ogni layer e ogni report, non
soltanto il successo del comando o il numero di righe.

### XLSX: date native leggibili e perdite silenziose corrette

Versioni colpite: **1.0.0, 1.0.1, 2.0.0, 3.0.0, 4.0.0, 4.1.0,
4.1.1, 5.0.0, 5.0.1 e 6.0.0**. Verifica statica con
`git show <tag>:crates/driver-xls/src/lib.rs` su ogni tag: `write_cell`
convertiva il numero con `as_f64()` senza controllo di esattezza; il mapping
(e, dove presente, lo spool) mandava date seriali ed errori nel ramo null.
Gli stessi percorsi esistono nelle prerelease `v0.1.0-rc.1`,
`v0.1.0-rc.2`, `v0.1.0-rc.3`, `v0.1.0-rc.4`, `v1.0.0-rc.1` e
`v1.0.0-rc.2`. Nella 2.0.0 e 3.0.0 il riferimento a `Data::Error`
compare anche in un test che pretendeva proprio null, non in una difesa.

- Scrittura: gli interi fuori da **[-2^53, 2^53]** sono ora rifiutati
  prima di pubblicare il file. Per esempio 9007199254740993 diventava
  9007199254740992 senza perdita dichiarata. Il limite e' conservativo:
  fuori dall'intervallo esistono interi esatti in binary64, ma anche la resa
  decimale XML del writer puo' cambiare l'intero (per esempio i64::MIN).
  Float32 viene ampliato esattamente a Float64; i float finiti conservano il
  valore binario nella rilettura. Il formatter `ryu` usa notazione esponenziale
  per i float estremi, evitando numeri XML di centinaia di cifre.
  Decimali e float non finiti restano rifiutati. U+FFFE e U+FFFF sono
  rifiutati in scrittura: gli escape del writer non tornano identici con
  il decoder calamine corrente.
- Lettura: `Data::DateTime` ora produce Date32 per formato solo data e
  seriale intero, Timestamp(Microsecond, None) per data/ora, oppure
  Duration(Microsecond) per stili durata come `[h]:mm:ss`. L'epoca 1900/1904
  viene letta direttamente da `workbook/workbookPr` nel namespace principale
  SpreadsheetML (booleani XML `1`, `true`, `0`, `false`); l'estensione
  `x15:workbookPr` di Excel non puo' sovrascriverla. Anche fogli, relazioni,
  formati, stili e celle sono selezionati per namespace e percorso XML.
  I formati incorporati CJK 27-36 e 50-58 sono riconosciuti, inclusi quelli
  orari 32/33/55/56. Un seriale in [0,1) con formato solo ora produce
  Time64(Microsecond), in entrambi i calendari. Nel 1900 i giorni 1-59 sono
  mappati esattamente; [0,1) con formato data e il falso 29/02/1900
  (intervallo [60,61)) sono rifiutati. Nel 1904 il giorno zero e' 1904-01-01. Seriali negativi,
  non finiti o oltre il 9999-12-31 sono rifiutati.
- Precisione Excel al millisecondo: si calcola il candidato ms dal seriale
  binary64 con decodifica IEEE esatta e arrotondamento razionale al piu'
  vicino (round-half-even). Dal rapporto canonico ms/86_400_000 si ricava
  il binary64 con arrotondamento razionale; si accettano i suoi bit oppure
  quelli ottenuti stampandolo a 15, 16 o 17 cifre significative e rileggendolo
  come f64. Il confronto e' esatto, senza tolleranze: riconosce anche le
  serializzazioni di openpyxl/pandas. La stessa regola vale per ore e durate; l'unita' Arrow resta il
  microsecondo (ms * 1000). Date32 richiede ms multiplo di 86_400_000.
  Sono ammessi 13:17:42, 23:59:59.999, 00:00:00.001 e la durata 37:15:30.5;
  seriali con informazione sub-ms non canonica sono rifiutati esplicitamente.
  La precedente regola sperimentale dei microsecondi razionalmente interi
  rifiutava anche orari Excel ordinari ed e' stata sostituita.
- Colonne con valori temporali e altri tipi (o tipi temporali diversi) sono
  rifiutate prima della consegna; i vuoti nella colonna restano null.
  `Data::Error` e numeri non finiti restano rifiutati. Date/durate ISO restano
  testo. Nessuna data nativa valida nel dominio esatto diventa null.

La scrittura temporale conserva la resa ISO gia' documentata e usa la
validazione unica di `driver-common`, condivisa con CSV: Date64 non allineati
al giorno e orari fuori intervallo sono rifiutati. La lettura dei seriali nativi
applica invece le regole Excel 1900/1904 descritte sopra. `planned_write_loss` non concede un
arrotondamento implicito dei valori. I test rileggono fixture OOXML 1900 e
1904 e confrontano esattamente i valori Arrow, inclusi vuoti ed errori.

**File gia' prodotti:** conservare e confrontare la sorgente originale.
Aprire XLSX come ZIP e confrontare gli interi sorgente con i valori `<v>`
delle celle numeriche in `xl/worksheets/sheet*.xml`, usando un parser decimale
esatto, non f64. Cercare anzitutto colonne Int64/UInt64 con valori fuori da
[-2^53, 2^53]; un valore arrotondato puo' cadere proprio sul confine, quindi
il solo output non basta a diagnosticare o recuperare il valore originario.
Per le letture precedenti, controllare nei workbook sorgente celle `t="e"`,
stili data/durata in `xl/styles.xml` e `date1904` in `xl/workbook.xml`;
confrontare le celle corrispondenti negli output derivati, che possono avere
null al posto dei valori. Rigenerare dalla sorgente dopo aver gestito
esplicitamente le celle rifiutate. Il dato originale non si ricostruisce dal
null o dal numero arrotondato.

### Contratti: pin `7f82263`

Il pin di plenora-contracts passa da `3aec9f7` a `7f82263`, che porta la regola
di profilo IO-ARROW-001: uno schema Arrow senza `plenora.contract.version` e
senza chiavi `plenora.` è una sorgente esterna e si accetta. Era già il
comportamento di IO; la deviazione «ARROW-001 / REJ-001 (versione del contratto
assente)» si ritira dal manifesto di adozione. Il comportamento non cambia.

Il pin porta anche la superficie Python dichiarata nel catalogo io-tools
(`conditional`, IO-PY-001). Questa adozione **non la rivendica**:
`plenora-python-sdk-v1` resta `not_applicable` nel manifesto, che lì vuol dire
«non rivendicato», ed è una garanzia indebolita dichiarata in
[`contracts/adozione-4.0.0.json`](../contracts/adozione-4.0.0.json).
