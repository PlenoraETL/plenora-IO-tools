# Piano 4.1.0 — residui, dipendenze e qualifica sostenibile

## Che cos'è questo documento, e che cosa non è

È il piano di lavoro della 4.1.0: i residui che la 4.0.0 ha lasciato aperti, il
costo delle dipendenze e della manutenzione, e gli strumenti di qualifica da
rendere sostenibili. Nasce dalle decisioni concordate il 15 settembre 2026,
subito dopo la pubblicazione della 4.0.0, e le integra nella documentazione
versionata: prima viveva in `.s9-checkpoint/pianificazione/`, ignorata da Git
per non modificare l'albero che la campagna stava misurando.

Non aggiunge condizioni alla 4.0.0, che è pubblicata. Non promette l'assenza di
fork o di duplicazioni: promette che ogni voce si chiuda con **una soluzione
provata oppure una motivazione precisa**. Un elenco di rimozioni sarebbe una
promessa; un elenco di esiti è un piano.

Il predecessore è [docs/PIANO-4.0.0.md](PIANO-4.0.0.md), che resta in questo
docset — vedi «Un documento che non può scadere» più sotto.

## Riconciliazione con lo stato effettivo

Il piano è stato scritto mentre il rilascio era in corso. Alcune sue righe
descrivevano cose ancora da verificare, e adesso hanno una risposta. Questa
sezione le riconcilia una per una, perché un piano che continua a chiedere ciò
che è già stato fatto fa ripetere il lavoro.

| Riga del piano concordato | Stato effettivo al 15 settembre 2026 |
|---|---|
| «La conferma della CI finale resta da verificare» | **verificata.** CI verde su `8cf013f`, l'ultimo commit del verbale; verde anche su `4d4298c` e `47f8667` |
| «Il censimento non è presente nella radice» | **recuperato e intatto**, vedi la sezione seguente |
| «Il volume target conserva circa 144 GiB e il disco virtuale non è stato compattato» | **superata.** Il resoconto di pulizia registra la compattazione: `docker_data.vhdx` da 516.045.144.064 a 19.636.682.752 byte |
| «Un solo container: `plenora-err`» | **rimosso** dalla pulizia. Zero container; sei immagini distinte, con `plenora-io-dev:latest` che punta alla stessa immagine di `plenora-io-dev-198` |
| P0 «Confermare pubblicazione e verifica della 4.0.0» | **chiusa.** Tag `v4.0.0` su `6beb410b9984f527f3e89bba420764ac9e24e436`, ventitré asset riscaricati dalla release identici a quelli della bozza, stato registrato, CI confermata |

La Fase 0 è quindi chiusa nelle sue quattro voci. Resta vero il suo avvertimento
operativo: **non ripetere cancellazioni sulla base di un inventario precedente**,
perché l'ambiente è stato rimisurato dopo il rilascio e le cifre della prima
tranche sono storiche.

## Il censimento delle dipendenze: dov'è, e perché non era dove il piano diceva

Il piano lo cita «nella radice IO-tools». Non c'era, e la ragione è nota: il
15 settembre alle 07:47 UTC è comparso in radice **durante** la corsa di livello
2 su `6beb410`, e l'ha fatta cadere con tre passi rossi — `sonde_docset`,
`check_docset` e soprattutto `albero_invariato`, che dice che l'albero è cambiato
sotto la misura. È stato **spostato, non cancellato**, in
`C:\Users\marco\Desktop\censimento-librerie.md`, e la corsa è stata rifatta da
capo perché l'invarianza dell'albero non si recupera a posteriori.

Il documento è integro e **non gli manca nulla di dichiarato**: 121.133 byte,
misurato contro `6beb410b9984f527f3e89bba420764ac9e24e436`, con
`cargo tree --offline --locked` su profili base e filegdb per Linux e Windows
x86_64 — «8/8 interrogazioni riuscite», e alla voce «Verifiche online non
riuscite» dice «Nessuna».

