# Installazione e migrazione — mettere in funzione 2.0.0

Questo documento si rivolge a chi **riceve** il prodotto: installa un artefatto,
lo collega a qualcosa che già funziona, e — se veniva da una 1.x — deve sapere
che cosa nel frattempo è cambiato sul filo.

Non descrive come gli artefatti si costruiscono né come si qualificano: quello
sta in [ENGINEERING.md § Distribuzione](ENGINEERING.md#distribuzione), ed è una
domanda che si fa chi lavora al prodotto, non chi lo usa. Qui la domanda è
un'altra: che cosa scaricare, come accertarsi che sia arrivato intero, e che
cosa riscrivere nel proprio codice.

## Due cose separate, e non per caso

Il prodotto è un **eseguibile nativo**, `plenora-io`, che parla JSON su stdout.
L'SDK Python è un **wrapper** che lo esegue e ne tipizza le risposte. Sono due
artefatti distinti, e il pacchetto Python non contiene l'eseguibile.

La separazione ha un prezzo e una ragione. Il prezzo è che installare l'SDK non
basta: va anche installato il binario, e i due vanno fatti trovare. La ragione è
che incorporare l'eseguibile vorrebbe dire una wheel per ogni piattaforma, il
triplo degli artefatti da qualificare, e un prodotto che si aggiorna soltanto
cambiando il pacchetto Python — legando la cadenza di rilascio del motore a
quella di un wrapper che ha ragioni di cambiare del tutto diverse.

Chi non usa Python installa soltanto il primo. È il caso normale.

## Che cosa esiste

### Gli artefatti nativi

Un archivio per **piattaforma × profilo**, chiamato secondo la forma

```
plenora-io-<versione>-<piattaforma>-<profilo>.<estensione>
```

per esempio `plenora-io-2.0.0-linux-x86_64-filegdb.tar.gz`. Le piattaforme
distribuite sono `linux-x86_64` (`tar.gz`) e `windows-x86_64` (`zip`); i profili
sono `base` e `filegdb`.

Il profilo sta nel nome perché due archivi della stessa versione e piattaforma
differiscono per una **capability**, non per un dettaglio di build: chi scarica
deve poterlo leggere dal nome invece di scoprirlo eseguendo `catalog`.

Le soglie di sistema sono dichiarate e misurate, non dedotte dall'ambiente di
sviluppo:

| Piattaforma | Requisito dichiarato |
|---|---|
| `linux-x86_64` | glibc ≥ 2.35 — Ubuntu 22.04 e successive |
| `windows-x86_64` | Windows 10 22H2 e successive, Windows 11, Windows Server 2022 e successive |

### I due artefatti Python

| Formato | Nome | Che cos'è |
|---|---|---|
| wheel | `plenora_io-<versione>-py3-none-any.whl` | il pacchetto installabile senza ricostruzione |
| sdist | `plenora_io-<versione>.tar.gz` | i sorgenti da cui il pacchetto si ricostruisce, con i test dentro |

`py3-none-any` vuol dire che di piattaforme non ne serve nessuna: non c'è un
solo file compilato. La sdist porta i test perché chi la riceve possa verificare
ciò che installa — e non tutti girano fuori dal repository: quali saltano, e
perché, è un elenco chiuso in
[`assurance/registries/sonde-saltate-nella-sdist.json`](../assurance/registries/sonde-saltate-nella-sdist.json).

`Requires-Python` è `>=3.11,<3.14`, e copre **soltanto** le versioni che la CI
prova. Dichiarare `>=3.11` senza limite superiore prometterebbe ogni Python
futuro, e una promessa del genere la si mantiene provandola.

## Come si ottiene, e sotto quali termini

Gli artefatti — i quattro nativi e i due Python — si consegnano per un **canale
riservato a clienti autorizzati**. Non stanno su PyPI né su altri indici
pubblici, e il pacchetto Python porta nei metadati il classificatore
`Private :: Do Not Upload`, che i servizi d'indice leggono per rifiutare il
caricamento.

Nessuna licenza first-party è dichiarata: dentro l'archivio non ci sono termini
che concedano qualcosa, e la loro assenza non è un permesso. Ciò che è concesso
lo stabilisce il contratto con cui l'artefatto è stato consegnato. In
particolare, chi riceve un artefatto non è autorizzato a ridistribuirlo né a
pubblicarlo.

Il contratto di release lo registra nell'invariante
`distribuzione.licenza-first-party` come **capacità differita** — perimetro
dichiarato, non blocco chiuso — e
[docs/RELEASE.md](RELEASE.md) ne riporta la riga con ciò che la 2.0.0 non
promette. Una distribuzione pubblica richiederebbe una decisione separata del
titolare.

Le licenze dei **componenti di terzi** sono un'altra cosa, e restano dovute:
ogni artefatto nativo porta in `LICENSES/` il testo di ognuna, e un gate conta
che ci siano tutte. Il pacchetto Python non ne porta perché non spedisce byte
di terzi.

Wheel e sdist sono Python puro e contengono i sorgenti leggibili; la sdist
anche i test. La consegna è riservata, non segreta: non è stata promessa alcuna
riservatezza del codice.

## Scegliere il profilo

`base` porta il binario, il manifesto, l'SBOM e le licenze. Non porta GDAL: il
workspace senza `gdal-backend` è Rust puro, e il driver FileGDB degrada a uno
stub che **rifiuta** la capability invece di fingerla. Su Windows porta comunque
il runtime C ridistribuibile, che ogni binario MSVC pretende.

`filegdb` aggiunge il runtime GDAL fissato — 3.9.3, dalla stessa catena su
entrambe le piattaforme — e i suoi dati.

Il modo per sapere che cosa si è installato non è leggere il nome del file:

```
plenora-io catalog
```

dichiara, per ogni driver, `available` e `required_feature`. Sul profilo `base`
il driver `filegdb` risulta non disponibile, e lo dice prima che qualcuno provi
a usarlo.

## Installare l'albero nativo

L'archivio si estrae dove si vuole: nessun percorso assoluto è incorporato nel
binario o nelle librerie. Il layout è

```
bin/plenora-io          (bin/plenora-io.exe su Windows)
lib/                    solo profilo filegdb — su Windows le DLL stanno in bin/
share/gdal/             solo profilo filegdb
share/proj/             solo profilo filegdb
MANIFEST.json
```

Su Windows le DLL stanno in `bin/` e non in `lib/` perché è lì che il caricatore
guarda per prima; spostarle romperebbe l'avvio.

### Verificare ciò che è arrivato

`MANIFEST.json` porta un digest per **ogni** file spedito, insieme a versione,
piattaforma, profilo, revisione e lock del runtime. La verifica si fa
sull'albero estratto, che è dove i file possono essere cambiati o spariti:

```
python3 - <<'FINE'
import hashlib, json, pathlib
radice = pathlib.Path(".")
manifesto = json.loads((radice / "MANIFEST.json").read_text(encoding="utf-8"))
for voce in manifesto["file"]:
    # I percorsi del manifesto costruito su Windows usano il separatore di lì.
    percorso = radice / voce["percorso"].replace("\\", "/")
    digest = hashlib.sha256(percorso.read_bytes()).hexdigest()
    if digest != voce["sha256"]:
        print("DIVERSO:", voce["percorso"])
print(len(manifesto["file"]), "file verificati")
FINE
```

Oggi nessuno strumento di verifica viene **spedito dentro** l'archivio: la
verifica sopra è a carico di chi riceve, e questo documento la scrive per esteso
proprio perché non c'è un comando da invocare al suo posto. Nel repository lo
stesso controllo lo fa `scripts/check-digest-manifesto.py --albero <estratto>`,
e la CI lo esegue su ogni artefatto costruito.

### La prima prova

```
bin/plenora-io --version
bin/plenora-io catalog
```

La prima risponde `{"status":"ok","version":"2.0.0"}` — due campi, né uno di
più. La seconda enumera i dieci driver con ciò che ciascuno sa fare.

Nulla va aggiunto al `PATH` perché il binario funzioni: se lo si aggiunge è per
comodità, e l'SDK Python lo troverebbe anche di lì (vedi sotto).

## Installare l'SDK Python

```
pip install plenora_io-2.0.0-py3-none-any.whl
```

oppure, se si preferisce ricostruire dai sorgenti:

```
pip install plenora_io-2.0.0.tar.gz
```

L'installazione non scarica nient'altro: il pacchetto non ha dipendenze a
runtime, e in particolare **non scarica il binario**. Un pacchetto Python che
tirasse giù un eseguibile sarebbe una via d'esecuzione di codice che nessun
lockfile controlla, e chi lo installa non l'ha chiesta.

### Come l'SDK trova il binario

Quattro posti, in quest'ordine, e nessun quinto:

1. il percorso passato esplicitamente: `Client(binary="/opt/plenora/bin/plenora-io")`;
2. la variabile d'ambiente `PLENORA_IO_BIN`;
3. `bin/plenora-io` nell'albero distribuito, se il pacchetto è stato installato
   accanto a uno;
4. il `PATH`.

Se non lo trova solleva `BinaryNotFound` **dicendo dove ha cercato**. Non ne
inventa uno e non ne procura uno.

L'ordine non è arbitrario. L'esplicito batte l'ambiente perché chi scrive una
riga di codice sta dicendo qualcosa di più preciso di chi ha esportato una
variabile tre shell fa; l'ambiente batte l'albero perché è il modo di provare un
binario diverso senza reinstallare; l'albero batte il `PATH` perché un artefatto
installato porta con sé le proprie librerie, e prendere dal `PATH` il binario di
un'altra installazione le mescolerebbe.

### La prima prova

```python
from plenora_io import Client

cliente = Client()
print(cliente.version())          # Version(status='ok', version='2.0.0')
print(len(cliente.catalog().drivers))
```

Chi ha bisogno di FileGDB può pretenderlo invece di sperarlo:

```python
cliente = Client()
cliente.require_profile("filegdb")    # solleva ProfileError se l'albero è `base`
```

Va chiamato **prima** del lavoro. Scoprire il profilo sbagliato dal fallimento
di una conversione a metà costa un file d'uscita parziale e un errore che parla
di un driver invece che di un pacchetto.

Il controllo legge `MANIFEST.json` accanto al binario. Un binario costruito da
`cargo` non ha un manifesto, ed è perfettamente usabile: l'assenza non è un
errore. Un manifesto **presente e illeggibile** lo è, perché vuol dire che
l'artefatto è rotto.

## Migrazione 3.x → 4.0.0 — il protocollo pubblico

La 4.0.0 adotta i contratti pubblici di `plenora-contracts`. Il cambiamento si
vede tutto sul confine del processo, ed è **incompatibile**: chi automatizza la
CLI deve toccare il proprio codice.

### Le cinque differenze che richiedono di toccare il codice

**1. I dati dell'operazione stanno in `result`.** Prima uscivano al primo
livello, accanto a `status` e `contract`.

```diff
- jq '.drivers'            # 3.x
+ jq '.result.drivers'     # 4.0.0
```

Vale per ogni comando: `catalog`, `inspect`, `layers`, `read`, `convert`.

**2. La busta d'errore esce su `stdout`.** Usciva su `stderr`, e `stdout`
restava vuoto. Ora è il contrario: **un** documento su `stdout`, `stderr`
vuoto, sempre — riuscita o fallita che sia l'invocazione.

```diff
- if ! out=$(plenora-io inspect x.shp 2>err.json); then jq . err.json; fi
+ out=$(plenora-io inspect x.shp); echo "$out" | jq '.status'
```

Chi leggeva `stderr` per il fallimento oggi non trova nulla. È la differenza che
rompe più silenziosamente, perché un consumatore che ignora `stdout` sui
fallimenti vede un errore senza diagnostica invece di un errore.

**3. I codici d'uscita proiettano la categoria.** Erano agganciati al codice
interno `IoErrorCode`; ora vengono dalla tabella di CLI 2.0 §8.

| categoria | 3.x | 4.0.0 |
|---|---:|---:|
| `invalid_plan`, `invalid_configuration` | 2 | **2** |
| `schema`, `data_mapping`, `crs`, `unsupported` | 2, 4, 5, 6 | **3** |
| `resource_limit` | 7 | **4** |
| `io`, `not_found`, `conflict`, `protocol`, `authentication`, `authorization`, `timeout`, `transient` | 1, 3, 8 | **5** |
| `execution` | 1 | **6** |
| `internal` | 2 | **70** |
| `cancelled` | 130 | **130** |

I valori `7` e `8` non esistono più. Chi decide in base al numero deve rifarsi
la tabella; chi decide in base a `.error.category` — che è ciò che il contratto
chiede — non deve cambiare niente.

**4. Ogni busta porta l'identità.** Quattro campi nuovi al primo livello:
`component` (`plenora-io-tools`), `component_version`, `command`, e
`protocol_version` che vale `2` anche sugli errori — prima valeva `1`.

**5. `--version` risponde nella busta comune.**

```diff
- plenora-io --version
- {"status":"ok","version":"3.0.0"}
+ plenora-io --version --format json
+ {"status":"ok","protocol_version":2,"component":"plenora-io-tools",…,
+  "result":{"component_version":"4.0.0","cli_protocol_version":2}}
```

`--version` da solo continua a rispondere, nella stessa busta.

### Il protocollo v1 non è più raggiungibile

`--legacy-protocol-v1-unsafe` è stato rimosso. Non è deprecato: non esiste, e
un flag sconosciuto fallisce chiuso come ogni altro.

Il profilo pubblico vieta a un artefatto di servire due versioni del protocollo
JSON — un consumatore che ne trovava due nello stesso binario non poteva sapere
quale gli sarebbe arrivata senza leggere il comando. La 3.0.0 resta scaricabile
e continua a emettere il v1 quando glielo si chiede: chi non può migrare subito
resta su quella, che è pubblicata e verificabile, invece di ottenere il v1 da un
binario che dichiara di parlare v2.

`release/cli-protocol-v1.json` resta nel repository come record di quei byte, e
un invariante del contratto di release ne presidia l'immutabilità: descrive
artefatti pubblicati, e quelli non cambiano.

### Che cosa non cambia

I nomi dei comandi, i loro argomenti, i formati supportati, la semantica delle
conversioni, i metadati Arrow e le categorie di perdita. La migrazione riguarda
**come** si legge la risposta, non che cosa il prodotto fa.

### Lo SDK Python

Aggiornato insieme al CLI: `plenora_io` legge la busta nuova e restituisce il
risultato. Chi usa lo SDK non vede nessuna delle cinque differenze, tranne che
`Version` ora espone `component_version` e `cli_protocol_version` — `.version`
resta leggibile e significa la stessa cosa.

## Migrazione 3.x → 4.0.0

La 4.0.0 è la prima release che reclama il profilo `io-tools` dei contratti
comuni, ed è una release **di rottura**. Le rotture sono tre, e nessuna è
silenziosa: ognuna produce un rifiuto tipizzato dove prima c'era un
comportamento.

### 1. `convert` vuole i due formati, e non li deduce più

Prima i formati venivano dalle estensioni dei due percorsi. Ora si nominano:

```
# prima
plenora-io convert dati.geojson uscita.gpkg

# dalla 4.0.0
plenora-io convert dati.geojson uscita.gpkg --from geojson --to gpkg
```

Omettere i due argomenti è un errore d'uso, non un ritorno alla deduzione: un
default che sopravvive alla deprecazione è la deprecazione che non avviene.

La ragione non è di stile. Il catalogo comune descrive `io.convert` come
operazione «between **explicit** formats», e il profilo vieta di scegliere il
comportamento specifico di un formato analizzando l'estensione quando
l'operazione la richiede esplicita. Dedurre era comodo e diceva tre cose false:
che `.json` significhi GeoJSON, che un percorso senza estensione non abbia
formato, e che il nome di un file sia un contratto.

Gli identificatori sono quelli che `catalog` rende — non estensioni, non nomi
lunghi. La corrispondenza con le estensioni di prima:

| estensione | identificatore |
|---|---|
| `.parquet` | `geoparquet` |
| `.geojson`, `.json` | `geojson` |
| `.csv` | `csv` |
| `.gpkg` | `gpkg` |
| `.shp`, `.shp.d` | `shp` |
| `.kml` | `kml` |
| `.xlsx` | `xls` |
| `.dxf` | `dxf` |
| `.gdb` | `filegdb` |
| `.arrow` | `ipc` |

`inspect`, `layers` e `read` **non** cambiano: quelle operazioni riconoscono la
sorgente invece di riceverla dichiarata — `io.inspect` rende «its **declared**
format» — e lì il suffisso resta l'unico segnale che c'è. Riconoscere e
dichiarare sono due cose, e il catalogo le distingue operazione per operazione.

### 2. Il nome del contratto nelle buste

Ogni busta annuncia ora l'identificatore che il contratto fissato le assegna.
Prima portavano tutte il suffisso `v2`, che è quello del **protocollo** — una
cosa diversa dalla versione dell'operazione. La coincidenza reggeva finché
nessuno confrontava.

| Comando | 3.x | 4.0.0 | fonte |
|---|---|---|---|
| `catalog` | `plenora-io-catalog-v2` | `plenora-io-catalog-v1` | catalogo comune |
| `inspect` | `plenora-io-inspect-v2` | `plenora-io-inspect-v1` | catalogo comune |
| `layers` | `plenora-io-layers-v2` | `plenora-io-layers-v1` | catalogo comune |
| `read` | `plenora-io-read-v2` | `plenora-io-read-result-v1` | catalogo comune |
| `write` | — | `plenora-io-write-result-v1` | catalogo comune |
| `convert` | `plenora-io-convert-v2` | `plenora-io-convert-v1` | catalogo comune |
| errori | `plenora-io-error-v1` | `plenora-error-v1` | ERRORS-1.0 |
| `capabilities` | `plenora-capabilities-v2` | invariato | CAPABILITY-DISCOVERY-2.0 |
| `--version` | `plenora-io-version-v2` | invariato | nessuna: non è un'operazione |

Due righe smentiscono la somiglianza dei nomi, ed è la ragione per cui la
tabella esiste invece di una regola: `read` rende `-read-**result**-v1` perché
il catalogo distingue l'ingresso `-read-input-v1` dall'uscita, e l'errore è
`plenora-error-v1` **senza** `io` perché i fallimenti pubblici mappano sul
contratto d'errore **comune**, che ha un nome suo. Resta nostro
`plenora-io-error-details-v1`, che è il contenuto facoltativo di `details` e non
la busta.

`--version` è l'unica busta senza un contratto fissato a cui allinearsi, ed è
l'unico caso in cui `v2` significa ancora quello che dice.

`protocol_version` resta `2`: il protocollo non è cambiato, sono cambiati i nomi
che le buste dichiarano.

### 3. La destinazione di una scrittura non deve più portare l'estensione

Prima ogni driver rifiutava una destinazione il cui nome non portasse
l'estensione attesa. In sette casi su dieci quel controllo era una convenzione:
dopo di esso l'estensione non veniva usata per niente. Toglierlo rende il
formato esplicito sufficiente a scegliere la destinazione — chi pubblica su un
percorso di staging o su un nome generato non viene più rifiutato per il nome
invece che per i dati.

Restano due vincoli, e sono dichiarati nel catalogo con la loro ragione:

| formato | suffissi | perché |
|---|---|---|
| `gpkg` | `.gpkg` | la specifica GeoPackage lo impone (OGC 12-128r, requisito 2) |
| `shp` | `.shp`, `.shp.d` | i file companion derivano il nome dal principale, e il suffisso sceglie la forma di pubblicazione |

Il catalogo dichiara anche, per ogni formato, i suffissi con cui viene
**riconosciuto** in lettura (`recognised_suffixes`). Scrivere su un nome diverso
è ammesso e non rende il file illeggibile: rende necessario dichiarare il
formato quando lo si rilegge, invece di lasciarlo dedurre.

### Un nuovo comando

`write INGRESSO.arrow DESTINAZIONE --to FORMATO` pubblica un dataset Arrow — il
file che `read --output` consegna — nel formato nominato. Non sostituisce
`convert`: quella collega due sorgenti esterne, questa pubblica il dataset che
il chiamante ha già in mano, ed è ciò che permette di mettere i propri passi fra
la lettura e la scrittura.

## Migrazione 1.x → 2.0.0

Il perimetro di compatibilità del prodotto è dichiarato e stretto:
`cli_json_only`. Ciò che si migra sono le buste JSON su stdout. L'API Rust è
interna e instabile in entrambe le versioni — non era un contratto pubblico
nella 1.x e non lo è ora — e chi ne dipendeva dipendeva da qualcosa che nessuna
regola proteggeva.

### La differenza è il protocollo predefinito

La 1.x emetteva il protocollo v1. La 2.0.0 emette il **v2**, e il v1 resta
raggiungibile con un'opzione esplicita.

| Comando | 1.x | 2.0.0 |
|---|---|---|
| `inspect` | `plenora-io-inspect-v1` | `plenora-io-inspect-v2` |
| `layers` | `plenora-io-layers-v1` | `plenora-io-layers-v2` |
| `read` | `plenora-io-read-v1` | `plenora-io-read-v2` |
| `convert` | `plenora-io-convert-v1` | `plenora-io-convert-v2` |
| `catalog` | `plenora-io-catalog-v1` | `plenora-io-catalog-v2` |
| errori | `plenora-io-error-v1` | `plenora-io-error-v1`, invariato |

Un consumatore che verifica `contract` — ed è ciò che un consumatore dovrebbe
fare — se ne accorge alla prima chiamata. Uno che verifica `protocol_version`
legge `2` dove leggeva `1`.

### Le tre differenze che richiedono di toccare il codice

Non sono dedotte dai manifesti: sono **misurate** eseguendo il binario nei due
protocolli sulla stessa fixture. Chi vuole rifare la misura:

```
python3 scripts/delta-protocollo.py --binario target/debug/plenora-io --lavoro /tmp/dp
```

**1. `counts` passa da oggetto a lista.** È la sola rottura vera, e riguarda
`convert`:

```jsonc
// v1 — le chiavi sono le categorie
"write_loss": { "counts": { "crs_id_not_preserved_absent": 1 } }

// v2 — le chiavi sono fisse, le categorie sono valori
"write_loss": { "counts": [ { "categoria": "crs_id_not_preserved_absent", "conteggio": 1 } ] }
```

Chi scriveva `loss["counts"]["qualche_categoria"]` deve scorrere la lista. Il
cambiamento non è cosmetico: nel v1 quelle chiavi non avevano tetto e potevano
arrivare a 4096, con identificatori controllati da chi fornisce il file. Un
oggetto le cui chiavi le decide l'input è una superficie che il consumatore non
può dimensionare.

**2. Sette campi nuovi dicono che cosa manca.** Ogni sezione diagnostica del v2
porta `troncato`, `omesse_esatte` e `omesse` con le quattro cause separate —
`categorie_omesse`, `ragioni_omesse`, `esempi_omessi`, `omesse_per_byte`:

```jsonc
"fidelity": {
  "troncato": false,
  "omesse_esatte": true,
  "omesse": { "categorie_omesse": 0, "ragioni_omesse": 0, "esempi_omessi": 0, "omesse_per_byte": 0 }
}
```

Sono campi aggiunti: chi non li legge continua a funzionare. Ma sono anche il
motivo per cui il v2 può permettersi dei tetti — nel v1 la diagnostica non ne
aveva, e la sua dimensione la decideva chi forniva il file. Nel v2 qualcosa può
restare fuori, e quando succede **lo dice**, con quanto e per quale delle quattro
ragioni. Un consumatore che aggrega conteggi dovrebbe guardarli: `troncato: true`
vuol dire che i numeri che sta sommando sono parziali, e nel v1 quel caso non
era distinguibile da un file senza perdite.

**3. I nomi presi dal file spariscono dai testi.** Nel v1 `reasons[].detail` e
gli esempi portavano nomi di layer e di attributo, più la forma `Debug` di tipi
di una dipendenza. Nel v2 non più: al loro posto ci sono `layer_index`,
`field_index` e `type_class`, e il testo è curato.

```jsonc
// v2
"reasons": [ { "code": "…", "detail": "…", "layer_index": 0, "field_index": 3 } ]
```

Chi mostrava `detail` all'utente finale mostra ora una frase che non nomina il
suo file; per risalire al nome deve incrociare l'indice con lo schema che ha già.
È il prezzo della regola, ed è deliberato: un identificatore che viene dal file
non è pubblicabile per il solo fatto di essere finito in un messaggio d'errore —
e nemmeno un suo hash, che resta un identificatore controllato da chi il file lo
fornisce. Il secondo effetto è che una dipendenza che cambia la propria `Debug`
non può più cambiare la busta senza che nessuno tocchi il protocollo.

### `catalog` cambia soltanto l'intestazione

Il catalogo è la busta che si migra più facilmente: i due protocolli producono
lo **stesso** elenco di driver, con le stesse capability e lo stesso
`determinism`. A cambiare sono `protocol_version` e `contract`, e nient'altro
— settantaquattro percorsi identici su settantaquattro.

Chi legge il catalogo per sapere che cosa il prodotto sa fare può quindi
passare al v2 senza toccare una riga, e verificare `contract` per accorgersi di
essere passato.

### Gli argomenti che questi comandi non conoscono

`catalog` accetta il solo flag legacy, e `--version` non accetta niente: ogni
altro argomento — un'opzione sconosciuta, un percorso, il flag ripetuto,
un'opzione buona per un altro comando — è un `CLI_USAGE`.

Vale la pena saperlo perché per un periodo non è stato vero: i due punti
d'ingresso senza sorgente scartavano gli argomenti senza guardarli, e
`catalog --limit 5` usciva con zero e una busta buona. Uno script scritto
contro quel comportamento — o un errore di battitura sopravvissuto perché
nessuno l'aveva mai visto fallire — ora fallisce, ed è il punto.

### Restare sul v1

```
plenora-io read dati.geojson --legacy-protocol-v1-unsafe
```

L'opzione va **dopo** il sottocomando. Il nome dice che cosa si sta scegliendo:
`--protocol 1` sarebbe stato più corto e avrebbe fatto sembrare le due versioni
due opzioni pari, e non lo sono.

Chi la usa riceve su **stderr** un avviso che nomina i due difetti che si sta
riprendendo: identificatori controllati dal file, e fino a 4096 chiavi in
`counts`. Su stdout non ci va niente, perché il v1 è congelato byte per byte e
aggiungere un avviso al documento sarebbe cambiarlo.

Che cosa si accetta scegliendolo: il v1 è `frozen_for_1_0` e non riceverà
correzioni — le sue regole dicono che cambiare il tipo o il significato di un
campo obbligatorio richiede una versione nuova, ed è esattamente ciò che il v2
ha fatto. I gate di release usano il v2 e soltanto il v2.

### Che cosa non cambia

`--version` risponde con due campi, `status` e `version`, e non è una busta v2:
è la busta di **bootstrap**, quella che si legge prima di sapere quale protocollo
il binario parli. Non porta `contract` né `protocol_version`, e non li porterà.

La busta d'errore resta `plenora-io-error-v1` con `protocol_version: 1`
qualunque sia il protocollo scelto per stdout. Non è un'incoerenza: sono due
superfici, va su stderr, e né la sua struttura né il suo significato cambiano
col v2. Crearne una v2 identica avrebbe moltiplicato i nomi senza aggiungere
un'affermazione.

Restano invariati anche il quartetto che classifica ogni errore — `category`,
`code`, `phase`, `retry` — e la regola che su di esso, e mai sul testo del
messaggio, un consumatore prende le proprie decisioni.

### L'SDK Python non si migra

Non esisteva nella 1.x. La sua prima versione è la 2.0.0, parla il v2 e basta, e
un binario che rispondesse v1 lo tratta come un errore di protocollo invece di
adattarsi. Non c'è codice 1.x da riscrivere: c'è, semmai, codice che chiamava
`subprocess` a mano e che l'SDK può sostituire.

## Dove guardare dopo

| | |
|---|---|
| [PRODUCT.md](PRODUCT.md) | che cosa ciascun driver promette, opzione per opzione |
| [ENGINEERING.md](ENGINEERING.md) | come è fatto e come viene verificato |
| [sdk/python/README.md](../sdk/python/README.md) | l'SDK: metodi, errori, deadline, cancellazione |
| [`release/cli-protocol-v2.json`](../release/cli-protocol-v2.json) | il contratto delle buste, campo per campo |