Da questo blocco è nel docset come
[docs/CENSIMENTO-LIBRERIE-2026-09-15.md](CENSIMENTO-LIBRERIE-2026-09-15.md),
con la data nel nome perché è **una fotografia, non uno stato corrente**: il
grafo cambia al primo aggiornamento, e un documento canonico senza data
inviterebbe a leggerlo come se non fosse invecchiato. Si ritira quando gli
interventi L1–L4 sono chiusi e il grafo è stato rimisurato; il suo successore
porterà la propria data.

## Vincoli dei gate incontrati in questo blocco

Il mandato di questo primo blocco era «solo documentazione». Due vincoli lo
attraversano, e sono segnalati invece di aggirati.

**L'allowlist del docset è codice, non dati.** `CANONICI` è una lista Python in
`scripts/check_docset.py`, e l'allowlist è **esatta**: nessun Markdown tracciato
può stare fuori da essa. Ammettere questo piano e il censimento richiede quindi
una modifica a `scripts/`, che non è documentazione. È l'unica modifica di
questo blocco fuori da `docs/` e dal `README.md`, era prevista dal piano
concordato — «l'ammissione necessaria nel docset» — e consiste nell'aggiungere
due voci a una lista. Non tocca alcuna logica di verifica.

**Il gate del contratto non è un controllo documentale.**
`scripts/check_release_contract.py`, anche fuori dalla modalità release, esegue
l'harness dei test con `cargo`. Dopo la pulizia post rilascio il volume target è
vuoto, quindi una sua corsa ricompila il workspace da zero: su una modifica di
sola documentazione costa ore e non misura nulla che la documentazione abbia
cambiato. In questo blocco è stato avviato per abitudine e **fermato**, senza
lasciare modifiche. Appartiene alla chiusura di un blocco di codice, non a una
modifica di `docs/`. I controlli proporzionati qui sono `check_docset.py`, le
sue trentanove sonde e `check_comments.py`.

**Le campagne lunghe non si lanciano durante lo sviluppo.** Le misure complete —
livello 2, soak prolungati, qualifica degli artefatti — si eseguono **sulla
candidate finale**, prima del rilascio. Durante lo sviluppo valgono prove mirate
alle modifiche e controlli proporzionati alla chiusura dei blocchi. Una campagna
lunga si ripete solo se una modifica successiva ne invalida le prove o se emerge
un problema concreto, e in quel caso va detto **quale misura è invalidata e
perché**. Se un gate pretendesse diversamente, il vincolo va segnalato: non
aggirato, e non risolto avviando altre ore di test.

## Due rilievi aperti dalla pubblicazione

Non erano nel piano concordato perché sono emersi dopo. Nessuno dei due mette in
discussione la 4.0.0 pubblicata; entrambi vanno trattati nella 4.1.0.

### Q1 — il workflow del tag contraddice il modello a due revisioni

La push di `v4.0.0` ha innescato «Release checkout qualification» sulla revisione
congelata, e il job `rust` è rosso con:

```
`candidate_release.tag_creato` vale «False» ... git trova il tag «v4.0.0»
```

Non è un difetto del prodotto né del rilascio. Il tag punta alla revisione
**congelata**, e in quella revisione lo stato non può sapere di un tag creato
dopo: `tag_creato` viene scritto in `2484019`, e `commit_di_assurance` nel
successivo, perché un commit non può nominare se stesso. Il workflow qualifica
il checkout del tag e vi applica un invariante che a quel checkout è falso per
costruzione.

Criterio di chiusura: il workflow distingue ciò che si verifica **sull'albero
congelato** da ciò che si verifica **sul commit di registrazione**, oppure
dichiara di non applicare gli invarianti di registrazione a un checkout di tag.
Non si chiude allentando l'invariante, che sul ramo di sviluppo serve.

### Q2 — panico upstream in parquet raggiunto da GeoParquet malformato

Lo smoke fuzz della corsa «Release checkout qualification» 34988124226, sulla
revisione pubblicata, ha trovato un finding nuovo su `geoparquet_reader`:

```
thread panicked at parquet-59.3.0/src/encodings/decoding/byte_stream_split_decoder.rs:61:38
index out of bounds: the len is 2 but the index is 2
```

L'input è conservato: 3.966 byte, magic `PAR1`,
`sha256=18cb2a7e0e394484…`, in
`IO-tools-evidenze-4.0.0\finding-geoparquet-34988124226\`.

**La causa è individuata.** In `parquet-59.3.0`,
`ByteStreamSplitDecoder::get` ricava due quantità da due fonti diverse — lo
`stride` dai byte **effettivi** della pagina, il conteggio del ciclo da
`total_num_values`, cioè dal **dichiarato** — e `join_streams_const` indicizza
`sub_src[i + j * stride]` senza confrontare l'indice con la lunghezza. Il campo
incoerente è `num_values` nell'header della data page.

Non è una descrizione dedotta dal panico: è dimostrata costruendola. Un
riproduttore scrive un GeoParquet **valido** con una colonna `DOUBLE`
`BYTE_STREAM_SPLIT`, verifica che si legga, poi altera **un solo byte** — lo
zigzag di 63 occupa lo stesso spazio di quello di 8, quindi nessun offset si
muove — e prova tutti i siti in cui quel varint compare. Solo l'offset 13, nel
header di pagina, produce l'indicizzazione fuori limite; l'offset 17, un altro
campo dello stesso header, non produce nulla. Il primo tentativo sostituiva il
varint con uno più lungo, spostava il resto e otteneva `EOF: Invalid page
header`: struttura rotta, non incoerenza semantica, e nessuna dimostrazione.

**Il prodotto non crolla, e non serve una 4.0.1.** Sul binario estratto
dall'archivio pubblicato: `read` esce 3 con `FORMAT_ERROR`, categoria
`data_mapping`, fase `read`, `retry: never`. Zero segnali, zero timeout. La
barriera `catch_unwind` sta in `plenora-io-core/src/driver.rs`, cioè nella
**libreria**: un consumatore Rust che chiama il driver direttamente riceve lo
stesso errore tipizzato, verificato separatamente dalla CLI.

Tre comandi su quattro **non** provano la barriera, e va detto: `inspect` e
`layers` leggono footer e schema senza decodificare pagine; `convert` si ferma
in fase `validate`, e il controfattuale lo dimostra — sullo stesso percorso con
un file valido esce `ok`, quindi sa leggere, e sul seme non ci arriva. L'unico
comando che esercita il decoder è `read`.

**Il finding resta valido.** L'`abort()` che `libfuzzer-sys` chiama prima
dell'unwinding impedisce al target di osservare il recupero; non rende
inesistente il difetto a monte.

#### Q2a — regressione locale: **chiusa**

`crates/driver-geoparquet/tests/byte_stream_split.rs` pretende i quattro assi
dell'errore e che il panico non sia propagato;
`byte_stream_split_sintetico.rs` è il riproduttore che identifica il campo. La
fixture sta in `crates/driver-geoparquet/tests/fixtures/`, **fuori** da
`fuzz/seeds/`, `fuzz/corpus/` e `fuzz/artifacts/`, che `scripts/fuzz-replay.sh`
riesegue tutte e tre.

Che le ultime due siano ignorate da Git non le esclude dal replay: dice solo che
una copia appena clonata parte senza. In locale persistono e il replay le legge —
`fuzz/artifacts/geoparquet_reader/` contiene oggi tre input del 2026-08-17, che
il replay del livello 2 su `6beb410` ha rieseguito verdi.

#### Q2b — segnalazione upstream: **da inviare**

Il riproduttore esiste e gira, il contributo no. Resta da preparare: un caso
minimo autonomo, senza dipendenze dal nostro workspace, e la segnalazione al
progetto `arrow-rs` con la versione, il punto e il campo. Q2 non è chiuso finché
questo non è inviato.

#### Q2c — gestire i finding noti del fuzz: **problema aperto, senza soluzione scelta**

La quarantena del progetto è **per bersaglio**: `fuzz/quarantine.txt` elenca
nomi di target, e un target in quarantena viene compilato sotto AddressSanitizer
ma non eseguito. Per questo finding sarebbe troppo ampia — smetterebbe di
esplorare `geoparquet_reader` — e non è stata usata. Il file stesso la riserva a
finding dove «uno smoke che fallisce sempre non è un gate, è rumore», e questo
non fallisce sempre.

Il problema da risolvere, scritto prima del meccanismo:

> gestire finding noti **senza** disabilitare il bersaglio, **senza** nascondere
> difetti nuovi, e **senza** dichiarare completa una campagna che si è
> interrotta.

Una lista di digest non lo risolve, e vale la pena dire perché invece di
scoprirlo dopo: il fuzzer può rigenerare lo stesso difetto da un input diverso,
o produrne una variante. La prova sta nei tre artefatti locali di
`geoparquet_reader` — due misurano 3.966 byte, la stessa dimensione del seme di
questo finding, con digest tutti diversi dal suo: stessa famiglia, quattro
digest. Il meccanismo va progettato a parte, e questa voce ne è il requisito.

## Compatibilità e criteri generali

La 4.1.0 conserva API pubbliche, protocollo CLI, semantica degli errori, formati
accettati, opzioni offerte, metadati, atomicità e proprietà di fedeltà promesse
dalla 4.0.0. Una versione esterna più recente non è automaticamente compatibile.

Se chiudere un residuo richiede una rottura, si prepara una soluzione
compatibile oppure quel punto passa a una major successiva. Non si impone la
versione più alta a una dipendenza transitiva contro i vincoli dei suoi
chiamanti. Non si eliminano controlli di sicurezza né si riscrivono parser per
ottenere un conteggio di librerie inferiore.

Ogni voce si chiude con uno dei tre esiti: **risolta con prova**, **ancora
necessaria con motivazione**, **dipendenza upstream o incompatibilità
identificata con seguito preciso**.

## Fase 1 — residui delle prove e della procedura

| ID | Intervento | Criterio di chiusura |
|---|---|---|
| R1 | Regressione dedicata a `MAX_BLOCCHI` nella prevalidazione Arrow | Caso sotto, al e oltre il limite; rifiuto della guardia prevista; costo misurato e collocazione proporzionata. Le dimensioni della fixture sono un costo da gestire, non un'impossibilità |
| R2 | Togliere alle campagne la dipendenza da `sleep` a scadenza e dalla cattura di `docker exec` | Processo di campagna e suo exit code osservabili; log persistenti fuori dal container; interruzione distinta dal successo; durata verificata senza ricavarla dal ritmo medio |
| R3 | Imporre il legame fra candidate, revisione qualificata ed evidenza corrente | Fixture rifiutano misura assente o di un'altra revisione; distinguono registrazione entro allowlist, evidenza storica e riuso ammesso; nessun collegamento affidato soltanto al verbale umano |
| R4 | Rendere esplicito il riuso delle evidenze | Regola per tipo di modifica e perimetro: prodotto, test, documenti, toolchain, feature, lock. Prova della validità e della provenienza del riuso; si conserva la revisione realmente misurata |
| R5 | Evitare corse concorrenti duplicate e push dopo controlli già falliti | Una misura per SHA e perimetro; esiti dei comandi controllati prima dei passi dipendenti; monitor legato allo SHA e alla corsa esatta; nessuna diagnosi basata sulla corsa precedente |
| R6 | Versionare il misuratore del soak con le prove dei casi già osservati | **Chiusa**: `scripts/soak_misurato.py`, regressioni in `scripts/test_soak_misurato.py` collegate a CI e checkpoint; CPU diagnostica; originali e giudizio corretto in `scripts/fixtures/soak/` |

R4 non significa ereditare automaticamente una qualifica completa dopo una
modifica ai test. Si definisce e si prova prima quali evidenze restano valide,
quali cambiano e come il gate le riconosce.

I tre difetti che R6 deve pinnare sono stati osservati durante la qualifica
della 4.0.0, e vanno scritti perché riguardano la fiducia nella misura:

1. confrontare i due soli orologi interni non rileva il congelamento della
   **VM**, perché si fermano insieme — dopo sedici ore di ibernazione dell'host
   il loro scarto valeva `0 s` su una corsa inservibile;
2. un'uscita diversa da zero non è un finding se la campagna non è mai partita:
   è successo con `rustup` assente dal PATH;
3. la soglia sull'uso della CPU boccia una campagna valida quando il fuzzer non
   è legato alla CPU, e non era un requisito del soak.

Il confronto del corpus fra mount Windows e volume Linux usa **lo stesso
bersaglio e gli stessi input**: l'I/O resta una causa possibile e non
quantificata del tempo non contabilizzato, non una spiegazione dimostrata.

R6 conserva i byte del misuratore e del referto originali, con digest verificati
dalle sonde, e un estratto separato del giudizio corretto già registrato in
`assurance/evidence/checkpoint-6beb410.json`. Il referto originale continua a
dire `durata_dimostrata: false`. La nuova analisi degli stessi numeri accetta
la durata senza imporre saturazione CPU; non costituisce una nuova campagna.

Il misuratore pretende un riepilogo conclusivo con durata sufficiente oltre
all'intervallo degli orologi: la preparazione non completa un soak troppo breve.
Riconosce i segnali di finding anche senza `Done`; un exit nonzero senza tali
segnali resta da classificare. La concordanza degli orologi significa **nessuna
discontinuità rilevata**, e uno scarto non identifica da solo la causa.
R2 resta aperto per il ciclo di vita della campagna e R3 per il legame con la
candidate. Uso e limiti sono descritti in [ENGINEERING.md](ENGINEERING.md#misurare-il-soak).

## Fase 2 — aggiornamenti delle librerie esterne

Le ultime stabili si rimisurano prima di intervenire: i numeri qui sotto sono la
fotografia del censimento, non obiettivi mobili della candidate.

| ID | Intervento | Punto di partenza | Criterio di chiusura |
|---|---|---|---|
| L1 | Aggiornare `rust_xlsxwriter` | 0.99.0 → 0.99.1 disponibile | API, XLSX prodotti e riletti, metadati e diagnostiche preservati; grafo risultante misurato |
| L2 | Aggiornare `jsonschema` | 0.55.1 → 0.56.0; minor 0.x potenzialmente incompatibile | Schemi validi e invalidi conservano i verdetti; resolver remoti restano disabilitati; closure e feature misurate |
| L3 | Esaminare aggiornamenti transitivi e advisory | Versioni e catene nel censimento | Patch compatibili distinte dai salti richiesti dagli upstream; nessun aggiornamento massivo incontrollato; audit e deroghe riallineati |
| L4 | Rimuovere il solo pin diretto inutilizzato `arrow-select` | Nessun crate lo eredita; Parquet lo introduce transitivamente | Dichiarazione e registri coerenti, grafo invariato. **Non** si presenta come libreria eliminata dal binario |

L1 e L2 sono identificatori di **questa tabella**, non livelli di checkpoint.
Interventi con rischi diversi restano separati; le registrazioni dello stesso
blocco si riuniscono prima della verifica.

## Fase 3 — versioni duplicate

La fotografia dei profili base con dipendenze ordinarie e di build mostra sette
nomi duplicati su Linux e otto su Windows. `syn` e `thiserror-impl` riguardano
le macro: una duplicazione non equivale a due copie di codice runtime. Le
versioni presenti soltanto nei lock per altri target o per i test non sono
duplicazioni del binario distribuito.

| ID | Libreria e versioni osservate | Causa e lavoro richiesto |
|---|---|---|
| D1 | `num-traits` 0.1.43 / 0.2.19 | Modernizzare `enum_primitive` nel fork DXF; verificare parser e generatori su valori enum validi e invalidi |
| D2 | `quick-xml` 0.41.0 / 0.42.0 | La vecchia è richiesta sia da `kml` sia da `calamine`: coordinare entrambi gli aggiornamenti preservando le codifiche supportate |
| D3 | `num-bigint` 0.4.8 / 0.5.1 | 0.4 è trattenuta da DXF/`num` e da `jsonschema`/`fraction`, 0.5 da Arrow: trattare entrambe le catene |
| D4 | `getrandom` 0.3.4 / 0.4.3 | Allineare i chiamanti, `ahash` e `tempfile`/`uuid` inclusi, rispettando target e feature; nessuna forzatura dal lock |
| D5 | `thiserror` e `thiserror-impl` 1.0.69 / 2.0.19 | WKT mantiene la linea precedente; aggiornare il chiamante con la semantica degli errori preservata |
| D6 | `syn` 2.0.119 / 3.0.3 | Dipendenze delle macro: seguire l'aggiornamento dei generatori. Misurare il costo di compilazione, non presumere costo runtime |
| D7 | `windows-sys` 0.52.0 / 0.61.2 | `atomicwrites` mantiene 0.52: aggiornamento upstream o sostituzione equivalente del wrapper Windows |

Per ogni eliminazione: confronto del grafo prima e dopo sugli stessi target,
profili e feature, e identificazione dei rami che mantengono ancora una
versione. Il solo conteggio di `Cargo.lock` non è un risultato.

## Fase 4 — riduzione di feature e catene

| ID | Candidata | Intervento e limite |
|---|---|---|
| F1 | ZIP → Zopfli | Verificare la disattivazione del compressore non usato dal writer XLSX, conservando Deflate. Va coordinata la richiesta di feature del workspace, di `calamine` e di `rust_xlsxwriter`: cambiarne una sola non basta |
| F2 | DXF → `image` e miniature BMP | Rendere opzionale la decodifica delle miniature solo con una politica compatibile per DXF validi e malformati. Oggi il parser le decodifica davvero: non si cancella `image` dichiarandola inutilizzata |
| F3 | Publisher Windows → `atomicwrites` | Valutare aggiornamento o sostituzione senza perdere no-clobber, atomicità e supporto directory. Nessun `unsafe` aggiunto al workspace per ridurre un conteggio |
| F4 | DXF → `num` | I due usi diretti di tipi di errore si possono semplificare con `std`, ma `jsonschema`/`fraction` continua a trattenere `num`. Dichiarare l'eventuale beneficio locale senza inventare una riduzione globale |
| F5 | `jsonschema` e validatori non usati dagli schemi incorporati | Valutare modularizzazione upstream. Non si scrive un validatore sostitutivo ad hoc e non si toglie il controllo dei metadati GeoParquet |
| F6 | Backend Deflate | Studiare l'unificazione della selezione `zlib-rs`/`miniz_oxide` con prove sui codec e misure reali. Due crate nel grafo non provano che entrambi i motori siano nel binario |
| F7 | Chiusura nativa GDAL | Valutare se una distribuzione GDAL più limitata sia sostenibile per FileGDB. Non si eliminano singole librerie dal prefisso: una variante richiede nuovi lock, provenance, SBOM e qualifica Linux e Windows |

Vincoli già corretti, da conservare: default KMZ di `kml` disabilitato; codec
immagine DXF limitati al BMP; feature Excel opzionali non abilitate; resolver
HTTP, file, TLS e IDNA di `jsonschema` disabilitati. I codec Parquet offerti dal
catalogo e le funzioni di orologio e UUID realmente usate non sono tagli
gratuiti.

F5, F6 e F7 sono analisi con decisione di convenienza, non autorizzazioni
preventive a introdurre fork o sostituire architetture. Se il costo supera il
beneficio, la valutazione si chiude con motivazione e la dipendenza resta.

## Fase 5 — fork e contratti condivisi

| ID | Intervento | Criterio di chiusura |
|---|---|---|
| U1 | Riconciliare tutti i delta di DXF, GDAL e Shapefile | Elenco completo confrontato **semanticamente** con le release ufficiali, non ricerca di nomi né stato `mergeable` di una PR |
| U2 | Aggiornare le basi e rimuovere le patch assorbite | La release ufficiale contiene il comportamento necessario e le regressioni passano senza la patch. Il fork si rimuove intero solo quando nessun delta necessario resta |
| U3 | Monitorare versioni e risposte upstream periodicamente | Segnalazione separata dalla CI della candidate: una release di terzi non rende rossa una revisione già qualificata |
| C1 | Trattare la deviazione `side_effect` di `io.read` | Soluzione concordata col contratto comune, o comportamento compatibile. La deviazione non è più presente solo quando il requisito è **effettivamente** soddisfatto |
| C2 | Trattare la regola di pubblicazione non espressa dal catalogo | Descrittore e comportamento concordano sul rifiuto dei tipi `unresolved` e sulle garanzie realmente offerte |
| C3 | Conservare sul filo la distinzione «scansione completa, nessuna geometria» | Coordinamento con la decisione 0006 e la PR #6 e col vocabolario successore; prova del secondo giro senza perdita della distinzione; compatibilità esplicita coi lettori precedenti |
| C4 | Riesaminare il ruolo provider e le deviazioni collegate | Nessun campo inventato per chiudere una riga; decisione comune se il significato riguarda più componenti |

Una chiave nuova non è automaticamente compatibile: i vocabolari possono essere
chiusi e le versioni ignote rifiutate. Se C3 non è realizzabile preservando la
compatibilità promessa dalla 4.0.0, la modifica incompatibile resta per una
major successiva.

Il pin corrente dei contratti condivisi è
`453c8d1ff2eb260840e6cedc033a2b76b58a0b9e`. La PR #6 e la decisione 0006 si
riesaminano **sullo stato effettivo**, senza assumere che siano integrate.

L'interoperabilità effettiva con `plenora-data-tools` e `plenora-database-tools`
resta differita per decisione dell'utente. Il supporto dei content type previsti
non equivale a una qualifica cross-component, e non diventa bloccante della
4.1.0 senza una nuova decisione esplicita.

## Un documento che non può scadere

`docs/PIANO-4.0.0.md` porta scritto che «quando la 4.0.0 è qualificata, questo
documento esce dall'allowlist invece di restare a invecchiare». La 4.0.0 è
qualificata e pubblicata, e la scadenza **non si può eseguire**: le due
deviazioni registrate in `contracts/adozione-4.0.0.json` indicano come proprio
luogo di tracciamento le sue righe B10 e B12c. Cancellarlo romperebbe i
riferimenti di due deviazioni ancora aperte.

Il documento resta quindi nel docset, e la sua scadenza si sposta a quando le
deviazioni C1 e C2 di questo piano sono chiuse — cioè quando il tracciamento non
ha più nulla da indicare. È un esempio del motivo per cui una scadenza scritta
in un documento va riconciliata invece che eseguita.

## Dipendenze: uso effettivo, chi le introduce, condizioni per rimuoverle

Per ogni dipendenza del grafo, la distinzione fra uso effettivo, catena di
introduzione e condizioni di rimozione sta nella tabella estesa del censimento,
[docs/CENSIMENTO-LIBRERIE-2026-09-15.md](CENSIMENTO-LIBRERIE-2026-09-15.md),
sezione «Tutte le dipendenze Rust: funzione, catena e possibilità di rimozione».
Non è ricopiata qui: due elenchi della stessa cosa divergono, e quello con la
data è la misura.

Il criterio che il censimento applica va letto prima delle sue tabelle:
**necessaria significa usata per conservare le capacità attuali**, non
insostituibile in assoluto. Da lì vengono quattro conclusioni che vincolano
questo piano, e ognuna smonta una riduzione che sembrava ovvia:

- `arrow-select` è il solo pin diretto non ereditato da alcun crate, ma resta
  nel grafo transitivo via Parquet: rimuovere la dichiarazione **non** riduce il
  binario;
- `libc` è dichiarata direttamente solo dal benchmark e resta transitiva nel
  prodotto: non è eliminabile dal binario;
- `jsonschema` non è solo test, `zip` non è solo duplicazione di `calamine`,
  `tempfile` non è solo fixture;
- `gdal` e la sua chiusura nativa sono condizionali al profilo FileGDB:
  rimuoverli rinuncia a quella capacità.

Gli archi normali del grafo includono le macro procedurali e non misurano i
simboli presenti nel binario. I lock nativi descrivono il **prefisso di
costruzione**: le librerie effettivamente spedite si leggono nei singoli SBOM,
non si deducono dall'intero lock Conda. Strumenti di CI, programmi di sistema e
dataset non sono librerie applicative e non si contano come tali.

## Sequenza di implementazione e verifiche

1. Integrare questo piano e il censimento nel docset — **fatto in questo
   blocco** — fissando baseline e criteri, senza spostare tag né riscrivere
   evidenze storiche.
2. Chiudere Q1, Q2b, Q2c e R1–R6 prima di una nuova candidate, così che gli
   strumenti per qualificare e registrare esistano già. Q2a è chiusa.
3. Aggiornamenti L1 e L2, pin inutilizzato e riduzioni minori, in blocchi
   distinti e verificabili.
4. Coordinare duplicazioni, fork e proposte upstream; affrontare le catene più
   grandi solo dopo una decisione sul beneficio misurabile.
5. Riconciliare contratti, deviazioni e compatibilità; chiudere ogni voce con
   esito e prova, oppure con rinvio motivato.
6. Preparare una sola revisione definitiva, eseguire la qualifica prevista,
   produrre gli artefatti sullo stesso SHA e registrare il congelamento entro
   l'allowlist.

Il regime delle misure:

- letture e pianificazione: nessun livello 1 né 2;
- sola documentazione: controlli mirati del docset;
- codice, feature e gate: prove mirate durante lo sviluppo, livello 1 alla
  chiusura di un blocco coerente;
- rimisure solo per i perimetri invalidati, nominando quale misura cade e
  perché; nessuna campagna concorrente duplicata;
- livello 2 completo e soak prolungati **sulla candidate definitiva**, con base
  differenziale esplicita, log duraturi e strumenti già pronti. Distribuzione e
  misure devono identificare la revisione effettivamente provata;
- nessuna aggiunta facoltativa durante il congelamento. La pubblicazione della
  4.1.0 richiede un'autorizzazione propria.

## Riferimenti

- Repository: <https://github.com/PlenoraETL/plenora-IO-tools>.
- Contratti condivisi: <https://github.com/PlenoraETL/plenora-contracts>.
- Riferimento ingegneristico: <https://github.com/PlenoraETL/plenora-database-tools>.
  La copia locale era arretrata al momento del confronto: non si aggiorna né si
  modifica implicitamente.
- Evidenze durature della 4.0.0: `C:\Users\marco\Desktop\IO-tools-evidenze-4.0.0\`,
  con la qualifica di `6beb410`, i confronti di bozza e release e l'input del
  finding Q2. Non sono materiale da eliminare nella pulizia.
- Resoconto della pulizia post rilascio:
  `.s9-checkpoint/pianificazione/PULIZIA-2026-09-15.md`, fuori dal controllo di
  versione come il resto di quella directory.

## Risultato atteso

La 4.1.0 conserva il comportamento pubblico della 4.0.0 e porta un elenco dei
residui trattati, dipendenze aggiornate o motivate, duplicazioni ridotte dove
possibile, e una procedura di qualifica meno costosa senza prove più deboli.

Il resoconto finale confronta versioni, feature, grafo per piattaforma e
profilo, dimensione e composizione degli artefatti quando misurate, tempo di
costruzione e di qualifica quando misurato, e delta residui dei fork. Non
presenta pacchetti non più nel lock come byte risparmiati, né un riuso di
evidenza come una nuova esecuzione.
